// This is very similar to DAG sync, except there are a few key differences:
// - The DAG is bipartite between block and certificate nodes
// - We have rounds, so we can take advantage of that (ex, only send previous round blocks if it has a full threshold signature?)
//
// Do we need to go back more than 1 round?
//
// Goal: Only send a node when they have all the parents of that node.

use std::{cmp::Reverse, collections::{BTreeSet, BinaryHeap}, fmt::Display, future::Future, iter::Peekable, marker::PhantomData, vec::IntoIter};

use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc::{UnboundedReceiver, UnboundedSender}, watch};
use tracing::{debug, error, warn};

use crate::{auth::DeviceId, network::protocol::{receive, send, MiniProtocol}, protocol::store_peer::dag_sync::{MAX_DELIVER_HEADERS, MAX_HAVE_HEADERS}, store::{bft::{BFTState, Block, BlockId, PartialSignature, Round, Signed, ThresholdSignature}, dag, UntypedStoreCommand}, util::{Sha256Hash, Stream}};

pub(crate) struct StoreBFTSync<Hash, StoreId, SHeaderId, SHeader, THeaderId, THeader> {
    peer: DeviceId,
    // Receive commands from store if we have initiative or send commands to store if we're the responder.
    recv_chan: Option<UnboundedReceiver<StoreBFTSyncCommand>>,
    // Send commands to store if we're the responder and send results back to store if we're the initiator.
    send_chan: UnboundedSender<UntypedStoreCommand<Hash, SHeaderId, SHeader, THeaderId, THeader>>, // JP: Make this a stream?
    // BFT state
    bft_state: watch::Receiver<BFTState<StoreId, SHeaderId>>,
}

impl<Hash, StoreId, SHeaderId, SHeader, THeaderId, THeader> StoreBFTSync<Hash, StoreId, SHeaderId, SHeader, THeaderId, THeader> {
    pub(crate) fn new_server(
        peer: DeviceId,
        recv_chan: UnboundedReceiver<StoreBFTSyncCommand>,
        send_chan: UnboundedSender<
            UntypedStoreCommand<Hash, SHeaderId, SHeader, THeaderId, THeader>,
        >,
        bft_state: watch::Receiver<BFTState<StoreId, SHeaderId>>,
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
        bft_state: watch::Receiver<BFTState<StoreId, SHeaderId>>,
    ) -> Self {
        StoreBFTSync {
            peer,
            recv_chan: None,
            send_chan,
            bft_state,
        }
    }
}

impl<Hash, StoreId, SHeaderId, SHeader, THeaderId, THeader> MiniProtocol for StoreBFTSync<Hash, StoreId, SHeaderId, SHeader, THeaderId, THeader>
where
    Hash: Send,
    StoreId: Clone + Send + Sync,
    SHeaderId: Clone + for<'a> Deserialize<'a> + Serialize + Send + Sync,
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
        mut self,
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

/// Identifier of either a partial or aggregate threshold signature.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Copy, Clone, Serialize, Deserialize)]
pub(crate) struct SignatureId(pub(crate) Sha256Hash);

impl Display for SignatureId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

// // Either the ID of the aggregate signature or IDs of the partial signatures.
// // JP: Or just send the signatures?
// #[derive(Debug, Serialize, Deserialize)]
// pub(crate) enum ThresholdSignatureId {
//     Threshold(SignatureId),
//     Partial(Vec<SignatureId>),
// }
// 
// impl ThresholdSignatureId {
//     pub fn new() -> Self {
//         ThresholdSignatureId::Partial(vec![])
//     }
// }

// For use in a Vec of block tips, ordered by (Round, BlockId). Block signatures ordered by SignatureId.
// TODO: Check that they are in order when parsed XXX
#[derive(Debug, Serialize, Deserialize)]
pub(crate) enum BlockStreamElement {
    Block(Round, BlockId),
    BlockSignature(SignatureId),
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
pub(crate) enum BFTSyncResponse<SHeaderId> {
    Block(Signed<Block<SHeaderId>>),
    CertificateSignature(BlockId, ThresholdSignature),
    CertificatePartialSignature(BlockId, DeviceId, PartialSignature),
    RoundCompleteSignature(Round, ThresholdSignature),
    RoundCompletePartialSignature(Round, DeviceId, PartialSignature),
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) enum MsgBFTSyncResponse<SHeaderId> {
    Response {
        response: Vec<BFTSyncResponse<SHeaderId>>,
        // blocks: Vec<Block<SHeaderId>>,
        // certificate_signatures: Vec<(BlockId, ThresholdSignature)>,
        // certificate_partial_signatures: Vec<(BlockId, PartialSignature)>,

        // round_complete_signatures: Vec<(Round, ThresholdSignature)>,
        // round_complete_partial_signatures: Vec<(Round, DeviceId, PartialSignature)>,

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
    // _phantom: PhantomData<fn(StoreId)>,
    todo: PhantomData<SHeaderId>, // JP: Is SHeaderId needed?
}

impl<SHeaderId> BFTSyncInitiator<SHeaderId> {
    /// Create a new ECGSyncInitiator and run the first round.
    async fn run_new<S: Stream<MsgStoreBFTSync<SHeaderId>>, StoreId>(stream: &mut S, bft_state: &mut watch::Receiver<BFTState<StoreId, SHeaderId>>) -> (Self, Vec<()>) {
        let req = {
            // Acquire read lock on state.
            let bft_state = bft_state.borrow_and_update();
            let round = bft_state.current_round();
            let current_round = &bft_state.round_states()[round as usize];

            // Get the current block and signature tips (sorted).
            let sorted_blocks = get_latest_block_and_signature_tips(&bft_state, None);

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
                            // Append signature
                            let elmt = BlockStreamElement::BlockSignature(*signature_id);
                            block_tips.push(elmt);
                            current_signature_pos = Some(j + 1);
                        } else {
                            // We're done with the signatures so go to the next block.
                            current_block_pos += 1;
                            current_signature_pos = None;
                        }
                    },
                    None => {
                        // Append block
                        let elmt = BlockStreamElement::Block(current_block.0, current_block.1);
                        block_tips.push(elmt);
                        current_signature_pos = Some(0);
                    }
                }
            }

            // Send round complete signatures.
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

/// Gets our latest blocks and signatures, sorted by (round, block_id).
/// If a round is provided, only blocks less than or equal to the given round will be returned.
fn get_latest_block_and_signature_tips<StoreId, SHeaderId>(bft_state: &watch::Ref<'_, BFTState<StoreId, SHeaderId>>, up_to_round: Option<Round>) -> Vec<(Round, BlockId, Vec<SignatureId>)> {
    // Get the current tips.
    let latest = bft_state.get_latest();

    // If upper bound on round is provided, filter rounds at this round or above.
    let latest = latest.iter().filter(|(round, _)| { up_to_round.is_none_or(|up_to_round| *round <= up_to_round) });

    // Sort by (Round, BlockId)
    let mut sorted_blocks = latest.map(|(round, block_id)| {
        let round_state = &bft_state.round_states()[*round as usize];

        // Return signatures on block, sorted.
        let signatures = round_state.certificates().get(&block_id).map_or_else(|| vec![], |ts| {
            let mut sig_ids = ts.signature_ids();
            sig_ids.sort();
            sig_ids
        });

        (*round, *block_id, signatures)
    }).collect::<Vec<_>>();
    sorted_blocks.sort_by_key(|(round, block_id, _)| (*round, *block_id));
    sorted_blocks
}


/// Get the blocks and signatures for this round (sorted by block id).
fn get_round_blocks_and_signatures<StoreId, SHeaderId>(bft_state: &watch::Ref<'_, BFTState<StoreId, SHeaderId>>, round: u64) -> Vec<(u64, BlockId, Vec<SignatureId>)> {
    let round_state = &bft_state.round_states()[round as usize];
    let mut blocks = round_state.blocks().values().map(|signed_block| {
        let block_id = signed_block.value().block_id();

        // Return signatures on block, sorted.
        let signatures = round_state.certificates().get(&block_id).map_or_else(|| vec![], |ts| {
            let mut sig_ids = ts.signature_ids();
            sig_ids.sort();
            sig_ids
        });

        (round, block_id, signatures)
    }).collect::<Vec<_>>();
    blocks.sort_by_key(|(round, block_id, _)| (*round, *block_id));
    blocks
}

#[derive(PartialEq, PartialOrd, Eq, Ord)]
pub enum BFTSyncResponseType {
    Block(BlockId),
    BlockSignature(BlockId, SignatureId),
    RoundSignature(SignatureId),
}

pub struct BFTSyncResponder {
    // their_round: Round, // JP: Remove these from here?
    // their_block_tips: Vec<BlockStreamElement>,
    // their_round_complete: Vec<RoundCompleteStreamElement>,
    their_known_blocks: BTreeSet<BlockId>,
    their_known_block_sigs: BTreeSet<(BlockId, SignatureId)>,
    their_known_round_sigs: BTreeSet<(Round, SignatureId)>,
    send_queue: BinaryHeap<Reverse<(Round, BFTSyncResponseType)>>,
}

impl BFTSyncResponder {

    async fn run_initial<S: Stream<MsgStoreBFTSync<SHeaderId>>, StoreId: Clone, SHeaderId: Clone>(
        stream: &mut S,
        bft_state: &mut watch::Receiver<BFTState<StoreId, SHeaderId>>,
        their_round: Round,
        their_block_tips: Vec<BlockStreamElement>,
        their_round_complete: Vec<RoundCompleteStreamElement>,
    ) -> Self {
        let mut new = Self {
            their_known_blocks: BTreeSet::new(),
            their_known_block_sigs: BTreeSet::new(),
            their_known_round_sigs: BTreeSet::new(),
            send_queue: BinaryHeap::new(),
        };

        let mut resp_e = {
            // Acquire read lock on state.
            let bft_state = bft_state.borrow_and_update();
            let round = bft_state.current_round();

            // If our round is behind theirs, tell them to wait.
            // JP: Maybe this isn't necessary? It's possible we might have state even if we're behind them? It probably is...
            if round < their_round {
                Err((their_block_tips, their_round_complete)) // JP: Send previous tips that they don't have?
            } else {
                new.build_response(bft_state, their_round, their_block_tips, their_round_complete)
                    .ok_or_else(|| (vec![BlockStreamElement::End], vec![RoundCompleteStreamElement::End]))
            }
        };

        let mut is_first_run = true;
        loop {
            match resp_e {
                Ok(resp) => {
                    send(stream, resp).await.expect("TODO");
                    return new;
                }
                Err((their_block_tips, their_round_complete)) => {
                    if is_first_run {
                        is_first_run = false;
                        send(stream, MsgBFTSyncResponse::Wait::<SHeaderId>).await.expect("TODO");
                    }

                    // Acquire read lock on state once we've caught up.
                    let bft_state = bft_state.wait_for(|s| s.current_round() >= their_round).await.expect("TODO: channel closed");

                    resp_e = new.build_response(bft_state, their_round, their_block_tips, their_round_complete)
                        .ok_or_else(|| (vec![BlockStreamElement::End], vec![RoundCompleteStreamElement::End]));
                }
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

        // new
    }

    /// Processes their request. Returns true if they've sent everything they have currently.
    /// Precondition: our_round >= their_round
    fn handle_their_latest<StoreId, SHeaderId>(
        &mut self,
        bft_state: &watch::Ref<'_, BFTState<StoreId, SHeaderId>>,
        mut their_round: Round,
        their_block_tips: Vec<BlockStreamElement>,
        their_round_complete: Vec<RoundCompleteStreamElement>,
    ) -> bool {
        // Get all previous tips (sorted) less than or equal to their current round?
        let our_previous_tips = get_latest_block_and_signature_tips(bft_state, Some(their_round));

        // Process their tips with our previous tips.
        let are_blocks_done = self.process_their_latest(their_block_tips, our_previous_tips);

        // Process their round complete signatures.
        let round_complete_signatures = get_round_complete_signatures(bft_state, their_round);
        let our_round = bft_state.current_round();
        // Optimization: If their_round < our_round, we can just queue our aggregate round signature.
        let is_complete_done = if their_round < our_round {
            self.queue_round_completes(their_round, &round_complete_signatures);
            true
        } else {
            self.process_their_round_completes(their_round, their_round_complete, round_complete_signatures)
        };

        // They don't know everything for the current round so we can't move onto next round.
        if !are_blocks_done || !is_complete_done {
            return false;
        }

        // Move onto next round.
        their_round += 1;

        // Keep processing rounds until they've caught up (or we've filled the buffer).
        while their_round <= our_round && !self.are_buffers_full() {
            let round_blocks = get_round_blocks_and_signatures(bft_state, their_round);
            self.queue_blocks(&mut round_blocks.into_iter().peekable(), None);

            let round_complete_signatures = get_round_complete_signatures(bft_state, their_round);
            self.queue_round_completes(their_round, &round_complete_signatures);

            their_round += 1;
        }

        true
    }

    /// Returns None if we don't have anything to share so they should wait.
    /// Precondition: our_round >= their_round
    fn build_response<StoreId, SHeaderId: Clone>(
        &mut self,
        bft_state: watch::Ref<'_, BFTState<StoreId, SHeaderId>>,
        their_round: Round,
        their_block_tips: Vec<BlockStreamElement>,
        their_round_complete: Vec<RoundCompleteStreamElement>,
    ) -> Option<MsgBFTSyncResponse<SHeaderId>> {
        let done = self.handle_their_latest(&bft_state, their_round, their_block_tips, their_round_complete);

        let response = self.prepare_response(&bft_state);
        if response.is_empty() {
            if done {
                // If they've sent us everything, we don't have anything to share so we'll tell them to wait.
                None
            } else {
                // Otherwise, send them back an empty response so they can send what else they have.
                Some(MsgBFTSyncResponse::Response {
                    response: vec![],
                })
            }
        } else {
            Some(MsgBFTSyncResponse::Response {
                response,
            })
        }
    }

    // fn bump_their_round(&mut self, latest_round: Round) {
    //     self.their_round = latest_round;
    // }

    // Process the block and signature tips they sent. Returns true if we're done and they don't
    // have any more to send.
    fn process_their_latest(&mut self, their_block_tips: Vec<BlockStreamElement>, blocks_and_sigs: Vec<(Round, BlockId, Vec<SignatureId>)>) -> bool {
        let mut their_block_tips = their_block_tips.into_iter();
        let mut blocks_and_sigs = blocks_and_sigs.into_iter().peekable();

        let Some(their_element) = their_block_tips.next() else {
            error!("TODO: Peer deviated from protocol. They sent an empty stream.");
            todo!("TODO: Peer deviated from protocol. Gracefully handle this");
        };

        warn!("TODO: For subsequent rounds, only queue if they don't already know it? Or check this when sending?");

        let mut their_current_block = match their_element {
            BlockStreamElement::Block(round, block_id) => (round, block_id),
            BlockStreamElement::BlockSignature(sha256_hash) => {
                error!("TODO: Peer deviated from protocol. Started with block signature id: {sha256_hash}");
                todo!("TODO: Peer deviated from protocol. Gracefully handle this");
            }
            BlockStreamElement::End => {
                // Send everything from blocks_and_sigs.
                self.queue_blocks(&mut blocks_and_sigs, None);

                return true;
            }
        };
        self.mark_block_as_known(their_current_block.1);

        // Queue everything we have that's before their_current_block.
        self.queue_blocks(&mut blocks_and_sigs, Some(their_current_block));

        let mut blocks_and_sigs = StreamableBlocks::new(blocks_and_sigs);
        while let Some(their_current_element) = their_block_tips.next() {
            let their_round = their_current_block.0;
            let their_block_id = their_current_block.1;

            match their_current_element {
                BlockStreamElement::BlockSignature(sig_id) => {
                    self.mark_block_sig_as_known(their_block_id, sig_id);

                    // Queue sigs for blocks less than the current signature.
                    self.queue_block_sigs(&mut blocks_and_sigs, their_round, their_block_id, Some(sig_id));
                }
                BlockStreamElement::Block(round, block_id) => {
                    self.mark_block_as_known(block_id);
                    their_current_block = (round, block_id);

                    // Queue remaining sigs for the current block.
                    self.queue_block_sigs(&mut blocks_and_sigs, their_round, their_block_id, None);

                    // Queue everything we have that's before their new block.
                    self.queue_blocks(blocks_and_sigs.stream(), Some((round, block_id)));
                }
                BlockStreamElement::End => {
                    // Queue remaining sigs for the current block.
                    self.queue_block_sigs(&mut blocks_and_sigs, their_round, their_block_id, None);

                    // Queue everything we have that's before their new block.
                    self.queue_blocks(blocks_and_sigs.stream(), None);

                    return true;
                }
            }
        }

        false
    }

    // Mark block as known by them.
    fn mark_block_as_known(&mut self, block_id: BlockId) {
        self.their_known_blocks.insert(block_id);
    }

    // Mark block signature as known by them.
    fn mark_block_sig_as_known(&mut self, block_id: BlockId, sig_id: SignatureId) {
        self.their_known_block_sigs.insert((block_id, sig_id));
    }

    // Mark round signature as known by them.
    fn mark_round_sig_as_known(&mut self, round: Round, their_sig_id: SignatureId) {
        self.their_known_round_sigs.insert((round, their_sig_id));
    }

    // Queue sigs for blocks less than or equal to their current block.
    // If an upper limit sig is provided, only queue up to that limit.
    // Otherwise, queue all the signatures that're remaining.
    fn queue_block_sigs(&mut self, blocks_and_sigs: &mut StreamableBlocks, their_round: Round, their_block_id: BlockId, upper_sig_id: Option<SignatureId>) {
        let Some(ref our_current_element) = blocks_and_sigs.current_element else {
            // We don't have any more to share.
            return;
        };
        let our_round = our_current_element.0;
        let our_block_id = our_current_element.1;

        if (our_round, our_block_id) < (their_round, their_block_id) {
            warn!("Invariant violated. It's possible they're misbehaving.");
            todo!("Invariant violated. It's possible they're misbehaving. Gracefully handle this."); // TODO: Temp. DELETEME
        }

        // If we don't have their current block, we don't have anything to share.
        if (our_round, our_block_id) > (their_round, their_block_id) {
            return;
        }

        // Send any signatures we have that they don't have for their current block.
        while let Some(sig_id) = our_current_element.2.get(blocks_and_sigs.current_sig_pos) {
            let done = match upper_sig_id {
                Some(upper_sig_id) => {
                    if *sig_id <= upper_sig_id {
                        false
                    } else {
                        true
                    }
                }
                None => {
                    false
                }
            };

            if done {
                return;
            } else {
                self.queue_block_sig(their_round, their_block_id, *sig_id);
                blocks_and_sigs.current_sig_pos += 1;
            }
        }

        // We're done with our current block so increment current element.
        blocks_and_sigs.current_element = blocks_and_sigs.stream.next();
        blocks_and_sigs.current_sig_pos = 0;
    }

    // // Queue blocks and their sigs less than the given block ID.
    // fn queue_block_sigs(&mut self, blocks_and_sigs: &mut Peekable<IntoIter<(Round, BlockId, Vec<Sha256Hash>)>>, block_id: (Round, BlockId)) {
    //     while let Some(sig_id) = blocks_and_sigs.next_if(|sig_id|)
    // }

    // // Queue sigs from sigs_m that are less than sig_id (if provided). Otherwise, queue them all.
    // fn queue_block_sigs(&mut self, block_id: BlockId, sigs_m: &mut Option<Peekable<IntoIter<Sha256Hash>>>, upper_sig_id: Option<Sha256Hash>) {
    //     if let Some(sigs) = sigs_m {
    //         while let Some(sig_id) = sigs.next_if( |sig_id|
    //             upper_sig_id.map_or(true, |upper_sig_id| *sig_id <= upper_sig_id)
    //         ) {
    //             // Skip our next sig if it equals their sig.
    //             if Some(sig_id) != upper_sig_id {
    //                 self.their_known_block_sigs.insert((block_id, sig_id));
    //             }
    //         }

    //         // TODO: DELETEME, moved to `if` above
    //         // // Drop our next sig if it equals their sig.
    //         // let _ = sigs.next_if(|sig_id| Some(*sig_id) == upper_sig_id);
    //     }
    // }

    /// Queue a block and its signatures.
    fn queue_block_and_sigs(&mut self, (our_round, our_block_id, our_sigs): (u64, BlockId, Vec<SignatureId>)) {
            let block = BFTSyncResponseType::Block(our_block_id);
            if !self.they_know(our_round, &block) {
                self.send_queue.push(Reverse((our_round, block)));
            }

            // Send all of the corresponding signatures (that they don't know).
            let sigs = our_sigs.into_iter().map(|sig_id| Reverse((our_round, BFTSyncResponseType::BlockSignature(our_block_id, sig_id)))).filter(|s| !self.they_know(s.0.0, &s.0.1)).collect::<Vec<_>>();
            self.send_queue.extend(sigs);
    }

    // Queue blocks and their sigs less than the given block ID (if provided). Otherwise sends them all.
    fn queue_blocks(&mut self, blocks_and_sigs: &mut Peekable<IntoIter<(u64, BlockId, Vec<SignatureId>)>>, upper_block_m: Option<(Round, BlockId)>) {
        while let Some(block_and_sigs) = blocks_and_sigs.next_if( |our_block|
            upper_block_m.is_none_or(|upper_block| (our_block.0, our_block.1) < upper_block)
        ) {
            self.queue_block_and_sigs(block_and_sigs);
        }
    }

    /// Checks whether any of our queue buffers are full.
    fn are_buffers_full(&self) -> bool {
        self.send_queue.len() >= MAX_DELIVER_HEADERS.into()
    }

    /// Returns true if we've queued everything to complete the round.
    fn process_their_round_completes(&mut self, round: u64, their_round_complete: Vec<RoundCompleteStreamElement>, our_round_complete_signatures: Vec<SignatureId>) -> bool {
        let mut their_round_complete = their_round_complete.into_iter();
        let mut our_round_complete_signatures = our_round_complete_signatures.into_iter().peekable();
        while let Some(their_element) = their_round_complete.next() {
            match their_element {
                RoundCompleteStreamElement::RoundSignatures(their_sig_id) => {
                    self.mark_round_sig_as_known(round, their_sig_id);
                    while let Some(our_sig_id) = our_round_complete_signatures.next_if(|our_sig_id| *our_sig_id <= their_sig_id) {
                        // Queue if they don't have our sig.
                        if our_sig_id < their_sig_id {
                            self.queue_round_completes(round, &[our_sig_id]);
                        }
                    }
                }
                RoundCompleteStreamElement::End => {
                    // Queue everything else.
                    self.queue_round_completes(round, &our_round_complete_signatures.collect::<Vec<_>>());
                    return true;
                }
            }
        }

        false
    }

    fn queue_round_completes(&mut self, round: u64, our_sig_ids: &[SignatureId]) {
        let sigs = our_sig_ids.iter().map(|sig_id| Reverse((round, BFTSyncResponseType::RoundSignature(*sig_id)))).filter(|s| !self.they_know(s.0.0, &s.0.1)).collect::<Vec<_>>();
        self.send_queue.extend(sigs);
    }

    fn prepare_response<StoreId, SHeaderId: Clone>(
        &mut self,
        bft_state: &watch::Ref<'_, BFTState<StoreId, SHeaderId>>,
    ) -> Vec<BFTSyncResponse<SHeaderId>> {
        let mut operations = Vec::with_capacity(MAX_DELIVER_HEADERS as usize);

        while let Some(Reverse((round, response))) = self.send_queue.pop() {
            // Skip if they already know this header.
            let skip = self.they_know(round, &response);

            if !skip {
                let response = match response {
                    BFTSyncResponseType::Block(block_id) => {
                        let block = bft_state.get_block(round, &block_id).expect("Block not found even though we added it");
                        BFTSyncResponse::Block(block.clone()) // TODO: Can we get rid of this clone?
                    }
                    BFTSyncResponseType::BlockSignature(block_id, sig_id) => {
                        let sig = bft_state.get_block_signature(round, block_id, sig_id).expect("Block signature not found even though we added it.");
                        match sig {
                            Ok(aggregate) => BFTSyncResponse::CertificateSignature(block_id, aggregate),
                            Err((device_id, partial)) => BFTSyncResponse::CertificatePartialSignature(block_id, device_id, partial),
                        }
                    }
                    BFTSyncResponseType::RoundSignature(sig_id) => {
                        let sig = bft_state.get_round_signature(round, sig_id).expect("Round signature not found even though we added it.");
                        match sig {
                            Ok(aggregate) => BFTSyncResponse::RoundCompleteSignature(round, aggregate),
                            Err((device_id, partial)) => BFTSyncResponse::RoundCompletePartialSignature(round, device_id, partial),
                        }
                    }
                };

                operations.push(response);
            }

            if operations.len() >= MAX_DELIVER_HEADERS.into() {
                break;
            }
        }

        operations
    }

    fn they_know(&self, round: Round, response: &BFTSyncResponseType) -> bool {
        match response {
            BFTSyncResponseType::Block(block_id) => self.their_known_blocks.contains(block_id),
            BFTSyncResponseType::BlockSignature(block_id, sig_id) => self.their_known_block_sigs.contains(&(*block_id, *sig_id)),
            BFTSyncResponseType::RoundSignature(sig_id) => self.their_known_round_sigs.contains(&(round, *sig_id)),
        }
    }

    fn queue_block_sig(&mut self, their_round: Round, their_block_id: BlockId, sig_id: SignatureId) {
        let sig = BFTSyncResponseType::BlockSignature(their_block_id, sig_id);
        if !self.they_know(their_round, &sig) {
            self.send_queue.push(Reverse((their_round, sig)));
        }
    }
}

/// Retrieve round complete signatures for this round.
fn get_round_complete_signatures<StoreId, SHeaderId>(bft_state: &watch::Ref<'_, BFTState<StoreId, SHeaderId>>, round: u64) -> Vec<SignatureId> {
    let round_state = &bft_state.round_states()[round as usize];
    let mut sig_ids = round_state.commit_round().signature_ids();
    sig_ids.sort();
    sig_ids
}

struct StreamableBlocks {
    stream: Peekable<IntoIter<(u64, BlockId, Vec<SignatureId>)>>,
    current_element: Option<(u64, BlockId, Vec<SignatureId>)>,
    current_sig_pos: usize,
}

impl StreamableBlocks {
    fn new(mut stream: Peekable<IntoIter<(u64, BlockId, Vec<SignatureId>)>>) -> Self {
        let current_element = stream.next();
        StreamableBlocks {
            stream,
            current_element,
            current_sig_pos: 0,
        }
    }

    fn stream(&mut self) -> &mut Peekable<IntoIter<(u64, BlockId, Vec<SignatureId>)>> {
        if let Some(current_element) = &self.current_element {
            assert!(self.current_sig_pos >= current_element.2.len(), "Invariant violated: Cannot mutate stream while currently processing signature stream.")
        }

        &mut self.stream
    }
}

