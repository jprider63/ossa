// This is very similar to DAG sync, except there are a few key differences:
// - The DAG is bipartite between block and certificate nodes
// - We have rounds, so we can take advantage of that (ex, only send previous round blocks if it has a full threshold signature?)
//
// Do we need to go back more than 1 round?

use std::{cmp::Reverse, collections::{BTreeSet, BinaryHeap}, future::Future, iter::Peekable, marker::PhantomData, vec::IntoIter};

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
        mut self,
        mut stream: S,
    ) -> impl Future<Output = ()> + Send {
        async move {
            debug!("StoreBFTSync client running!");
            let mut bft_sync: Option<BFTSyncResponder<SHeaderId>> = None;

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
    BlockSignature(SignatureId),
    SignaturesEnd,
    BlocksEnd,
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
        let mut sent_blocks = BTreeSet::new();
        let mut sent_signatures = BTreeSet::new();
        let mut sent_round_signatures = BTreeSet::new();

        let req = {
            // Acquire read lock on state.
            let bft_state = bft_state.borrow_and_update();
            let round = bft_state.current_round();

            // Get the current block tips.
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

            // Iterate over them, up to the limit. Note, we save one slot in case we need to send End.
            while block_tips.len() + 1 < MAX_HAVE_HEADERS.into() {
                let Some(current_block) = sorted_blocks.get(current_block_pos) else {
                    // We're done so exit the loop.
                    break;
                };

                match current_signature_pos {
                    Some(j) => {
                        // Have more signatures to send.
                        if let Some(signature_id) = current_block.2.get(j) {
                            // Append signature
                            let elmt = BlockStreamElement::BlockSignature(*signature_id);
                            block_tips.push(elmt);
                            current_signature_pos = Some(j + 1);

                            // Record signature as sent.
                            sent_signatures.insert((current_block.1, signature_id));
                        // No more signatures to send.
                        } else {
                            // Record block as sent if it currently has no signatures.
                            if j == 0 {
                                sent_blocks.insert(current_block.1);
                            }

                            // We're done with the signatures so go to the next block.
                            current_block_pos += 1;
                            current_signature_pos = None;
                        }
                    },
                    None => {
                        // Append block
                        let elmt = BlockStreamElement::Block(*current_block.0, current_block.1);
                        block_tips.push(elmt);
                        current_signature_pos = Some(0);
                    }
                }
            }
            // If we're done, let them know we're done with this block.
            let is_done = {
                if let Some(current_block) = sorted_blocks.get(current_block_pos) {
                    let is_done = if let Some(j) = current_signature_pos {
                        current_block.2.get(j).is_none()
                    } else {
                        true
                    };

                    if is_done { Some(BlockStreamElement::SignaturesEnd) } else { None }
                } else {
                    // We're done with all blocks for this round.
                    Some(BlockStreamElement::BlocksEnd)
                }
            };
            if let Some(end) = is_done {
                block_tips.push(end);
            }

            // Send round complete signatures.
            let current_round = &bft_state.round_states()[round as usize];
            let mut round_complete = current_round.commit_round().signature_ids().into_iter().map(RoundCompleteStreamElement::RoundSignatures).collect::<Vec<_>>();
            if round_complete.len() + 1 < MAX_HAVE_HEADERS.into() {
                round_complete.push(RoundCompleteStreamElement::End);
            } else {
                round_complete.truncate(MAX_HAVE_HEADERS as usize - 1);
            }

            // Store sent signatures.
            round_complete.iter().for_each(|sig| {
                if let RoundCompleteStreamElement::RoundSignatures(sig) = sig {
                    sent_round_signatures.insert(sig);
                }
            });

            MsgBFTSyncRequest::BFTInitialSync {
                round, 
                block_tips,
                round_complete,
            }
        };
        send(stream, req).await.expect("TODO");

        // TODO: Store remaining_round_complete and other state.

        todo!()
    }
}

pub struct BFTSyncResponder<SHeaderId> {
    send_queue: BinaryHeap<(Reverse<Round>, BlockId)>,
    our_unknown: BTreeSet<BlockId>,
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
            _phantom: PhantomData,
            send_queue: BinaryHeap::new(),
            our_unknown: BTreeSet::new(),
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
                let resp = new.build_response(bft_state, their_round, &their_block_tips, &their_round_complete);
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

                let resp = new.build_response(bft_state, their_round, &their_block_tips, &their_round_complete);
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
        their_round: Round,
        their_block_tips: &[BlockStreamElement],
        their_round_complete: &[RoundCompleteStreamElement],
    ) -> MsgBFTSyncResponse<SHeaderId> {
        self.handle_their_tips(&bft_state, their_round, their_block_tips, their_round_complete);

        self.build_response_helper(store_peer, bft_state, their_round, their_block_tips, their_round_complete)
        // Send all previous tips (sorted) less that the current round (that they don't have)?
        // Get and sort our tips. Along with everything starting from their_round???
        // Iterate over their tips
        // JP: With this approach, we'll miss nodes where we know a child that depends on it but they dont?
        //  Or they know a child that depends on it but we do? But in this case, it's ok, we'll just send it even though they don't need it.


        // Alternative:
        //
        // Mark th

    }

    fn handle_their_tips(&mut self, bft_state: &watch::Ref<'_, BFTState<SHeaderId>>, their_round: Round, their_block_tips: &[BlockStreamElement], their_round_complete: &[RoundCompleteStreamElement]) {
        // Handle blocks and signatures in their tips.
        let mut current_block = None;
        their_block_tips.iter().for_each(|elmt| {
            match elmt {
                BlockStreamElement::Block(round, block_id) => {
                    self.process_remaining_signatures(&mut current_block);

                    let b = (*round, *block_id);

                    // Check if we know the block.
                    if bft_state.contains(&b) {
                        // Mark block and its ancestors as known.
                        self.mark_as_known(bft_state, &b);

                        // Add children to send queue.
                        self.send_children_blocks(bft_state, &b);

                        // Remember block signatures.
                        let block_signatures = self.block_signatures_iter(bft_state, &b).peekable();
                        current_block = Some((b, block_signatures));
                    } else {
                        // Record block as known by them but not us.
                        self.our_unknown.insert(b.1);

                        // Remember block signatures.
                        let block_signatures = vec![].into_iter().peekable();
                        current_block = Some((b, block_signatures));
                    }
                }
                BlockStreamElement::BlockSignature(signature_id) => {
                    let Some((b, ref mut block_signatures)) = current_block else {
                        todo!("Peer diverged from protocol. Must receive block before signatures on blocks.");
                    };

                    // Mark signature as known.
                    // JP: Only do this if we know the signature, otherwise add to our_unknown??
                    self.mark_signature_as_known(bft_state, &b, *signature_id);

                    loop {
                        // We receive signature IDs in order, so we know whether or not they have this signature.
                        let Some(next) = block_signatures.peek() else {
                            // We don't have any more signatures for this block so we're done.
                            break;
                        };
                        if next <= signature_id {
                            // Pop this next signature.
                            let next = block_signatures.next().unwrap();

                            if next < *signature_id {
                                self.queue_signature(bft_state, &b, next);
                            }
                        } else {
                            // Our next block exceeds their current block so move onto their next block.
                            break;
                        }
                    }
                }
                BlockStreamElement::SignaturesEnd => {
                    // That's all the signatures they know so send any remaining signatures for the current block.
                    self.process_remaining_signatures(&mut current_block);
                }
                BlockStreamElement::BlocksEnd => {
                    // That's all the blocks they know so send all remaining blocks for their round and our tips less than this round.
                    let rounds = bft_state.round_states();
                    let our_round = rounds.get(their_round as usize).expect("Invariant: We must have their round");
                    our_round.blocks().values().for_each(|signed_block| {
                        let block_id = signed_block.value().block_id();
                        self.queue_block(bft_state, their_round, block_id);
                    });

                    bft_state.previous_tips().iter().for_each(|(round, device_id)| {
                        // Only queue tips from older rounds.
                        let round = *round;
                        if round < their_round {
                            let round_st = rounds.get(round as usize).unwrap();
                            let signed_block = round_st.blocks().get(device_id).unwrap();
                            let block_id = signed_block.value().block_id();
                            self.queue_block(bft_state, round, block_id);
                        }
                    });
                }
            }
        });

        // Handle stream of their current round's signatures.
        let mut round_signatures = self.round_complete_signatures_iter(bft_state, their_round);
        their_round_complete.iter().for_each(|round_sig| {
            match round_sig {
                RoundCompleteStreamElement::RoundSignatures(signature_id) => {
                    // Mark round signature as known by them.
                    // JP: Do we need this?
                    self.mark_round_signature_as_known(bft_state, their_round, round_sig);

                    loop {
                        // We receive signature IDs in order, so we know whether or not they have this signature.
                        let Some(next) = round_signatures.peek() else {
                            // We don't have any more signatures for this round so we're done.
                            break;
                        };

                        if next <= signature_id {
                            let next = round_signatures.next().unwrap();
                            if next < *signature_id {
                                self.queue_round_signature(bft_state, their_round, next);
                            }
                        } else {
                            // Our next signature exceeds theirs so we move on to their next signature.
                            break;
                        }
                    }
                }
                RoundCompleteStreamElement::End => {
                    while let Some(sig) = round_signatures.next() {
                        self.queue_round_signature(bft_state, their_round, sig);
                    }
                }
            }
        });
    }

    fn mark_as_known(&self, bft_state: &watch::Ref<'_, BFTState<SHeaderId>>, b: &(u64, BlockId)) {
        todo!()
    }

    fn send_children_blocks(&self, bft_state: &watch::Ref<'_, BFTState<SHeaderId>>, b: &(u64, BlockId)) {
        todo!()
    }

    // Iterator over a block's signatures, ordered by signature ID.
    fn block_signatures_iter(&self, bft_state: &watch::Ref<'_, BFTState<SHeaderId>>, b: &(u64, BlockId)) -> std::vec::IntoIter<SignatureId> { // impl Iterator<Item = SignatureId> {
        todo!();
        vec![].into_iter()
    }

    fn mark_signature_as_known(&self, bft_state: &watch::Ref<'_, BFTState<SHeaderId>>, b: &(u64, BlockId), signature_id: Sha256Hash) {
        todo!()
    }

    fn queue_signature(&self, bft_state: &watch::Ref<'_, BFTState<SHeaderId>>, b: &(u64, BlockId), signature_id: Sha256Hash) {
        // TODO:
        // Mark as known once it is sent?
        // self.mark_signature_as_known(bft_state, &b, signature_id);

        todo!()
    }

    fn process_remaining_signatures(&self, current_block: &mut Option<((u64, BlockId), Peekable<IntoIter<Sha256Hash>>)>) {
        todo!()
    }

    fn queue_block(&self, bft_state: &watch::Ref<'_, BFTState<SHeaderId>>, round: Round, block_id: BlockId) {
        todo!()
    }

    fn round_complete_signatures_iter(&self, bft_state: &watch::Ref<'_, BFTState<SHeaderId>>, their_round: u64) -> Peekable<IntoIter<SignatureId>> {
        // let round_st = bft_state.round_states().get(their_round as usize).expect("Invariant: We must have their round");
        todo!()
    }

    fn mark_round_signature_as_known(&self, bft_state: &watch::Ref<'_, BFTState<SHeaderId>>, their_round: Round, round_sig: &RoundCompleteStreamElement) {
        todo!()
    }

    fn queue_round_signature(&self, bft_state: &watch::Ref<'_, BFTState<SHeaderId>>, their_round: Round, next: SignatureId) {
        todo!()
    }
}
