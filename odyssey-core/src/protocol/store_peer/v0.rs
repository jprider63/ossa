use std::{fmt::Debug, future::Future, marker::PhantomData, ops::Range};

use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc::{UnboundedReceiver, UnboundedSender}, oneshot};
use tracing::{debug, warn};

use crate::{auth::DeviceId, network::protocol::{receive, send, MiniProtocol}, store::{self, ecg::{self, ECGHeader}, ECGStatus, HandlePeerRequest, StoreCommand, UntypedStoreCommand}, util::Stream};


#[derive(Debug, Serialize, Deserialize)]
pub(crate) enum MsgStoreSync<Hash> {
    Request(MsgStoreSyncRequest<Hash>),
    MetadataHeaderResponse(MsgStoreSyncMetadataResponse<Hash>),
    MerkleResponse(MsgStoreSyncMerkleResponse<Hash>),
    PiecesResponse(MsgStoreSyncPieceResponse),
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) enum MsgStoreSyncRequest<Hash> {
    MetadataHeader,
    MerkleHashes {
        ranges: Vec<Range<u64>>,
    },
    InitialStatePieces {
        ranges: Vec<Range<u64>>,
    },
    ECGSync {
        meet: Vec<Hash>,
        tip: Vec<Hash>,
    }
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
pub(crate) struct MsgStoreSyncMerkleResponse<Hash> (StoreSyncResponse<Vec<Hash>>);

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct MsgStoreSyncPieceResponse (StoreSyncResponse<Vec<Option<Vec<u8>>>>);

impl<Hash> Into<MsgStoreSync<Hash>> for MsgStoreSyncRequest<Hash> {
    fn into(self) -> MsgStoreSync<Hash> {
        MsgStoreSync::Request(self)
    }
}

impl<Hash> TryInto<MsgStoreSyncRequest<Hash>> for MsgStoreSync<Hash> {
    type Error = ();
    fn try_into(self) -> Result<MsgStoreSyncRequest<Hash>, ()> {
        match self {
            MsgStoreSync::Request(r) => Ok(r),
            MsgStoreSync::MetadataHeaderResponse(_) => Err(()),
            MsgStoreSync::MerkleResponse(_) => Err(()),
            MsgStoreSync::PiecesResponse(_) => Err(()),
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
            MsgStoreSync::PiecesResponse(_) => Err(()),
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
            MsgStoreSync::PiecesResponse(_) => Err(()),
        }
    }
}

impl<StoreId> Into<MsgStoreSync<StoreId>> for MsgStoreSyncPieceResponse {
    fn into(self) -> MsgStoreSync<StoreId> {
        MsgStoreSync::PiecesResponse(self)
    }
}

impl<StoreId> TryInto<MsgStoreSyncPieceResponse> for MsgStoreSync<StoreId> {
    type Error = ();
    fn try_into(self) -> Result<MsgStoreSyncPieceResponse, ()> {
        match self {
            MsgStoreSync::Request(_) => Err(()),
            MsgStoreSync::MetadataHeaderResponse(_) => Err(()),
            MsgStoreSync::MerkleResponse(_) => Err(()),
            MsgStoreSync::PiecesResponse(r) => Ok(r),
        }
    }
}

pub(crate) struct StoreSync<Header: ECGHeader<T>, T, Hash> {
    peer: DeviceId,
    // Receive commands from store if we have initiatives or send commands to store if we're the responder.
    recv_chan: Option<UnboundedReceiver<StoreSyncCommand<Hash>>>,
    // Send commands to store if we're the responder and send results back to store if we're the initiator.
    send_chan: UnboundedSender<StoreCommand<Header, T, Hash>>, // JP: Make this a stream?
}

impl<Header: ECGHeader<T>, T, Hash> StoreSync<Header, T, Hash> {
    pub(crate) fn new_server(peer: DeviceId, recv_chan: UnboundedReceiver<StoreSyncCommand<Hash>>, send_chan: UnboundedSender<StoreCommand<Header, T, Hash>>) -> Self {
        let recv_chan = Some(recv_chan);
        Self { peer, recv_chan, send_chan }
    }

    pub(crate) fn new_client(peer: DeviceId, send_chan: UnboundedSender<StoreCommand<Header, T, Hash>>) -> Self {
        Self { peer, recv_chan: None, send_chan }
    }



    #[inline(always)]
    async fn run_client_helper<S: Stream<MsgStoreSync<Hash>>, Req, Resp, SResp>(&self, stream: &mut S, request: Req, build_command: fn(HandlePeerRequest<Req, Resp>) -> StoreCommand<Header, T, Hash>, build_response: fn(StoreSyncResponse<Resp>) -> SResp)
    where
        Hash: Debug,
        SResp: Into<MsgStoreSync<Hash>> + Debug,
        Resp: Debug,
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

pub(crate) enum StoreSyncCommand<Hash> {
    MetadataHeaderRequest,
    MerkleRequest(Vec<Range<u64>>),
    InitialStatePieceRequest(Vec<Range<u64>>),
    ECGSyncRequest{
        ecg_status: ECGStatus<Hash>,
        // ecg_state: ecg::State<Header, T>,
    },
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
                        send(&mut stream, MsgStoreSyncRequest::MerkleHashes{ ranges: ranges.clone() }).await.expect("TODO");

                        let MsgStoreSyncMerkleResponse(result) = receive(&mut stream).await.expect("TODO");
                        match result {
                            StoreSyncResponse::Response(pieces) => {
                                // Send store the piece hashes and tell store we're ready.
                                let msg = UntypedStoreCommand::ReceivedMerkleHashes {peer: self.peer, ranges, pieces};
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
                    StoreSyncCommand::InitialStatePieceRequest(ranges) => {
                        send(&mut stream, MsgStoreSyncRequest::InitialStatePieces { ranges: ranges.clone() }).await.expect("TODO");
                        let MsgStoreSyncPieceResponse(result) = receive(&mut stream).await.expect("TODO");
                        match result {
                            StoreSyncResponse::Response(pieces) => {
                                // Send store the pieces and tell store we're ready.
                                let msg = UntypedStoreCommand::ReceivedInitialStatePieces {peer: self.peer, ranges, pieces};
                                self.send_chan.send(msg).expect("TODO");
                            }
                            StoreSyncResponse::Wait =>
                                // Wait for response (Must be Reject or Repsonse).
                                todo!(),
                            StoreSyncResponse::Reject =>
                                // Send store they rejected and tell store we're ready.
                                todo!(),
                        }
                    }
                    StoreSyncCommand::ECGSyncRequest { ecg_status } => {
                        if ecg_status.meet_needs_update {
                            // TODO: Actually figure out meet.
                            warn!("TODO: Actually figure out meet");
                            let meet = ecg_status.meet;
                            let msg = UntypedStoreCommand::ReceivedUpdatedMeet { peer: self.peer, meet};
                            self.send_chan.send(msg).expect("TODO");
                        } else {
                            // TODO: Only request updates if they have a tip we don't know or if they're caught up to us?
                            warn!("TODO: Only request updates if they have a tip we don't know or if they're caught up to us?");

                            let tip = todo!();
                            let msg = MsgStoreSyncRequest::ECGSync { meet: ecg_status.meet, tip };
                            send(&mut stream, msg).await.expect("TODO");
                            let MsgStoreSyncPieceResponse(result) = receive(&mut stream).await.expect("TODO");
                            match result {
                                StoreSyncResponse::Response(ooperations) => {
                                    todo!();
                                }
                                StoreSyncResponse::Wait =>
                                    // Wait for response (Must be Reject or Repsonse).
                                    todo!(),
                                StoreSyncResponse::Reject =>
                                    // Send store they rejected and tell store we're ready.
                                    todo!(),
                            }
                        }
                    }
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
                        const fn build_command<H>(req: HandlePeerRequest<(), store::v0::MetadataHeader<H>>) -> UntypedStoreCommand<H> {
                            UntypedStoreCommand::HandleMetadataPeerRequest(req)
                        }
                        const fn build_response<H>(h: StoreSyncResponse<store::v0::MetadataHeader<H>>) -> MsgStoreSyncMetadataResponse<H> {
                            MsgStoreSyncMetadataResponse(h)
                        }

                        self.run_client_helper::<_, (), store::v0::MetadataHeader<StoreId>, MsgStoreSyncMetadataResponse<StoreId>>(&mut stream, (), build_command, build_response).await;
                    }
                    MsgStoreSyncRequest::MerkleHashes { ranges } => {
                        const fn build_command<H>(req: HandlePeerRequest<Vec<Range<u64>>, Vec<H>>) -> UntypedStoreCommand<H> {
                            UntypedStoreCommand::HandleMerklePeerRequest(req)
                        }
                        const fn build_response<H>(h: StoreSyncResponse<Vec<H>>) -> MsgStoreSyncMerkleResponse<H> {
                            MsgStoreSyncMerkleResponse(h)
                        }

                        self.run_client_helper::<_, Vec<Range<u64>>, Vec<StoreId>, MsgStoreSyncMerkleResponse<StoreId>>(&mut stream, ranges, build_command, build_response).await;
                    }
                    MsgStoreSyncRequest::InitialStatePieces { ranges } => {
                        const fn build_command<H>(req: HandlePeerRequest<Vec<Range<u64>>, Vec<Option<Vec<u8>>>>) -> UntypedStoreCommand<H> {
                            UntypedStoreCommand::HandlePiecePeerRequest(req)
                        }
                        const fn build_response(h: StoreSyncResponse<Vec<Option<Vec<u8>>>>) -> MsgStoreSyncPieceResponse {
                            MsgStoreSyncPieceResponse(h)
                        }

                        self.run_client_helper::<_, Vec<Range<u64>>, Vec<Option<Vec<u8>>>, MsgStoreSyncPieceResponse>(&mut stream, ranges, build_command, build_response).await;
                    }
                    MsgStoreSyncRequest::ECGSync { meet, tip } => {
                        const fn build_command<H>(req: HandlePeerRequest<(Vec<H>, Vec<H>), Vec<()>>) -> UntypedStoreCommand<H> {
                            todo!()
                        }
                        const fn build_response(h: StoreSyncResponse<Vec<()>>) -> MsgStoreSyncPieceResponse {
                            todo!()
                        }
                        self.run_client_helper::<_, _, _, _>(&mut stream, (meet, tip), build_command, build_response).await;
                    }
                }
            }
        }
    }
}
