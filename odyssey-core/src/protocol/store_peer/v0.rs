use std::{fmt::Debug, future::Future, marker::PhantomData, ops::Range};

use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc::{UnboundedReceiver, UnboundedSender}, oneshot};
use tracing::debug;

use crate::{auth::DeviceId, network::protocol::{receive, send, MiniProtocol}, store::{self, UntypedStoreCommand}, util::Stream};


#[derive(Debug, Serialize, Deserialize)]
pub(crate) enum MsgStoreSync<StoreId> {
    Request(MsgStoreSyncRequest),
    MetadataHeaderResponse(MsgStoreSyncMetadataResponse<StoreId>),
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) enum MsgStoreSyncRequest {
    MetadataHeaderRequest,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) enum MsgStoreSyncMetadataResponse<StoreId> {
    Metadata(store::v0::MetadataHeader<StoreId>),
    // Don't currently know, wait until we share it with you.
    Wait,
    // They may or may not have the metadata, but they rejected the request.
    Reject,
}

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
                        send(&mut stream, MsgStoreSyncRequest::MetadataHeaderRequest).await.expect("TODO");

                        let result = receive(&mut stream).await.expect("TODO");
                        match result {
                            MsgStoreSyncMetadataResponse::Metadata(metadata) => {

                                // Send store the metadata and tell store we're ready.
                                let msg = UntypedStoreCommand::ReceivedMetadata { peer: self.peer, metadata };
                                self.send_chan.send(msg).expect("TODO");
                            }
                            MsgStoreSyncMetadataResponse::Wait => {
                                // Wait for response (Must be Cancel or MetadataHeader).
                                todo!();
                            }
                            MsgStoreSyncMetadataResponse::Reject => {
                                // Send store they canceled and tell store we're ready.
                                todo!();
                            }
                        }
                    }
                    StoreSyncCommand::MerkleRequest(ranges) => todo!(),
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
                    MsgStoreSyncRequest::MetadataHeaderRequest => {
                        // Send request to store.
                        let (response_chan, recv_chan) = oneshot::channel();
                        let cmd = UntypedStoreCommand::HandlePeerRequest {
                            peer: self.peer,
                            request,
                            response_chan,
                        };
                        self.send_chan.send(cmd).expect("TODO");

                        // Wait for response.
                        match recv_chan.await.expect("TODO") {
                            Ok(metadata) => {
                                // Send response to peer.
                                send(&mut stream, MsgStoreSyncMetadataResponse::Metadata(metadata)).await.expect("TODO");
                            }
                            Err(chan) => {
                                // If waiting, tell peer.
                                send(&mut stream, MsgStoreSyncMetadataResponse::Wait).await.expect("TODO");

                                // Wait for response and send to peer.
                                let response = if let Some(metadata) = chan.await.expect("TODO") {
                                    MsgStoreSyncMetadataResponse::Metadata(metadata)
                                } else {
                                    MsgStoreSyncMetadataResponse::Reject
                                };

                                send(&mut stream, response).await.expect("TODO");
                            }
                        }
                    }
                }
            }
        }
    }
}
