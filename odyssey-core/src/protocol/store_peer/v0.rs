use std::{fmt::Debug, future::Future, marker::PhantomData, ops::Range};

use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::UnboundedReceiver;
use tracing::debug;

use crate::{network::protocol::{receive, send, MiniProtocol}, store, util::Stream};


#[derive(Debug, Serialize, Deserialize)]
pub(crate) enum MsgStoreSync<StoreId> {
    Request(MsgStoreSyncRequest),
    MetadataHeaderResponse(MsgStoreSyncMetadataHeaderResponse<StoreId>),
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) enum MsgStoreSyncRequest {
    MetadataHeaderRequest,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) enum MsgStoreSyncMetadataHeaderResponse<StoreId> {
    MetadataHeader(store::v0::MetadataHeader<StoreId>),
    // Don't currently know, wait until we share it with you.
    Wait,
    // They may or may not have the metadata, but they canceled since the request timed out or think we already have it.
    Cancel,
    // They don't have the metadata.
    Unknown,
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

impl<StoreId> Into<MsgStoreSync<StoreId>> for MsgStoreSyncMetadataHeaderResponse<StoreId> {
    fn into(self) -> MsgStoreSync<StoreId> {
        MsgStoreSync::MetadataHeaderResponse(self)
    }
}

impl<StoreId> TryInto<MsgStoreSyncMetadataHeaderResponse<StoreId>> for MsgStoreSync<StoreId> {
    type Error = ();
    fn try_into(self) -> Result<MsgStoreSyncMetadataHeaderResponse<StoreId>, ()> {
        match self {
            MsgStoreSync::Request(_) => Err(()),
            MsgStoreSync::MetadataHeaderResponse(r) => Ok(r),
        }
    }
}

pub(crate) struct StoreSync<StoreId> {
    command_chan: Option<UnboundedReceiver<StoreSyncCommand>>,
    _phantom: PhantomData<StoreId>,
}

impl<StoreId> StoreSync<StoreId> {
    pub(crate) fn new_server(command_chan: UnboundedReceiver<StoreSyncCommand>) -> Self {
        let command_chan = Some(command_chan);
        Self { command_chan, _phantom: PhantomData }
    }

    pub(crate) fn new_client() -> Self {
        let command_chan = None;
        Self { command_chan, _phantom: PhantomData }
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
            let mut command_chan = self.command_chan.expect("Unreachable. Server must be given a receive channel.");
            while let Some(cmd) = command_chan.recv().await {
                match cmd {
                    StoreSyncCommand::MetadataHeaderRequest => {
                        send(&mut stream, MsgStoreSyncRequest::MetadataHeaderRequest).await.expect("TODO");

                        let result = receive(&mut stream).await.expect("TODO");
                        match result {
                            MsgStoreSyncMetadataHeaderResponse::Unknown => {
                                // Tell store they don't have the metadata and we're ready.
                                todo!();
                            }
                            MsgStoreSyncMetadataHeaderResponse::MetadataHeader(metadata) => {
                                // Send store the metadata and tell store we're ready.
                                todo!();
                            }
                            MsgStoreSyncMetadataHeaderResponse::Wait => {
                                // Wait for response (Must be Cancel or MetadataHeader).
                                todo!();
                            }
                            MsgStoreSyncMetadataHeaderResponse::Cancel => {
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


            todo!();
        }
    }

    fn run_client<S: Stream<Self::Message>>(self, stream: S) -> impl Future<Output = ()> + Send {
        async move {
            todo!();
        }
    }
}
