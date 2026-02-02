// This is very similar to DAG sync, except there are a few key differences:
// - The DAG is bipartite between block and certificate nodes
// - We have rounds, so we can take advantage of that (ex, only send previous round blocks if it has a full threshold signature?)
//
// Do we need to go back more than 1 round?

use std::{future::Future, marker::PhantomData};

use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc::{UnboundedReceiver, UnboundedSender}, watch};
use tracing::{debug, warn};

use crate::{auth::DeviceId, network::protocol::{receive, send, MiniProtocol}, protocol::store_peer::dag_sync::MAX_HAVE_HEADERS, store::{bft::{BFTState, Block, BlockId, PartialSignature, Round, ThresholdSignature}, dag, UntypedStoreCommand}, util::{Sha256Hash, Stream}};

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

                        let bft_sync_ = BFTSyncResponder::run_initial(
                            &mut stream,
                            &mut self.bft_state,
                            round,
                            block_tips,
                            round_complete,
                        ).await;
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

// For use in a Vec of block tips, ordered by (Round, BlockId). Block signatures ordered by SignatureId.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) enum BlockStreamElement {
    Block(Round, BlockId),
    BlockSignatures(SignatureId),
    End,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) enum RoundCompleteStreamElement {
    RoundSignatures(SignatureId),
    End,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) enum MsgBFTSyncRequest {
    BFTInitialSync {
        /// Current round we're on.
        round: Round,

        /// Tips/frontier of blocks with their round and corresponding signatures.
        block_tips: Vec<BlockStreamElement>,

        /// Signatures attesting to completion of the current round, in order by SignatureId.
        // JP: Or just send the signatures?
        round_complete: Vec<RoundCompleteStreamElement>,
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

impl<SHeaderId> From<MsgBFTSyncResponse<SHeaderId>> for MsgStoreBFTSync<SHeaderId> {
    fn from(msg: MsgBFTSyncResponse<SHeaderId>) -> Self {
        MsgStoreBFTSync::BFTResponse(msg)
    }
}

impl<SHeaderId> TryInto<MsgBFTSyncResponse<SHeaderId>> for MsgStoreBFTSync<SHeaderId> {
    type Error = ();

    fn try_into(self) -> Result<MsgBFTSyncResponse<SHeaderId>, Self::Error> {
        match self {
            MsgStoreBFTSync::Request(_msg_bftsync_request) => Err(()),
            MsgStoreBFTSync::BFTResponse(msg_bftsync_response) => Ok(msg_bftsync_response),
        }
    }
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
            // Acquire read lock on state.
            let bft_state = bft_state.borrow_and_update();
            let round = bft_state.current_round();

            // Get the current tips.
            let current_tips = bft_state.get_current_tips();

            // Sort by (Round, BlockId)
            let mut sorted_blocks = current_tips.iter().map(|(round, peer_id)| {
                let round_state = &bft_state.round_states()[*round as usize];
                let signed_block = round_state.blocks().get(peer_id).expect("Block not found even though it is a tip");
                let block = signed_block.value();
                let block_id = block.block_id();

                // Return signatures on block, sorted.
                let signatures = round_state.certificates().get(&block_id).map_or_else(|| vec![], |ts| {
                    let mut sig_ids = ts.signature_ids();
                    sig_ids.sort();
                    sig_ids
                });

                (round, block_id, signatures)
            }).collect::<Vec<_>>();
            sorted_blocks.sort_by_key(|(round, peer_id, _)| (*round, *peer_id));

            let mut block_tips = Vec::with_capacity(MAX_HAVE_HEADERS as usize);
            let mut current_block_pos = 0;
            let mut current_signature_pos = None;

            // Iterate over them, up to the limit:
            while block_tips.len() < MAX_HAVE_HEADERS.into() {
                let Some(current_block) = sorted_blocks.get(current_block_pos) else {
                    // We're done so end the stream and exit the loop.
                    block_tips.push(BlockStreamElement::End);
                    break;
                };

                match current_signature_pos {
                    Some(j) => {
                        if let Some(signature_id) = current_block.2.get(j) {
                            //  Append signature
                            let elmt = BlockStreamElement::BlockSignatures(*signature_id);
                            block_tips.push(elmt);
                            current_signature_pos = Some(j + 1);
                        } else {
                            // We're done with the signatures so go to the next block.
                            current_block_pos += 1;
                            current_signature_pos = None;
                        }
                    },
                    None => {
                        //  Append block
                        let elmt = BlockStreamElement::Block(*current_block.0, current_block.1);
                        block_tips.push(elmt);
                        current_signature_pos = Some(0);
                    }
                }
            }

            // Send round complete signatures.
            let current_round = &bft_state.round_states()[round as usize];
            let mut round_complete = current_round.commit_round().signature_ids().into_iter().map(RoundCompleteStreamElement::RoundSignatures).collect::<Vec<_>>();
            round_complete.push(RoundCompleteStreamElement::End);
            let remaining_round_complete = round_complete.split_off(MAX_HAVE_HEADERS.into());

            // TODO: Store remaining_round_complete and other state.

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

pub struct BFTSyncResponder<SHeaderId> {
    their_round: Round,
    their_block_tips: Vec<BlockStreamElement>,
    their_round_complete: Vec<RoundCompleteStreamElement>,
    _phantom: PhantomData<SHeaderId>, // JP: Is SHeaderId needed?
}

impl<SHeaderId> BFTSyncResponder<SHeaderId> {

    async fn run_initial<S: Stream<MsgStoreBFTSync<SHeaderId>>>(
        stream: &mut S,
        bft_state: &mut watch::Receiver<BFTState<SHeaderId>>,
        their_round: Round,
        their_block_tips: Vec<BlockStreamElement>,
        their_round_complete: Vec<RoundCompleteStreamElement>,
    ) -> Self {
        let new = Self {
            their_round,
            their_block_tips,
            their_round_complete,
            _phantom: PhantomData,
        };

        // // TODO: Record everything they have
        // //
        // // For each block_tips:
        // //   If round is less than our round and we have the block, respond with all children of that block recursively (signatures should be aggregate for these blocks)
        // //   
        // // If their round matches our round, send everything they don't have from the current round.
        // // If round is less than our round, respond with round_complete signatures for rounds greater than or equal to round (and less than the rounds that we fully responded with).

        let resp_m = {
            // Acquire read lock on state.
            let bft_state = bft_state.borrow_and_update();
            let round = bft_state.current_round();

            // If our round is behind theirs, tell them to wait.
            if round < their_round {
                None // JP: Send previous tips that they don't have?
            } else {
                let resp = new.build_response(bft_state);
                Some(resp)
            }
        };

        match resp_m {
            Some(resp) => {
                send(stream, resp).await.expect("TODO");
            }
            None => {
                send(stream, MsgBFTSyncResponse::Wait).await.expect("TODO");

                // Acquire read lock on state once we've caught up.
                let bft_state = bft_state.wait_for(|s| s.current_round() >= their_round).await.expect("TODO: channel closed");

                let resp = new.build_response(bft_state);
                send(stream, resp).await.expect("TODO");
            }
        }

        // TODO: Send tips in order (Round, BlockId)? Upon receipt of a tip, if we see that a tip was skipped, respond with the skipped tips. Upon receiving END, queue everything after the last tip
        //
        //
        // Upon receipt of their tip, cases:
        //     - we have tip
        //         - Check for skipped tips and send those. (or if we have an aggregate signature and they don't, send the aggregate signature
        //         - Start queue of children to send
        //     - we don't have tip
        //         - Either the tip is:
        //             - a descendent of what we know (round is later than our current round?)
        //             - in a fork/sibling of our view (round is <= our current round)

        new
    }

    // Precondition: our_round >= their_round
    fn build_response(
        &self,
        bft_state: watch::Ref<'_, BFTState<SHeaderId>>,
    ) -> MsgBFTSyncResponse<SHeaderId> {
         // Send all previous tips (sorted) less that the current round (that they don't have)?
        // Get and sort our tips. Along with everything starting from their_round???
        // Iterate over their tips
        // JP: With this approach, we'll miss nodes where we know a child that depends on it but they dont?
        //  Or they know a child that depends on it but we do? But in this case, it's ok, we'll just send it even though they don't need it.


        // Alternative:
        //
        // Mark th

        todo!()
    }
}
