// What's the goal? Long term: Request all operations that responder has that initiator doesn't have (R \ I).
// At this step, compute unknown frontier? (Only responder knows it? Or return it?)
//
// The following assumes a snapshot of the ECG DAG
//
// Initiator                  Responder
// ---------                  ---------
// tips_i                     ->
//                                 Mark all their tips and the tips' ancestors as known by them?
//
//                                 // if we don't have any of their tips, our tips are either ancestors or branches
//                                 //    We don't care about ancestors, but we do want to let them know about branches
//                                 for their tips we do have, the tips' children are in the unknown frontier
//                                    Send the tips' children
//                                 for their tips we don't have, our tips are either their ancestors or branches
//                                    We don't care about their ancestors, but we do want to let them know about branches
//
//                            <-   tips_r \ tips_i (and potentially their exponential ancestors?)
// 
// have_bitmap, any_new_tips  ->   
//                                 Mark all their new tips and ancestors as known by them?
//
//                                 If they have the tip we suggested, it's an ancestor so we're done with that tip (it's fine to add children since we don't have children?).
//                                    Mark all the tips and the tip's ancestors as known by them?
//                                 If they don't have the tip, it's a branch
//                            <-      send exponential ancestors (queue order by reverse depth?)
//                                   
// have_bitmap, any_new_tips  ->   ...
//
// 
// Return the frontier? Or the wants? Or the wanted operations themselves?
// If don't have anything to share, tell them to wait?
// Decision: Return the operations.
//
//
// Downsides: O(N) time + space for responder, especially for marking operations as known. Could overshoot and report more wants than necessary
// Good: logN network communication
//
// Byzantine resistance: Probably? As long as initiator validates the provided responses. The responder is upper bounded by the depth of its DAG. The intiator isn't upper bounded though so cut off the peer after some threshold of ? Also cut off peer if I has acknowledged a have but the responder hasn't shared any operations?
//
//
// If we learn about new operations between rounds, what should we do? Send our updated tips? Or just restart if we've learned beyond a threshold number of new operations?
 
use std::{cmp::Reverse, collections::{BTreeSet, BinaryHeap, VecDeque}, fmt::Debug, future::Future, marker::PhantomData};

use bitvec::array::BitArray;
use tokio::sync::oneshot;
use tracing::{debug, warn};

use crate::{network::protocol::{receive, send}, protocol::store_peer::v0::{HeaderBitmap, MsgStoreECGSyncResponse, MsgStoreSync, MsgStoreSyncRequest, StoreSync, MAX_DELIVER_HEADERS, MAX_HAVE_HEADERS}, store::{ecg::{self, RawECGBody}, UntypedStoreCommand}, util::{is_power_of_two, Stream}};


// Has initiative
pub(crate) struct ECGSyncInitiator<Hash, HeaderId, Header> {
    have: Vec<HeaderId>,
    phantom: PhantomData<fn(Hash, HeaderId, Header)>,
}

impl<Hash: Send + Sync, HeaderId: Clone + Debug + Ord + Send + Sync, Header: Debug + Send + Sync> ECGSyncInitiator<Hash, HeaderId, Header> {
    async fn receive_response_helper<S: Stream<MsgStoreSync<Hash, HeaderId, Header>>>(stream: &mut S) -> (Vec<HeaderId>, Vec<(Header, RawECGBody)>) {
        let response = receive(stream).await.expect("TODO");
        let (have, operations) = match response {
            MsgStoreECGSyncResponse::Response { have, operations } => {
                (have, operations)
            }
            MsgStoreECGSyncResponse::Wait => {
                let MsgStoreECGSyncResponse::Response { have, operations } = receive(stream).await.expect("TODO") else {
                    todo!("TODO: Prevent this with session types.");
                };
                (have, operations)
            }
        };
        warn!("TODO: Check response sizes.");

        (have, operations)
    }

    /// Create a new ECGSyncInitiator and run the first round.
    // TODO: Eventually take an Arc<RWLock>
    pub(crate) async fn run_new<S: Stream<MsgStoreSync<Hash, HeaderId, Header>>>(stream: &mut S, ecg_state: &ecg::UntypedState<HeaderId, Header>) -> (Self, Vec<(Header, RawECGBody)>) {
        // TODO: Limit on tips (128? 64? 32? MAX_HAVE_HEADERS)
        warn!("TODO: Check request sizes.");
        let req = MsgStoreSyncRequest::ECGInitialSync { tips: ecg_state.tips().iter().cloned().collect() };
        send(stream, req).await.expect("TODO");

        // Receive response.
        let (have, operations) = Self::receive_response_helper(stream).await;

        let ecg_sync = ECGSyncInitiator {
            have,
            phantom: PhantomData,
        };

        (ecg_sync, operations)
    }

    /// Run a round of ECG sync, requesting new operations from peer.
    pub(crate) async fn run_round<S: Stream<MsgStoreSync<Hash, HeaderId, Header>>>(&mut self, stream: &mut S, ecg_state: &ecg::UntypedState<HeaderId, Header>) -> Vec<(Header, RawECGBody)> {

        // Check which headers they sent us that we know.
        let mut known_bitmap = BitArray::ZERO;
        for (i, header_id) in self.have.iter().enumerate() {
            if ecg_state.contains(header_id) {
                // Respond with which headers we know.
                known_bitmap.set(i, true);
            }
        }

        // TODO: Limit on tips (128? 64? 32? MAX_HAVE_HEADERS)
        warn!("TODO: Check request sizes.");
        let req = MsgStoreSyncRequest::ECGSync { tips: ecg_state.tips().iter().cloned().collect(), known: known_bitmap };
        send(stream, req).await.expect("TODO");

        // Receive response.
        let (have, operations) = Self::receive_response_helper(stream).await;

        self.have = have;

        operations
    }
}

// Responder
pub(crate) struct ECGSyncResponder<Hash, HeaderId, Header> {
    pub(crate) their_known: BTreeSet<HeaderId>,
    pub(crate) our_unknown: BTreeSet<HeaderId>,
    pub(crate) send_queue: BinaryHeap<(Reverse<u64>, HeaderId)>,
    // Haves we sent.
    pub(crate) sent_haves: Vec<HeaderId>,
    pub(crate) haves_queue: BinaryHeap<(bool, u64, HeaderId, u64)>,
    phantom: PhantomData<fn(Hash, HeaderId, Header)>,
}

pub(crate) fn mark_as_known_helper<HeaderId, Header>(
    their_known: &mut BTreeSet<HeaderId>,
    state: &ecg::UntypedState<HeaderId, Header>,
    header_id: HeaderId,
) where
    HeaderId: Copy + Ord,
{
    let mut queue = VecDeque::new();
    queue.push_back(header_id);

    while let Some(header_id) = queue.pop_front() {
        let contains = their_known.insert(header_id);
        if !contains {
            if let Some(parents) = state.get_parents(&header_id) {
                queue.extend(parents);
            } else {
                unreachable!("Precondition violated. Header must be known.");
            }
        }

        // If we know header, we can remove it from our_unknown.
        warn!("TODO: What do we do with unknown? Remove them up front after receiving every updated ecgstate?");
        // self.our_unknown.remove(&header_id);
    }
}

impl<Hash, HeaderId, Header> ECGSyncResponder<Hash, HeaderId, Header> {
    fn they_know(&self, header_id: &HeaderId) -> bool
    where
        HeaderId: Copy + Ord,
    {
        self.their_known.contains(header_id) || self.our_unknown.contains(header_id)
    }

    // Marks a header as known by them. Requires that we know it.
    // Precondition: `state` contains header_id.
    // Invariant: if a header is in `their_known`, all the header's ancestors are in `their_known`.
    // JP: Can we avoid this linear time + memory?
    pub(crate) fn mark_as_known(
        &mut self,
        state: &ecg::UntypedState<HeaderId, Header>,
        // their_known: &mut BTreeSet<Header::HeaderId>,
        header_id: HeaderId,
    ) where
        HeaderId: Copy + Ord,
    {
        mark_as_known_helper(&mut self.their_known, state, header_id)
    }

    pub(crate) fn new() -> Self
    where
        HeaderId: std::cmp::Ord,
    {
        ECGSyncResponder {
            their_known: BTreeSet::new(),
            send_queue: BinaryHeap::new(),
            our_unknown: BTreeSet::new(),
            sent_haves: Vec::with_capacity(MAX_HAVE_HEADERS as usize),
            haves_queue: BinaryHeap::new(),
            phantom: PhantomData,
        }
    }

    pub(crate) async fn run_response_helper<S: Stream<MsgStoreSync<Hash, HeaderId, Header>>>(&mut self, store_peer: &StoreSync<Hash, HeaderId, Header>, stream: &mut S, mut ecg_state: ecg::UntypedState<HeaderId, Header>, their_tips: Vec<HeaderId>)
    where
        HeaderId: Debug + Ord + Copy,
        Header: Debug + Clone,
    {
        let mut is_first_run = true;
        loop {
            if !is_first_run {
                // Reprocess their tips to save a roundtrip.
                warn!("TODO: Use a heuristic to decide whether to submit new operations?");
                self.handle_their_tips(&ecg_state, &their_tips);
            }

            // Add our tips to the queue.
            self.prepare_our_tips(&ecg_state);

            // Send the operations we have.
            let operations = self.prepare_operations(&ecg_state);

            // Propose headers we have.
            self.prepare_sent_haves(&ecg_state);

            debug!("Prepared sent_haves: {:?}", self.sent_haves);
            debug!("Prepared operations: {:?}", operations);

            // Check response is empty.
            if operations.is_empty() && self.sent_haves.is_empty() {
                // Tell them to wait if we haven't responded yet.
                if is_first_run {
                    is_first_run = false;
                    let msg = MsgStoreECGSyncResponse::Wait;
                    send(stream, msg).await.expect("TODO");
                }

                // Subscribe for updates.
                debug!("Subscribing to ECG state");
                let (response_chan, recv_chan) = oneshot::channel();
                let cmd = UntypedStoreCommand::SubscribeECG { peer: store_peer.peer(), tips: Some(ecg_state.tips().iter().cloned().collect()), response_chan };
                store_peer.send_chan().send(cmd).expect("TODO");

                // Wait for ECG updates.
                ecg_state = recv_chan.await.expect("TODO");
                self.update_our_unknown(&ecg_state);

                // self.run_response_helper(store_peer, stream, &ecg_state, false).await;
            } else {
                // Send response.
                let msg = MsgStoreECGSyncResponse::Response { have: self.sent_haves.clone(), operations };
                send(stream, msg).await.expect("TODO");
                return;
            }
        }
    }

    /// Create a new ECGSyncResponder and run the first round.
    // TODO: Eventually take an Arc<RWLock>? Could also take a watch::Receiver?
    pub(crate) async fn run_initial<S: Stream<MsgStoreSync<Hash, HeaderId, Header>>>(&mut self, store_peer: &StoreSync<Hash, HeaderId, Header>, stream: &mut S, ecg_state: ecg::UntypedState<HeaderId, Header>, their_tips: Vec<HeaderId>)
    where
        HeaderId: Debug + Ord + Copy,
        Header: Debug + Clone,
    {
        // Process their tips.
        self.handle_their_tips(&ecg_state, &their_tips);

        self.run_response_helper(store_peer, stream, ecg_state, their_tips).await;
    }

    pub(crate) async fn run_round<S: Stream<MsgStoreSync<Hash, HeaderId, Header>>>(&mut self, store_peer: &StoreSync<Hash, HeaderId, Header>, stream: &mut S, ecg_state: ecg::UntypedState<HeaderId, Header>, their_tips: Vec<HeaderId>, their_known: HeaderBitmap)
    where
        HeaderId: Debug + Copy + Ord,
        Header: Debug + Clone,
    {
        // Process their tips.
        self.handle_their_tips(&ecg_state, &their_tips);

        // Record which headers they say they already know.
        self.handle_their_known(&ecg_state, their_known);

        self.run_response_helper(store_peer, stream, ecg_state, their_tips).await;
    }

    fn prepare_operations(&mut self, ecg_state: &ecg::UntypedState<HeaderId, Header>) -> Vec<(Header, RawECGBody)>
    where
        HeaderId: Ord + Copy,
        Header: Clone,
    {
        let mut operations = Vec::with_capacity(MAX_DELIVER_HEADERS as usize);

        while let Some((_depth, header_id)) = self.send_queue.pop() {
            // Skip if they already know this header.
            let skip = self.they_know(&header_id);
            if !skip {
                // Send header to peer.
                if let Some(node) = ecg_state.get_node(&header_id) {
                    operations.push((node.header().clone(), node.operations().clone()));

                    // Mark header as known by peer.
                    self.mark_as_known(ecg_state, header_id);
                } else {
                    unreachable!("We know this header.");
                }
            }

            // Add children to queue.
            let children = ecg_state
                .get_children_with_depth(&header_id)
                .expect("Unreachable since we proposed this header.");
            self.send_queue.extend(children);

            if operations.len() == MAX_DELIVER_HEADERS.into() {
                return operations;
            }
        }

        operations
    }

    // Prepares and stores the sent_haves we will send back to peer.
    fn prepare_sent_haves(&mut self, ecg_state: &ecg::UntypedState<HeaderId, Header>)
    where
        HeaderId: Ord + Copy
    {
        self.sent_haves.clear();

        while let Some((_is_tip, depth, header_id, distance)) = self.haves_queue.pop() {
            // If they already know this header, they already know its ancestors.
            let skip = self.they_know(&header_id);
            if !skip {
                // If header is at an exponential distance (or is a child of the root node), send it with `haves`.
                if is_power_of_two(distance) || depth == 1 {
                    self.sent_haves.push(header_id);
                }

                // Add parents to queue.
                if let Some(parents) = ecg_state.get_parents_with_depth(&header_id) {
                    for (depth, parent_id) in parents {
                        self.haves_queue.push((false, depth, parent_id, distance + 1));
                    }
                } else {
                    unreachable!("Invariant: we know everything in queue.");
                }
            }

            // If we've reached the upper bound, stop.
            if self.sent_haves.len() == MAX_HAVE_HEADERS.into() {
                return;
            }
        }
    }

    fn handle_their_tips(&mut self, ecg_state: &ecg::UntypedState<HeaderId, Header>, their_tips: &[HeaderId])
    where
        HeaderId: Ord + Copy,
    {
        // If they don't have any operations, share all root nodes.
        if their_tips.is_empty() {
            let root_nodes = ecg_state.get_root_nodes_with_depth();
            self.send_queue.extend(root_nodes);
        }

        their_tips.iter().for_each(|header_id| {
            // Check if we know the header.
            if ecg_state.contains(header_id) {
                // Mark header and its ancestors as known.
                self.mark_as_known(ecg_state, *header_id);

                // Add children to send queue.
                let children = ecg_state
                    .get_children_with_depth(header_id)
                    .expect("Unreachable since we have this header.");
                self.send_queue.extend(children);
            } else {
                // Record header as known by them but not us.
                self.our_unknown.insert(*header_id);
            }
        });
    }

    fn prepare_our_tips(&mut self, ecg_state: &ecg::UntypedState<HeaderId, Header>)
    where
        HeaderId: std::cmp::Ord + Copy,
    {
        // Initialize the queue with our tips, zipped with distance 0.
        // JP: Reset/clear haves_queue when our tips changes? Probably not
        self.haves_queue.extend(ecg_state.tips().iter()
            // .filter(|x| self.they_know(x))
            .map(|x| {
                if let Some(depth) = ecg_state.get_header_depth(x) {
                    (true, depth, *x, 0)
                } else {
                    unreachable!("We have this header");
                }
            }
        ));
    }

    fn handle_their_known(&mut self, ecg_state: &ecg::UntypedState<HeaderId, Header>, their_known: HeaderBitmap)
    where
        HeaderId: Copy + Ord,
    {
        for (i, header_id) in self.sent_haves.iter().cloned().enumerate() {
            // Check if they claimed they know this header.
            let they_know = *their_known
                .get(i)
                .expect("Unreachable since we're iterating on the headers we sent.");
            if they_know {
                // Mark header as known by them.
                mark_as_known_helper(&mut self.their_known, ecg_state, header_id);

                // Send children if they know this node.
                let children = ecg_state
                    .get_children_with_depth(&header_id)
                    .expect("Unreachable since we sent this header.");
                self.send_queue.extend(children);
            } else {
                let parents = ecg_state
                    .get_parents(&header_id)
                    .expect("Unreachable since we sent this header.");
                // Send the node if they don't know it and they know all its parents (including if it's a root node).
                if ecg_state.is_root_node(&header_id) || parents.iter().all(|p| self.their_known.contains(p)) {
                    let depth = ecg_state
                        .get_header_depth(&header_id)
                        .expect("Unreachable since we sent this header.");
                    self.send_queue.push((Reverse(depth), header_id));
                }
            }
        }
    }

    pub(crate) fn update_our_unknown(&mut self, ecg_state: &ecg::UntypedState<HeaderId, Header>)
    where
        HeaderId: Ord + Copy,
    {
        // If we now know a header, remove it from our unknown and mark it as known to them.
        for header_id in self.our_unknown.extract_if(|header_id| ecg_state.contains(header_id)) {
            mark_as_known_helper(&mut self.their_known, ecg_state, header_id);
        }
    }
}


