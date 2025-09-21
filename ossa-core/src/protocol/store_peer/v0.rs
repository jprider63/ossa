use std::{collections::BTreeSet, fmt::Debug, future::Future, ops::Range};

use serde::{Deserialize, Serialize};
use tokio::sync::{
    mpsc::{UnboundedReceiver, UnboundedSender},
    oneshot,
};
use tracing::debug;

use crate::{
    auth::DeviceId,
    network::protocol::{receive, send, MiniProtocol},
    protocol::store_peer::ecg_sync::{ECGSyncInitiator, ECGSyncResponder, DAGStateSubscriber, MsgDAGSyncRequest, MsgDAGSyncResponse},
    store::{
        self,
        dag,
        HandlePeerRequest, UntypedStoreCommand,
    },
    util::Stream,
};

#[derive(Debug, Serialize, Deserialize)]
pub(crate) enum MsgStoreSync<Hash, HeaderId, Header> {
    Request(MsgStoreSyncRequest<HeaderId>),
    MetadataHeaderResponse(MsgStoreSyncMetadataResponse<Hash>),
    MerkleResponse(MsgStoreSyncMerkleResponse<Hash>),
    BlocksResponse(MsgStoreSyncBlockResponse),
    ECGResponse(MsgDAGSyncResponse<HeaderId, Header>),
}

/// The maximum number of `have` hashes that can be sent in each message.
pub const MAX_HAVE_HEADERS: u16 = 32;
/// The maximum number of headers that can be sent in each message.
pub const MAX_DELIVER_HEADERS: u16 = 32;

#[derive(Debug, Serialize, Deserialize)]
pub(crate) enum MsgStoreSyncRequest<HeaderId> {
    MetadataHeader,
    MerkleHashes {
        ranges: Vec<Range<u64>>,
    },
    InitialStateBlocks {
        ranges: Vec<Range<u64>>,
    },
    ECGSync {
        request: MsgDAGSyncRequest<HeaderId>,
    },
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct MsgStoreSyncMetadataResponse<Hash>(
    StoreSyncResponse<store::v0::MetadataHeader<Hash>>,
);

#[derive(Debug, Serialize, Deserialize)]
pub(crate) enum StoreSyncResponse<A> {
    Response(A),
    // Don't currently know, wait until we share it with you.
    Wait,
    // They may or may not have the metadata, but they rejected the request.
    Reject,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct MsgStoreSyncMerkleResponse<Hash>(StoreSyncResponse<Vec<Hash>>);

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct MsgStoreSyncBlockResponse(StoreSyncResponse<Vec<Option<Vec<u8>>>>);

impl<Hash, HeaderId, Header> Into<MsgStoreSync<Hash, HeaderId, Header>>
    for MsgStoreSyncRequest<HeaderId>
{
    fn into(self) -> MsgStoreSync<Hash, HeaderId, Header> {
        MsgStoreSync::Request(self)
    }
}

impl<Hash, HeaderId, Header> TryInto<MsgStoreSyncRequest<HeaderId>>
    for MsgStoreSync<Hash, HeaderId, Header>
{
    type Error = ();
    fn try_into(self) -> Result<MsgStoreSyncRequest<HeaderId>, ()> {
        match self {
            MsgStoreSync::Request(r) => Ok(r),
            MsgStoreSync::MetadataHeaderResponse(_) => Err(()),
            MsgStoreSync::MerkleResponse(_) => Err(()),
            MsgStoreSync::BlocksResponse(_) => Err(()),
            MsgStoreSync::ECGResponse(_) => Err(()),
        }
    }
}

impl<Hash, HeaderId, Header> Into<MsgStoreSync<Hash, HeaderId, Header>>
    for MsgDAGSyncRequest<HeaderId>
{
    fn into(self) -> MsgStoreSync<Hash, HeaderId, Header> {
        MsgStoreSync::Request( MsgStoreSyncRequest::ECGSync { request: self })
    }
}

impl<Hash, HeaderId, Header> Into<MsgStoreSync<Hash, HeaderId, Header>>
    for MsgStoreSyncMetadataResponse<Hash>
{
    fn into(self) -> MsgStoreSync<Hash, HeaderId, Header> {
        MsgStoreSync::MetadataHeaderResponse(self)
    }
}

impl<Hash, HeaderId, Header> TryInto<MsgStoreSyncMetadataResponse<Hash>>
    for MsgStoreSync<Hash, HeaderId, Header>
{
    type Error = ();
    fn try_into(self) -> Result<MsgStoreSyncMetadataResponse<Hash>, ()> {
        match self {
            MsgStoreSync::Request(_) => Err(()),
            MsgStoreSync::MetadataHeaderResponse(r) => Ok(r),
            MsgStoreSync::MerkleResponse(_) => Err(()),
            MsgStoreSync::BlocksResponse(_) => Err(()),
            MsgStoreSync::ECGResponse(_) => Err(()),
        }
    }
}

impl<Hash, HeaderId, Header> Into<MsgStoreSync<Hash, HeaderId, Header>>
    for MsgStoreSyncMerkleResponse<Hash>
{
    fn into(self) -> MsgStoreSync<Hash, HeaderId, Header> {
        MsgStoreSync::MerkleResponse(self)
    }
}

impl<Hash, HeaderId, Header> TryInto<MsgStoreSyncMerkleResponse<Hash>>
    for MsgStoreSync<Hash, HeaderId, Header>
{
    type Error = ();
    fn try_into(self) -> Result<MsgStoreSyncMerkleResponse<Hash>, ()> {
        match self {
            MsgStoreSync::Request(_) => Err(()),
            MsgStoreSync::MetadataHeaderResponse(_) => Err(()),
            MsgStoreSync::MerkleResponse(r) => Ok(r),
            MsgStoreSync::BlocksResponse(_) => Err(()),
            MsgStoreSync::ECGResponse(_) => Err(()),
        }
    }
}

impl<Hash, HeaderId, Header> Into<MsgStoreSync<Hash, HeaderId, Header>>
    for MsgStoreSyncBlockResponse
{
    fn into(self) -> MsgStoreSync<Hash, HeaderId, Header> {
        MsgStoreSync::BlocksResponse(self)
    }
}

impl<Hash, HeaderId, Header> TryInto<MsgStoreSyncBlockResponse>
    for MsgStoreSync<Hash, HeaderId, Header>
{
    type Error = ();
    fn try_into(self) -> Result<MsgStoreSyncBlockResponse, ()> {
        match self {
            MsgStoreSync::Request(_) => Err(()),
            MsgStoreSync::MetadataHeaderResponse(_) => Err(()),
            MsgStoreSync::MerkleResponse(_) => Err(()),
            MsgStoreSync::BlocksResponse(r) => Ok(r),
            MsgStoreSync::ECGResponse(_) => Err(()),
        }
    }
}

impl<Hash, HeaderId, Header> Into<MsgStoreSync<Hash, HeaderId, Header>>
    for MsgDAGSyncResponse<HeaderId, Header>
{
    fn into(self) -> MsgStoreSync<Hash, HeaderId, Header> {
        MsgStoreSync::ECGResponse(self)
    }
}

impl<Hash, HeaderId, Header> TryInto<MsgDAGSyncResponse<HeaderId, Header>>
    for MsgStoreSync<Hash, HeaderId, Header>
{
    type Error = ();
    fn try_into(self) -> Result<MsgDAGSyncResponse<HeaderId, Header>, ()> {
        match self {
            MsgStoreSync::Request(_) => Err(()),
            MsgStoreSync::MetadataHeaderResponse(_) => Err(()),
            MsgStoreSync::MerkleResponse(_) => Err(()),
            MsgStoreSync::BlocksResponse(_) => Err(()),
            MsgStoreSync::ECGResponse(r) => Ok(r),
        }
    }
}

pub(crate) struct StoreSync<Hash, HeaderId, Header> {
    peer: DeviceId,
    // Receive commands from store if we have initiative or send commands to store if we're the responder.
    recv_chan: Option<UnboundedReceiver<StoreSyncCommand<HeaderId, Header>>>,
    // Send commands to store if we're the responder and send results back to store if we're the initiator.
    send_chan: UnboundedSender<UntypedStoreCommand<Hash, HeaderId, Header>>, // JP: Make this a stream?
}

impl<Hash, HeaderId, Header> DAGStateSubscriber<Hash, HeaderId, Header> for StoreSync<Hash, HeaderId, Header>
where
    HeaderId: Ord + Copy,
{
    async fn request_dag_state(
        &self,
        responder: &mut ECGSyncResponder<Hash, HeaderId, Header>,
        tips: Option<BTreeSet<HeaderId>>,
    ) -> dag::UntypedState<HeaderId, Header> {
        debug!("Requesting ECG state");

        // Send request to store.
        let (response_chan, recv_chan) = oneshot::channel(); // TODO: Use tokio::sync::watch?
        let cmd = UntypedStoreCommand::SubscribeECG {
            peer: self.peer(),
            tips,
            response_chan,
        };
        self.send_chan().send(cmd).expect("TODO");

        // Wait for ECG updates.
        let state = recv_chan.await.expect("TODO");
        responder.update_our_unknown(&state);

        debug!("Received ECG state");

        state
    }
}

impl<Hash, HeaderId, Header> StoreSync<Hash, HeaderId, Header> {
    pub(crate) fn new_server(
        peer: DeviceId,
        recv_chan: UnboundedReceiver<StoreSyncCommand<HeaderId, Header>>,
        send_chan: UnboundedSender<UntypedStoreCommand<Hash, HeaderId, Header>>,
    ) -> Self {
        let recv_chan = Some(recv_chan);
        Self {
            peer,
            recv_chan,
            send_chan,
        }
    }

    pub(crate) fn new_client(
        peer: DeviceId,
        send_chan: UnboundedSender<UntypedStoreCommand<Hash, HeaderId, Header>>,
    ) -> Self {
        Self {
            peer,
            recv_chan: None,
            send_chan,
        }
    }

    #[inline(always)]
    async fn run_client_helper<S: Stream<MsgStoreSync<Hash, HeaderId, Header>>, Req, Resp, SResp>(
        &self,
        stream: &mut S,
        request: Req,
        build_command: fn(
            HandlePeerRequest<Req, Resp>,
        ) -> UntypedStoreCommand<Hash, HeaderId, Header>,
        build_response: fn(StoreSyncResponse<Resp>) -> SResp,
    ) where
        Hash: Debug,
        SResp: Into<MsgStoreSync<Hash, HeaderId, Header>> + Debug,
        Req: Debug,
        Resp: Debug,
    {
        debug!("Received request from peer: {:?}", request);

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
                send(
                    stream,
                    MsgStoreSyncMetadataResponse(StoreSyncResponse::Wait),
                )
                .await
                .expect("TODO");

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

    pub(crate) fn peer(&self) -> DeviceId {
        self.peer
    }

    pub(crate) fn send_chan(
        &self,
    ) -> &UnboundedSender<UntypedStoreCommand<Hash, HeaderId, Header>> {
        &self.send_chan
    }
}

#[derive(Debug)]
pub(crate) enum StoreSyncCommand<HeaderId, Header> {
    MetadataHeaderRequest,
    MerkleRequest(Vec<Range<u64>>),
    InitialStateBlockRequest(Vec<Range<u64>>),
    ECGSyncRequest {
        // ecg_status: ECGStatus<HeaderId>,
        ecg_state: dag::UntypedState<HeaderId, Header>,
    },
}

impl<
        Hash: Debug + Serialize + for<'a> Deserialize<'a> + Send + Sync + Clone,
        HeaderId: Debug + Ord + Serialize + for<'a> Deserialize<'a> + Send + Sync + Clone + Copy,
        Header: Clone + Debug + Send + Sync + Serialize + for<'a> Deserialize<'a>,
    > MiniProtocol for StoreSync<Hash, HeaderId, Header>
{
    type Message = MsgStoreSync<Hash, HeaderId, Header>;

    // Has initiative
    fn run_server<S: Stream<Self::Message>>(
        self,
        mut stream: S,
    ) -> impl Future<Output = ()> + Send {
        async move {
            let mut ecg_sync: Option<ECGSyncInitiator<Hash, HeaderId, Header>> = None;

            // Wait for command from store.
            let mut recv_chan = self
                .recv_chan
                .expect("Unreachable. Server must be given a receive channel.");
            while let Some(cmd) = recv_chan.recv().await {
                match cmd {
                    StoreSyncCommand::MetadataHeaderRequest => {
                        send(&mut stream, MsgStoreSyncRequest::MetadataHeader)
                            .await
                            .expect("TODO");

                        let MsgStoreSyncMetadataResponse(result) =
                            receive(&mut stream).await.expect("TODO");
                        match result {
                            StoreSyncResponse::Response(metadata) => {
                                // Send store the metadata and tell store we're ready.
                                let msg = UntypedStoreCommand::ReceivedMetadata {
                                    peer: self.peer,
                                    metadata,
                                };
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
                        debug!("Sending MerkleHashes request: {:?}", ranges);
                        send(
                            &mut stream,
                            MsgStoreSyncRequest::MerkleHashes {
                                ranges: ranges.clone(),
                            },
                        )
                        .await
                        .expect("TODO");

                        let MsgStoreSyncMerkleResponse(result) =
                            receive(&mut stream).await.expect("TODO");
                        debug!("Received merkle response: {:?}", result);
                        match result {
                            StoreSyncResponse::Response(nodes) => {
                                // Send store the merkle hashes and tell store we're ready.
                                let msg = UntypedStoreCommand::ReceivedMerkleHashes {
                                    peer: self.peer,
                                    ranges,
                                    nodes,
                                };
                                self.send_chan.send(msg).expect("TODO");
                            }
                            StoreSyncResponse::Wait => {
                                // Wait for response (Must be Cancel or MerkleHashes).
                                todo!();
                            }
                            StoreSyncResponse::Reject => {
                                // Send store they canceled and tell store we're ready.
                                todo!();
                            }
                        }
                    }
                    StoreSyncCommand::InitialStateBlockRequest(ranges) => {
                        send(
                            &mut stream,
                            MsgStoreSyncRequest::InitialStateBlocks {
                                ranges: ranges.clone(),
                            },
                        )
                        .await
                        .expect("TODO");
                        let MsgStoreSyncBlockResponse(result) =
                            receive(&mut stream).await.expect("TODO");
                        match result {
                            StoreSyncResponse::Response(blocks) => {
                                // Send store the blocks and tell store we're ready.
                                let msg = UntypedStoreCommand::ReceivedInitialStateBlocks {
                                    peer: self.peer,
                                    ranges,
                                    blocks,
                                };
                                self.send_chan.send(msg).expect("TODO");
                            }
                            StoreSyncResponse::Wait =>
                            // Wait for response (Must be Reject or Response).
                            {
                                todo!()
                            }
                            StoreSyncResponse::Reject =>
                            // Send store they rejected and tell store we're ready.
                            {
                                todo!()
                            }
                        }
                    }
                    StoreSyncCommand::ECGSyncRequest { ecg_state } => {
                        // Run ECG sync to get new operations from peer.
                        let operations = match ecg_sync {
                            None => {
                                // First round of ECG sync, so create and run first round.

                                // JP: Eventually switch ecg_state to an Arc<RWLock>?
                                let (new_ecg_sync, operations) =
                                    ECGSyncInitiator::run_new(&mut stream, &ecg_state).await;
                                ecg_sync = Some(new_ecg_sync);
                                operations
                            }
                            Some(ref mut ecg_sync) => {
                                // Subsequent rounds of ECG sync.
                                ecg_sync.run_round(&mut stream, &ecg_state).await
                            }
                        };

                        debug!("Received ECG operations from peer: {:?}", operations);
                        // JP: Should we check if the operation set is empty?
                        // if !operations.is_empty() {
                        let msg = UntypedStoreCommand::ReceivedECGOperations {
                            peer: self.peer,
                            operations,
                        };
                        self.send_chan.send(msg).expect("TODO");
                        // } else { todo!() }
                    }
                }
            }

            debug!("StorePeer client miniprotocol ended");

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

    fn run_client<S: Stream<Self::Message>>(
        self,
        mut stream: S,
    ) -> impl Future<Output = ()> + Send {
        async move {
            let mut ecg_sync: Option<ECGSyncResponder<Hash, HeaderId, Header>> = None;

            // TODO: Check when done.
            loop {
                // Receive request.
                let request = receive(&mut stream).await.expect("TODO");
                match request {
                    MsgStoreSyncRequest::MetadataHeader => {
                        const fn build_command<Hash, HeaderId, Header>(
                            req: HandlePeerRequest<(), store::v0::MetadataHeader<Hash>>,
                        ) -> UntypedStoreCommand<Hash, HeaderId, Header> {
                            UntypedStoreCommand::HandleMetadataPeerRequest(req)
                        }
                        const fn build_response<Hash, HeaderId, Header>(
                            h: StoreSyncResponse<store::v0::MetadataHeader<Hash>>,
                        ) -> MsgStoreSyncMetadataResponse<Hash> {
                            MsgStoreSyncMetadataResponse(h)
                        }

                        self.run_client_helper::<_, (), store::v0::MetadataHeader<Hash>, MsgStoreSyncMetadataResponse<Hash>>(&mut stream, (), build_command, build_response::<_, HeaderId, Header>).await;
                    }
                    MsgStoreSyncRequest::MerkleHashes { ranges } => {
                        const fn build_command<Hash, HeaderId, Header>(
                            req: HandlePeerRequest<Vec<Range<u64>>, Vec<Hash>>,
                        ) -> UntypedStoreCommand<Hash, HeaderId, Header> {
                            UntypedStoreCommand::HandleMerklePeerRequest(req)
                        }
                        const fn build_response<H>(
                            h: StoreSyncResponse<Vec<H>>,
                        ) -> MsgStoreSyncMerkleResponse<H> {
                            MsgStoreSyncMerkleResponse(h)
                        }

                        self.run_client_helper::<_, Vec<Range<u64>>, Vec<Hash>, MsgStoreSyncMerkleResponse<Hash>>(&mut stream, ranges, build_command, build_response).await;
                    }
                    MsgStoreSyncRequest::InitialStateBlocks { ranges } => {
                        const fn build_command<Hash, HeaderId, Header>(
                            req: HandlePeerRequest<Vec<Range<u64>>, Vec<Option<Vec<u8>>>>,
                        ) -> UntypedStoreCommand<Hash, HeaderId, Header> {
                            UntypedStoreCommand::HandleBlockPeerRequest(req)
                        }
                        const fn build_response(
                            h: StoreSyncResponse<Vec<Option<Vec<u8>>>>,
                        ) -> MsgStoreSyncBlockResponse {
                            MsgStoreSyncBlockResponse(h)
                        }

                        self.run_client_helper::<_, Vec<Range<u64>>, Vec<Option<Vec<u8>>>, MsgStoreSyncBlockResponse>(&mut stream, ranges, build_command, build_response).await;
                    }
                    MsgStoreSyncRequest::ECGSync { request } => {
                        match request {
                            MsgDAGSyncRequest::DAGInitialSync { tips } => {
                                debug!("Received initial ECG sync request with tips: {tips:?}");

                                if ecg_sync.is_some() {
                                    todo!("TODO: Error, ECG sync has already been initialized.");
                                }

                                let mut ecg_sync_ = ECGSyncResponder::new();

                                let ecg_state = self.request_dag_state(&mut ecg_sync_, None).await;

                                ecg_sync_
                                    .run_initial(&self, &mut stream, ecg_state, tips)
                                    .await;
                                ecg_sync = Some(ecg_sync_);
                            }
                            MsgDAGSyncRequest::DAGSync { tips, known } => {
                                let Some(ref mut ecg_sync) = ecg_sync else {
                                    todo!("TODO: Error, ECG sync hasn't been initialized.");
                                };

                                let ecg_state = self.request_dag_state(ecg_sync, None).await;

                                ecg_sync
                                    .run_round(&self, &mut stream, ecg_state, tips, known)
                                    .await;
                            }
                        }
                    }
                }
            }
            debug!("StoreSyncCommand receiver channel closed");
        }
    }
}
