// This is very similar to DAG sync, except there are a few key differences:
// - The DAG is bipartite between block and certificate nodes
// - We have rounds, so we can take advantage of that (ex, only send previous round blocks if it has a full threshold signature?)
//
// Do we need to go back more than 1 round?

use std::{future::Future, marker::PhantomData};

use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc::{UnboundedReceiver, UnboundedSender}, watch};
use tracing::{debug, warn};

use crate::{auth::DeviceId, network::protocol::{receive, send, MiniProtocol}, store::{bft::{BFTState, Block, BlockId, PartialSignature, Round, ThresholdSignature}, dag, UntypedStoreCommand}, util::{Sha256Hash, Stream}};


pub(crate) struct StoreBFTSync<Hash, SHeaderId, SHeader, THeaderId, THeader> {
    peer: DeviceId,
    // Receive commands from store if we have initiative or send commands to store if we're the responder.
    recv_chan: Option<UnboundedReceiver<StoreBFTSyncCommand>>,
    // Send commands to store if we're the responder and send results back to store if we're the initiator.
    send_chan: UnboundedSender<UntypedStoreCommand<Hash, SHeaderId, SHeader, THeaderId, THeader>>, // JP: Make this a stream?
    // BFT state
    bft_state: watch::Receiver<BFTState<SHeaderId>>,
}

impl<Hash, SHeaderId, SHeader, THeaderId, THeader> StoreBFTSync<Hash, SHeaderId, SHeader, THeaderId, THeader> {
    pub(crate) fn new_server(
        peer: DeviceId,
        recv_chan: UnboundedReceiver<StoreBFTSyncCommand>,
        send_chan: UnboundedSender<
            UntypedStoreCommand<Hash, SHeaderId, SHeader, THeaderId, THeader>,
        >,
        bft_state: watch::Receiver<BFTState<SHeaderId>>,
    ) -> Self {
        let recv_chan = Some(recv_chan);
        StoreBFTSync {
            peer,
            recv_chan,
            send_chan,
            bft_state,
        }
    }

    pub(crate) fn new_client(
        peer: DeviceId,
        send_chan: UnboundedSender<
            UntypedStoreCommand<Hash, SHeaderId, SHeader, THeaderId, THeader>,
        >,
        bft_state: watch::Receiver<BFTState<SHeaderId>>,
    ) -> Self {
        StoreBFTSync {
            peer,
            recv_chan: None,
            send_chan,
            bft_state,
        }
    }
}

impl<Hash, SHeaderId, SHeader, THeaderId, THeader> MiniProtocol for StoreBFTSync<Hash, SHeaderId, SHeader, THeaderId, THeader>
where
    Hash: Send,
    SHeaderId: for<'a> Deserialize<'a> + Serialize + Send + Sync,
    SHeader: Send,
    THeaderId: Send,
    THeader: Send,
{
    type Message = MsgStoreBFTSync<SHeaderId>;

    // Has initiative
    // JP: Why does this have initiative again?
    fn run_server<S: Stream<Self::Message>>(
        mut self,
        mut stream: S,
    ) -> impl Future<Output = ()> + Send {
        async move {
            debug!("StoreDAGSync server running!");
            let mut bft_sync: Option<BFTSyncInitiator<SHeaderId>> = None;

            let mut recv_chan = self
                .recv_chan
                .expect("Unreachable. Server must be given a receive channel.");
            while let Some(cmd) = recv_chan.recv().await {
                match cmd {
                    StoreBFTSyncCommand::BFTSyncRequest => {
                        let updates = match bft_sync {
                            None => {
                                let (new_bft_sync, operations) =
                                    BFTSyncInitiator::run_new(
                                        &mut stream,
                                        &mut self.bft_state,
                                    )
                                    .await;
                                bft_sync = Some(new_bft_sync);
                                operations
                            }
                            Some(ref mut dag_sync) => {
                                todo!();
                            }
                        };
                        // let msg = UntypedStoreCommand::ReceivedBFTOperations {
                        //     peer: self.peer,
                        //     updates,
                        // };
                        // self.send_chan.send(msg).expect("TODO");
                        todo!();
                    }
                }
            }

            debug!("StoreBFTSync receiver channel closed");
        }
    }

    fn run_client<S: Stream<Self::Message>>(
        self,
        mut stream: S,
    ) -> impl Future<Output = ()> + Send {
        async move {
            debug!("StoreBFTSync client running!");
            let mut bft_sync: Option<BFTSyncResponder> = None;

            // TODO: Check when done.
            loop {
                // Receive request.
                let request = receive(&mut stream).await.expect("TODO");
                match request {
                    MsgBFTSyncRequest::BFTInitialSync { round, block_tips, round_complete } => {
                        debug!("Received initial BFT sync request with round: {round:?}\nblock_tips: {block_tips:?}\nround_complete: {round_complete:?}");

                        if bft_sync.is_some() {
                            todo!("TODO: Error, BFT sync has already been initialized.");
                        }

                        let mut bft_sync_ = BFTSyncResponder::new();
                        bft_sync_.run_initial().await;
                        bft_sync = Some(bft_sync_);



                        // TODO: If our round is behind their round, we may want to request updates from them
                    }
                }
            }
            debug!("StoreBFTSync client exited");
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) enum MsgStoreBFTSync<SHeaderId> {
    Request(MsgBFTSyncRequest),
    BFTResponse(MsgBFTSyncResponse<SHeaderId>),
}

pub(crate) type SignatureId = Sha256Hash;

// Either the ID of the aggregate signature or IDs of the partial signatures.
// JP: Or just send the signatures?
#[derive(Debug, Serialize, Deserialize)]
pub(crate) enum ThresholdSignatureId {
    Threshold(SignatureId),
    Partial(Vec<SignatureId>),
}

impl ThresholdSignatureId {
    pub fn new() -> Self {
        ThresholdSignatureId::Partial(vec![])
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) enum MsgBFTSyncRequest {
    BFTInitialSync {
        /// Current round we're on.
        round: Round,

        /// Tips/frontier of blocks with their round and corresponding signatures.
        block_tips: Vec<(Round, BlockId, ThresholdSignatureId)>,

        /// Signatures attesting to completion of the current round.
        // JP: Or just send the signatures?
        round_complete: ThresholdSignatureId,
    },
}

impl<SHeaderId> From<MsgBFTSyncRequest> for MsgStoreBFTSync<SHeaderId> {
    fn from(msg: MsgBFTSyncRequest) -> Self {
        MsgStoreBFTSync::Request(msg)
    }
}

impl<SHeaderId> TryInto<MsgBFTSyncRequest> for MsgStoreBFTSync<SHeaderId> {
    type Error = ();

    fn try_into(self) -> Result<MsgBFTSyncRequest, Self::Error> {
        match self {
            MsgStoreBFTSync::Request(msg_bftsync_request) => Ok(msg_bftsync_request),
            MsgStoreBFTSync::BFTResponse(_msg_bftsync_response) => Err(()),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) enum MsgBFTSyncResponse<SHeaderId> {
    Response {
        blocks: Vec<Block<SHeaderId>>,
        certificate_signatures: Vec<(BlockId, ThresholdSignature)>,
        certificate_partial_signatures: Vec<(BlockId, PartialSignature)>,

        round_complete_signatures: Vec<(Round, ThresholdSignature)>,
        round_complete_partial_signatures: Vec<(Round, DeviceId, PartialSignature)>,

        // TODO: Share what we have too? Still need to find the meet of our dags?
        // For old rounds, we only need to send blocks that are fully certified?? Or maybe that's not enough if 1 malicious node saw the aggregate signature?
    },
    Wait,
}


#[derive(Debug)]
pub(crate) enum StoreBFTSyncCommand {
    BFTSyncRequest,
    // {
    //     // ecg_status: ECGStatus<HeaderId>,
    //     dag_state: crate::store::dag::UntypedState<HeaderId, Header>,
    // },
}

pub struct BFTSyncInitiator<SHeaderId> {
    todo: PhantomData<SHeaderId>, // JP: Is SHeaderId needed?
}

impl<SHeaderId> BFTSyncInitiator<SHeaderId> {
    /// Create a new ECGSyncInitiator and run the first round.
    async fn run_new<S: Stream<MsgStoreBFTSync<SHeaderId>>>(stream: &mut S, bft_state: &mut watch::Receiver<BFTState<SHeaderId>>) -> (Self, Vec<()>) {
        let req = {
            // TODO: Limit on request sizes.
            warn!("TODO: Check request sizes.");
            // Acquire read lock on state.
            let bft_state = bft_state.borrow_and_update();
            let round = bft_state.current_round();
            let current_round = &bft_state.round_states()[round as usize];
            let block_tips = bft_state.previous_tips().iter().map(|(round, peer_id)| {
                let round_state = &bft_state.round_states()[*round as usize];
                let signed_block = round_state.blocks().get(peer_id).expect("Block not found even though it is a tip");
                let block = signed_block.value();
                let block_id = block.block_id();
                let signature_ids = round_state.certificates().get(&block_id).expect("Signature not found for previous block that's a tip").signature_ids();

                (*round, block_id, signature_ids)
            }).chain(
                current_round.blocks().values().map(|signed_block| {
                    let block = signed_block.value();
                    let block_id = block.block_id();
                    let signature_ids = current_round.certificates().get(&block_id).map_or_else(|| ThresholdSignatureId::new(), |s| s.signature_ids());
                    (round, block_id, signature_ids)
                })
            ).collect();


            let round_complete = current_round.commit_round().signature_ids();
            MsgBFTSyncRequest::BFTInitialSync {
                round, 
                block_tips,
                round_complete,
            }
        };
        send(stream, req).await.expect("TODO");

        todo!()
    }
}

pub struct BFTSyncResponder {
}

impl BFTSyncResponder {
    fn new() -> Self {
        Self {  }
    }

    async fn run_initial(&self) {
        continuehere
        // TODO: Record everything they have
        //
        // For each block_tips:
        //   If round is less than our round and we have the block, respond with all children of that block recursively (signatures should be aggregate for these blocks)
        //   
        // If their round matches our round, send everything they don't have from the current round.
        // If round is less than our round, respond with round_complete signatures for rounds greater than or equal to round (and less than the rounds that we fully responded with).
        todo!()
    }
}
