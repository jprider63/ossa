// This is very similar to DAG sync, except there are a few key differences:
// - The DAG is bipartite between block and certificate nodes
// - We have rounds, so we can take advantage of that (ex, only send previous round blocks if it has a full threshold signature?)
//
// Do we need to go back more than 1 round?
//
// Goal: Only send a node when they have all the parents of that node.

use std::{cmp::Reverse, collections::{BTreeSet, BinaryHeap}, future::Future, iter::Peekable, marker::PhantomData, vec::IntoIter};

use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc::{UnboundedReceiver, UnboundedSender}, watch};
use tracing::{debug, error, warn};

use crate::{auth::DeviceId, network::protocol::{receive, send, MiniProtocol}, protocol::store_peer::dag_sync::{MAX_DELIVER_HEADERS, MAX_HAVE_HEADERS}, store::{bft::{BFTState, Block, BlockId, PartialSignature, Round, ThresholdSignature}, dag, UntypedStoreCommand}, util::{Sha256Hash, Stream}};

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
            let current_round = &bft_state.round_states()[round as usize];

            // Get the current block and signature tips (sorted).
            let sorted_blocks = get_current_block_and_signature_tips(&bft_state, None);

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

/// Gets our current block and signature tips, sorted by (round, block_id).
/// If a round is provided, only blocks less than the given round will be returned.
fn get_current_block_and_signature_tips<SHeaderId>(bft_state: &watch::Ref<'_, BFTState<SHeaderId>>, up_to_round: Option<Round>) -> Vec<(u64, BlockId, Vec<Sha256Hash>)> {
    // Get the current tips.
    let current_tips = bft_state.get_current_tips();

    // If upper bound on round is provided, filter rounds at this round or above.
    let current_tips = current_tips.iter().filter(|(round, _)| { up_to_round.map_or(true, |up_to_round| *round < up_to_round) });

    // Sort by (Round, BlockId)
    let mut sorted_blocks = current_tips.map(|(round, peer_id)| {
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

        (*round, block_id, signatures)
    }).collect::<Vec<_>>();
    sorted_blocks.sort_by_key(|(round, block_id, _)| (*round, *block_id));
    sorted_blocks
}


/// Get the blocks and signatures for this round (sorted by block id).
fn get_round_blocks_and_signatures<SHeaderId>(bft_state: &watch::Ref<'_, BFTState<SHeaderId>>, round: u64) -> Vec<(u64, BlockId, Vec<Sha256Hash>)> {
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

pub struct BFTSyncResponder<SHeaderId> {
    // their_round: Round, // JP: Remove these from here?
    // their_block_tips: Vec<BlockStreamElement>,
    // their_round_complete: Vec<RoundCompleteStreamElement>,
    their_known_blocks: BTreeSet<BlockId>,
    their_known_block_sigs: BTreeSet<(BlockId, Sha256Hash)>,
    send_queue_blocks: BinaryHeap<Reverse<(Round, BlockId)>>,
    send_queue_signatures: BinaryHeap<Reverse<(Round, BlockId, Sha256Hash)>>,
    send_queue_round_signatures: BinaryHeap<Reverse<(Round, Sha256Hash)>>,
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
            // their_round,
            // their_block_tips,
            // their_round_complete,
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
        mut their_round: Round,
        their_block_tips: Vec<BlockStreamElement>,
        their_round_complete: Vec<RoundCompleteStreamElement>,
    ) -> MsgBFTSyncResponse<SHeaderId> {
        // Get all previous tips (sorted) less than the current round?
        let our_previous_tips = get_current_block_and_signature_tips(&bft_state, Some(their_round));

        // Process their tips with our previous tips.
        let mut their_block_tips = their_block_tips.into_iter().peekable();
        let mut status = self.process_their_tips(their_round, &mut their_block_tips, our_previous_tips);
        // TODO: If End, queue following rounds... Lazily?

        let our_round = bft_state.current_round();
        let mut their_round_complete = their_round_complete.into_iter().peekable();

        // Keep processing rounds until we're done (or we've filled the buffer).
        while status != StreamProcessing::Done && their_round <= our_round && !self.buffers_full() {
            let round_blocks = get_round_blocks_and_signatures(&bft_state, their_round);
            status = self.process_their_tips(their_round, &mut their_block_tips, round_blocks);

            // Send round signatures if we've shared everything for this round.
            if status != StreamProcessing::Done {
                // Process the round complete signatures if there are any.
                let round_complete_signatures = get_round_complete_signatures(&bft_state, their_round);
                let complete_status = self.process_their_round_completes(their_round, &mut their_round_complete, round_complete_signatures);
                todo!("...");
                if complete_full {
                    break;
                } else {
                    // They're caught up to this round.

                    if their_round < our_round {
                        // They're still behind so bump their_round and continue.
                        // self.bump_their_round(self.their_round + 1);
                        their_round += 1;
                    } else {
                        // They're caught up to us, so stop.
                        break;
                    }
                }
            }
        }

        // Send all previous tips (sorted) less than the current round (that they don't have)?
        // Get and sort our tips. Along with everything starting from their_round???
        // Iterate over their tips
        // JP: With this approach, we'll miss nodes where we know a child that depends on it but they dont?
        //  Or they know a child that depends on it but we do? But in this case, it's ok, we'll just send it even though they don't need it.


        // Alternative:
        //
        // Mark th

        todo!()
    }

    // fn bump_their_round(&mut self, latest_round: Round) {
    //     self.their_round = latest_round;
    // }

    fn process_their_tips<I:Iterator<Item = BlockStreamElement>>(&mut self, their_round: Round, their_block_tips: &mut Peekable<I>, blocks_and_sigs: Vec<(Round, BlockId, Vec<Sha256Hash>)>) -> StreamProcessing {
        let Some(their_element) = their_block_tips.next() else {
            // TODO: Do we need to do anything here???
            return StreamProcessing::Done;
        };

        warn!("TODO: For subsequent rounds, only queue if they don't already know it? Or check this when sending?");

        let mut blocks_and_sigs = blocks_and_sigs.into_iter();

        let mut their_current_block = match their_element {
            BlockStreamElement::Block(round, block_id) => (round, block_id),
            BlockStreamElement::BlockSignature(sha256_hash) => {
                error!("TODO: Peer deviated from protocol. Started with block signature id: {sha256_hash}");
                todo!("TODO: Peer deviated from protocol. Gracefully handle this");
            }
            BlockStreamElement::End => {
                // Send everything from blocks_and_sigs.
                self.queue_blocks(blocks_and_sigs);

                return StreamProcessing::ReachedEnd;
            }
        };
        self.mark_block_as_known(their_current_block.1);

        while let Some(our_current_element) = blocks_and_sigs.next() {
            let their_round = their_current_block.0;
            let their_block_id = their_current_block.1;
            let our_round = our_current_element.0;
            let our_block_id = our_current_element.1;

            if our_round < their_round || (our_round == their_round && our_block_id < their_block_id) {
                // They don't have our block so send it to them.
                self.queue_block_and_sigs(our_current_element);
            } else { 
                // Send any signatures we have that they don't have for their current block.
                let mut sigs_m = if our_round == their_round && our_block_id == their_block_id {
                    Some(our_current_element.2.into_iter().peekable())
                } else {
                    None
                };

                loop {
                    match their_block_tips.next() {
                        Some(BlockStreamElement::BlockSignature(sig_id)) => {
                            self.mark_block_sig_as_known(their_block_id, sig_id);

                            // Queue sigs from sigs_m that are less than sig_id.
                            self.queue_block_sigs(our_block_id, &mut sigs_m, Some(sig_id));
                        }
                        Some(BlockStreamElement::Block(round, block_id)) => {
                            self.mark_block_as_known(block_id);
                            their_current_block = (round, block_id);

                            // Queue remaining sigs in sigs_m.
                            self.queue_block_sigs(our_block_id, &mut sigs_m, None);
                            break;
                        }
                        Some(BlockStreamElement::End) => {
                            // Queue remaining sigs in sigs_m.
                            self.queue_block_sigs(our_block_id, &mut sigs_m, None);

                            // Send everything remaining in blocks_and_sigs.
                            self.queue_blocks(blocks_and_sigs);

                            return StreamProcessing::ReachedEnd;
                        }
                        None => {
                            // We've reached the end of their stream.
                            return StreamProcessing::Done;
                        }
                    }
                }
            }
        }

        // Process the remaining items they've sent until they get to the next round.
        while let Some(element) = their_block_tips.peek() {

            // Stop if we get to the next round.
            match element {
                BlockStreamElement::Block(round, _) if *round > their_round => {
                    return StreamProcessing::Continue;
                }
                _ => {}
            }
            
            let element = their_block_tips.next().expect("We already peeked");
            match element {
                BlockStreamElement::Block(round, block_id) => {
                    self.mark_block_as_known(block_id);
                    their_current_block = (round, block_id);
                }
                BlockStreamElement::BlockSignature(sig_id) => {
                    self.mark_block_sig_as_known(their_current_block.1, sig_id);
                }
                BlockStreamElement::End => {
                    return StreamProcessing::ReachedEnd;
                }
            }
        }

        StreamProcessing::Done
    }
        // MAX_DELIVER_HEADERS

    // Mark block as known by them.
    fn mark_block_as_known(&mut self, block_id: BlockId) {
        self.their_known_blocks.insert(block_id);
    }

    // Mark block signature as known by them.
    fn mark_block_sig_as_known(&mut self, block_id: BlockId, sig_id: Sha256Hash) {
        self.their_known_block_sigs.insert((block_id, sig_id));
    }

    // Queue sigs from sigs_m that are less than sig_id (if provided). Otherwise, queue them all.
    fn queue_block_sigs(&mut self, block_id: BlockId, sigs_m: &mut Option<Peekable<IntoIter<Sha256Hash>>>, upper_sig_id: Option<Sha256Hash>) {
        if let Some(sigs) = sigs_m {
            while let Some(sig_id) = sigs.next_if( |sig_id|
                upper_sig_id.map_or(true, |upper_sig_id| *sig_id < upper_sig_id)
            ) {
                self.their_known_block_sigs.insert((block_id, sig_id));
            }

            // Drop our next sig if it equals their sig.
            let _ = sigs.next_if(|sig_id| Some(*sig_id) == upper_sig_id);
        }
    }

    /// Queue a block and its signatures.
    fn queue_block_and_sigs(&mut self, (our_round, our_block_id, our_sigs): (u64, BlockId, Vec<Sha256Hash>)) {
            self.send_queue_blocks.push(Reverse((our_round, our_block_id)));

            // Send all of the corresponding signatures.
            let sigs = our_sigs.into_iter().map(|sig_id| Reverse((our_round, our_block_id, sig_id))).collect::<Vec<_>>();
            self.send_queue_signatures.extend(sigs);
    }

    fn queue_blocks(&mut self, blocks_and_sigs: IntoIter<(u64, BlockId, Vec<Sha256Hash>)>) {
        for block_and_sigs in blocks_and_sigs {
            self.queue_block_and_sigs(block_and_sigs);
        }
    }

    fn buffers_full(&self) -> bool {
        self.send_queue_blocks.len() >= MAX_DELIVER_HEADERS.into()
            || self.send_queue_signatures.len() >= MAX_DELIVER_HEADERS.into()
            || self.send_queue_round_signatures.len() >= MAX_DELIVER_HEADERS.into()
    }

    fn process_their_round_completes(&self, their_round: u64, their_round_complete: &mut Peekable<IntoIter<RoundCompleteStreamElement>>, round_complete_signatures: Vec<Sha256Hash>) -> _ {
        todo!()
    }
}

/// Retrieve round complete signatures for this round.
fn get_round_complete_signatures<SHeaderId>(bft_state: &watch::Ref<'_, BFTState<SHeaderId>>, round: u64) -> Vec<Sha256Hash> {
    let round_state = &bft_state.round_states()[round as usize];
    let mut sig_ids = round_state.commit_round().signature_ids();
    sig_ids.sort();
    sig_ids
}

#[derive(PartialEq, Eq)]
enum StreamProcessing {
    Done, // Processed the entire stream, but they did not send "END" so they have more in their tips.
    ReachedEnd, // They sent "END", so they don't have more tips and we can send them everything remaining.
    Continue, // We're still processing the stream.
}
