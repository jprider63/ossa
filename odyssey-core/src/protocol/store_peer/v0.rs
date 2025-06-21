use std::{fmt::Debug, future::Future, marker::PhantomData, ops::Range};

use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc::{UnboundedReceiver, UnboundedSender}, oneshot};
use tracing::debug;

use crate::{auth::DeviceId, network::protocol::{receive, send, MiniProtocol}, store::{self, HandlePeerRequest, UntypedStoreCommand}, util::Stream};


#[derive(Debug, Serialize, Deserialize)]
pub(crate) enum MsgStoreSync<StoreId> {
    Request(MsgStoreSyncRequest),
    MetadataHeaderResponse(MsgStoreSyncMetadataResponse<StoreId>),
    MerkleResponse(MsgStoreSyncMerkleResponse<StoreId>),
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) enum MsgStoreSyncRequest {
    MetadataHeader,
    MerklePieces {
        ranges: Vec<Range<u64>>,
    },
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct MsgStoreSyncMetadataResponse<StoreId> (StoreSyncResponse<store::v0::MetadataHeader<StoreId>>);

#[derive(Debug, Serialize, Deserialize)]
pub(crate) enum StoreSyncResponse<A> {
    Response(A),
    // Don't currently know, wait until we share it with you.
    Wait,
    // They may or may not have the metadata, but they rejected the request.
    Reject,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct MsgStoreSyncMerkleResponse<Hash> (StoreSyncResponse<Vec<Option<Hash>>>);

impl<StoreId> Into<MsgStoreSync<StoreId>> for MsgStoreSyncRequest {
    fn into(self) -> MsgStoreSync<StoreId> {
        MsgStoreSync::Request(self)
    }
}

impl<StoreId> TryInto<MsgStoreSyncRequest> for MsgStoreSync<StoreId> {
    type Error = ();
    fn try_into(self) -> Result<MsgStoreSyncRequest, ()> {
        match self {
            MsgStoreSync::Request(r) => Ok(r),
            MsgStoreSync::MetadataHeaderResponse(_) => Err(()),
            MsgStoreSync::MerkleResponse(_) => Err(()),
        }
    }
}

impl<StoreId> Into<MsgStoreSync<StoreId>> for MsgStoreSyncMetadataResponse<StoreId> {
    fn into(self) -> MsgStoreSync<StoreId> {
        MsgStoreSync::MetadataHeaderResponse(self)
    }
}

impl<StoreId> TryInto<MsgStoreSyncMetadataResponse<StoreId>> for MsgStoreSync<StoreId> {
    type Error = ();
    fn try_into(self) -> Result<MsgStoreSyncMetadataResponse<StoreId>, ()> {
        match self {
            MsgStoreSync::Request(_) => Err(()),
            MsgStoreSync::MetadataHeaderResponse(r) => Ok(r),
            MsgStoreSync::MerkleResponse(_) => Err(()),
        }
    }
}

impl<StoreId> Into<MsgStoreSync<StoreId>> for MsgStoreSyncMerkleResponse<StoreId> {
    fn into(self) -> MsgStoreSync<StoreId> {
        MsgStoreSync::MerkleResponse(self)
    }
}

impl<StoreId> TryInto<MsgStoreSyncMerkleResponse<StoreId>> for MsgStoreSync<StoreId> {
    type Error = ();
    fn try_into(self) -> Result<MsgStoreSyncMerkleResponse<StoreId>, ()> {
        match self {
            MsgStoreSync::Request(_) => Err(()),
            MsgStoreSync::MetadataHeaderResponse(_) => Err(()),
            MsgStoreSync::MerkleResponse(r) => Ok(r),
        }
    }
}

pub(crate) struct StoreSync<StoreId> {
    peer: DeviceId,
    // Receive commands from store if we have initiatives or send commands to store if we're the responder.
    recv_chan: Option<UnboundedReceiver<StoreSyncCommand>>,
    // Send commands to store if we're the responder and send results back to store if we're the initiator.
    send_chan: UnboundedSender<UntypedStoreCommand<StoreId>>, // JP: Make this a stream?
}

impl<StoreId> StoreSync<StoreId> {
    pub(crate) fn new_server(peer: DeviceId, recv_chan: UnboundedReceiver<StoreSyncCommand>, send_chan: UnboundedSender<UntypedStoreCommand<StoreId>>) -> Self {
        let recv_chan = Some(recv_chan);
        Self { peer, recv_chan, send_chan }
    }

    pub(crate) fn new_client(peer: DeviceId, send_chan: UnboundedSender<UntypedStoreCommand<StoreId>>) -> Self {
        Self { peer, recv_chan: None, send_chan }
    }


    async fn run_client_helper<S: Stream<MsgStoreSync<StoreId>>, R, SR>(&self, stream: &mut S, request: MsgStoreSyncRequest, build_command: fn(HandlePeerRequest<R>) -> UntypedStoreCommand<StoreId>, build_response: fn(StoreSyncResponse<R>) -> SR)
    where
        StoreId: Debug,
        SR: Into<MsgStoreSync<StoreId>> + Debug,
        R: Debug,
    {
        // Send request to store.
        let (response_chan, recv_chan) = oneshot::channel();
        let cmd = build_command(HandlePeerRequest {
            peer: self.peer,
            request,
            response_chan,
        });
        self.send_chan.send(cmd).expect("TODO");

        // Wait for response.
        match recv_chan.await.expect("TODO") {
            Ok(response) => {
                // Send response to peer.
                let response = build_response(StoreSyncResponse::Response(response));
                debug!("Sending response to peer ({}): {response:?}", self.peer);
                send(stream, response).await.expect("TODO");
            }
            Err(chan) => {
                // If waiting, tell peer.
                debug!("Telling peer to wait for response ({})", self.peer);
                send(stream, MsgStoreSyncMetadataResponse(StoreSyncResponse::Wait)).await.expect("TODO");

                // Wait for response and send to peer.
                let response = if let Some(res) = chan.await.expect("TODO") {
                    debug!("Sending response to peer ({}): {res:?}", self.peer);
                    StoreSyncResponse::Response(res)
                } else {
                    debug!("Not sending response to peer ({})", self.peer);
                    StoreSyncResponse::Reject
                };

                send(stream, build_response(response)).await.expect("TODO");
            }
        }
    }

}

pub(crate) enum StoreSyncCommand {
    MetadataHeaderRequest,
    MerkleRequest(Vec<Range<u64>>), // Upper bound of 8000 requested hashes.
    InitialPieceRequest(Vec<Range<u64>>), // JP: Vec of ranges?
}

impl<StoreId: Debug + Serialize + for<'a> Deserialize<'a> + Send> MiniProtocol for StoreSync<StoreId> {
    type Message = MsgStoreSync<StoreId>;

    // Has initiative
    fn run_server<S: Stream<Self::Message>>(self, mut stream: S) -> impl Future<Output = ()> + Send {
        async move {
            // Wait for command from store.
            let mut recv_chan = self.recv_chan.expect("Unreachable. Server must be given a receive channel.");
            while let Some(cmd) = recv_chan.recv().await {
                match cmd {
                    StoreSyncCommand::MetadataHeaderRequest => {
                        send(&mut stream, MsgStoreSyncRequest::MetadataHeader).await.expect("TODO");

                        let MsgStoreSyncMetadataResponse(result) = receive(&mut stream).await.expect("TODO");
                        match result {
                            StoreSyncResponse::Response(metadata) => {
                                // Send store the metadata and tell store we're ready.
                                let msg = UntypedStoreCommand::ReceivedMetadata { peer: self.peer, metadata };
                                self.send_chan.send(msg).expect("TODO");
                            }
                            StoreSyncResponse::Wait => {
                                // Wait for response (Must be Cancel or MetadataHeader).
                                todo!();
                            }
                            StoreSyncResponse::Reject => {
                                // Send store they canceled and tell store we're ready.
                                todo!();
                            }
                        }
                    }
                    StoreSyncCommand::MerkleRequest(ranges) => {
                        send(&mut stream, MsgStoreSyncRequest::MerklePieces{ ranges: ranges.clone() }).await.expect("TODO");

                        let MsgStoreSyncMerkleResponse(result) = receive(&mut stream).await.expect("TODO");
                        match result {
                            StoreSyncResponse::Response(pieces) => {
                                // Send store the piece hashes and tell store we're ready.
                                let msg = UntypedStoreCommand::ReceivedMerklePieces {peer: self.peer, ranges, pieces};
                                self.send_chan.send(msg).expect("TODO");
                            }
                            StoreSyncResponse::Wait => {
                                // Wait for response (Must be Cancel or MerklePieces).
                                todo!();
                            }
                            StoreSyncResponse::Reject => {
                                // Send store they canceled and tell store we're ready.
                                todo!();
                            }
                        }
                    }
                    StoreSyncCommand::InitialPieceRequest(ranges) => todo!(),
                }
            }

            debug!("StoreSyncCommand receiver channel closed");



            // ??
            // Send our store's status.
            // Wait for their status.
            //
            // Status:
            // | DownloadingHeader
            // | DownloadingInitialState {BlocksIds}
            // | Syncing {ecg_known: frontier, ecg_have: frontier}

        }
    }

    fn run_client<S: Stream<Self::Message>>(self, mut stream: S) -> impl Future<Output = ()> + Send {
        async move {
            // TODO: Check when done.
            loop {
                // Receive request.
                let request = receive(&mut stream).await.expect("TODO");
                match request {
                    MsgStoreSyncRequest::MetadataHeader => {
                        const fn build_command<H>(req: HandlePeerRequest<store::v0::MetadataHeader<H>>) -> UntypedStoreCommand<H> {
                            UntypedStoreCommand::HandleMetadataPeerRequest(req)
                        }
                        const fn build_response<H>(h: StoreSyncResponse<store::v0::MetadataHeader<H>>) -> MsgStoreSyncMetadataResponse<H> {
                            MsgStoreSyncMetadataResponse(h)
                        }

                        self.run_client_helper::<_, store::v0::MetadataHeader<StoreId>, MsgStoreSyncMetadataResponse<StoreId>>(&mut stream, request, build_command, build_response).await;
                    }
                    MsgStoreSyncRequest::MerklePieces { .. } => {
                        todo!("run_client_helper");
                    }
                }
            }
        }
    }
}
