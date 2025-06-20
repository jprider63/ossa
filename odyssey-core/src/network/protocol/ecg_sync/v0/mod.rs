use crate::store::ecg::{self, ECGHeader};
use async_session_types::{Eps, Recv, Send};
use bitvec::{order::Msb0, BitArr};
use odyssey_crdt::CRDT;
use std::cmp::Reverse;
use std::fmt::{self, Debug};
use std::marker::PhantomData;
use std::num::TryFromIntError;

pub mod client;
pub mod server;
#[cfg(false)]
mod test;

/// TODO:
/// The session type for the ecg-sync protocol.
pub type ECGSync = Send<(), Eps>; // TODO

// Client:
//
// Send hashes of tips.
// Send hashes of (skipped) ancestors.
//
// Server:
//
// Send hashes of tips.
// Send hashes of (skipped) ancestors.
// Send bitmap indices of prev.have that we have.
//
// Loop until meet identified:
//
//   Client:
//
//   Send hashes of (skipped) ancestors.
//   Send bitmap indices of prev.have that we have.
//
//   Server:
//
//   Send hashes of (skipped) ancestors.
//   Send bitmap indices of prev.have that we have.
//
// Client:
//
// Send all headers he have that they don't (batched).
//
// Client:
//
// Send all headers he have that they don't (batched).
//

/// The maximum number of `have` hashes that can be sent in each message.
pub const MAX_HAVE_HEADERS: u16 = 32;
/// The maximum number of headers that can be sent in each message.
pub const MAX_DELIVER_HEADERS: u16 = 32;

#[derive(Debug, PartialEq)]
pub enum ECGSyncError {
    // We have too many tips to run the sync protocol.
    TooManyTips(TryFromIntError),
    // TODO: Timeout, IO error, connection terminated, etc...
}

pub enum MsgECGSync<H: ECGHeader<T>, T: CRDT> {
    Request(MsgECGSyncRequest<H, T>),
    Response(MsgECGSyncResponse<H, T>),
    Sync(MsgECGSyncData<H, T>),
}

pub struct MsgECGSyncRequest<Header: ECGHeader<T>, T: CRDT> {
    /// Number of tips the client has.
    tip_count: u16,
    /// Hashes of headers the client has.
    /// The first `tip_count` hashes (potentially split across multiple messages) are tip headers.
    /// The maximum length is `MAX_HAVE_HEADERS`.
    have: Vec<Header::HeaderId>, // Should this include ancestors? Yes.
    phantom: PhantomData<T>,
}

impl<Header: ECGHeader<T> + Debug, T: CRDT> Debug for MsgECGSyncRequest<Header, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MsgECGSyncRequest")
            .field("tip_count", &self.tip_count)
            .field("have", &self.have)
            .finish()
    }
}

pub struct MsgECGSyncResponse<Header: ECGHeader<T>, T: CRDT> {
    /// Number of tips the server has.
    tip_count: u16,
    /// `MsgECGSyncData` sync response.
    sync: MsgECGSyncData<Header, T>,
}

impl<Header: ECGHeader<T> + Debug, T: CRDT> Debug for MsgECGSyncResponse<Header, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MsgECGSyncResponse")
            .field("tip_count", &self.tip_count)
            .field("sync", &self.sync)
            .finish()
    }
}

pub type HeaderBitmap = BitArr!(for MAX_HAVE_HEADERS as usize, in u8, Msb0);
pub struct MsgECGSyncData<Header: ECGHeader<T>, T: CRDT> {
    /// Hashes of headers the server has.
    /// The first `tip_count` hashes (potentially split across multiple messages) are tip headers.
    /// The maximum length is `MAX_HAVE_HEADERS`.
    have: Vec<Header::HeaderId>,
    /// Bitmap of the hashes that the server knows from the previously sent headers `prev.have`.
    known: HeaderBitmap,
    /// Headers being delivered to the other party.
    /// The maximum length is `MAX_DELIVER_HEADERS`.
    headers: Vec<Header>,
    phantom: PhantomData<T>,
}

impl<Header: ECGHeader<T> + Debug, T: CRDT> Debug for MsgECGSyncData<Header, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MsgECGSyncData")
            .field("have", &self.have)
            .field("known", &self.known)
            .field("headers", &self.headers)
            .finish()
    }
}

// pub struct ECGSyncState<HeaderId> {
//     our_tips: Vec<HeaderId>,
// }
//
// impl<HeaderId> ECGSyncState<HeaderId> {
//     pub fn new(tips: Vec<HeaderId>) -> Self {
//         ECGSyncState {
//             our_tips: tips.to_vec(),
//         }
//     }
// }

use std::cmp::min;
use std::collections::{BTreeSet, BinaryHeap, VecDeque};
fn prepare_haves<Header: ECGHeader<T>, T: CRDT>(
    state: &ecg::State<Header, T>,
    queue: &mut BinaryHeap<(bool, u64, Header::HeaderId, u64)>,
    their_known: &BTreeSet<Header::HeaderId>,
    haves: &mut Vec<Header::HeaderId>,
) where
    Header::HeaderId: Copy + Ord,
{
    fn go<Header: ECGHeader<T>, T: CRDT>(
        state: &ecg::State<Header, T>,
        queue: &mut BinaryHeap<(bool, u64, Header::HeaderId, u64)>,
        their_known: &BTreeSet<Header::HeaderId>,
        haves: &mut Vec<Header::HeaderId>,
    ) where
        Header::HeaderId: Copy + Ord,
    {
        if haves.len() == MAX_HAVE_HEADERS.into() {
            return;
        }

        if let Some((_is_tip, depth, header_id, distance)) = queue.pop() {
            // If they already know this header, they already know its ancestors.
            let skip = their_known.contains(&header_id);
            if !skip {
                // If header is at an exponential distance (or is a child of the root node), send it with `haves`.
                if is_power_of_two(distance) || depth == 1 {
                    haves.push(header_id);
                }

                // Add parents to queue.
                if let Some(parents) = state.get_parents_with_depth(&header_id) {
                    for (depth, parent_id) in parents {
                        queue.push((false, depth, parent_id, distance + 1));
                    }
                } else {
                    // TODO XXX
                    todo!("Do we need to do anything?")
                }
            }

            go(state, queue, their_known, haves)
        }
    }

    haves.clear();
    go(state, queue, their_known, haves)
}

// Handle the haves that the peer sent to us.
// Returns the bitmap of which haves we know.
fn handle_received_have<Header: ECGHeader<T>, T: CRDT>(
    state: &ecg::State<Header, T>,
    their_tips_remaining: &mut usize,
    their_tips: &mut Vec<Header::HeaderId>,
    their_known: &mut BTreeSet<Header::HeaderId>,
    // send_queue: &mut BinaryHeap<(u64, Header::HeaderId)>,
    have: &Vec<Header::HeaderId>,
    known_bitmap: &mut HeaderBitmap,
) where
    Header::HeaderId: Copy + Ord,
{
    // Accumulate their_tips.
    let provided_tip_c = min(*their_tips_remaining, have.len());
    their_tips.extend(&have[0..provided_tip_c]);
    *their_tips_remaining = *their_tips_remaining - provided_tip_c;
    // TODO: if their_tips is done, update peer_state.

    known_bitmap.fill(false);
    for (i, header_id) in have.iter().enumerate() {
        if state.contains(header_id) {
            // Record their known headers.
            mark_as_known(state, their_known, *header_id);

            // Respond with which headers we know.
            known_bitmap.set(i, true);

            // TODO: Do we actually need this? XXX
            // // If we know the header, potentially send the children of that header.
            // if let Some(children) = state.get_children_with_depth(&header_id) {
            //     send_queue.extend(children);
            // } else {
            //     // TODO XXX
            //     todo!("Do we need to do anything?")
            // }
        }
    }
}

// Handle (and verify) headers they sent to us.
// Returns if all the headers were valid.
fn handle_received_headers<Header: ECGHeader<T>, T: CRDT>(
    state: &mut ecg::State<Header, T>,
    headers: Vec<Header>,
) -> bool {
    let mut all_valid = true;
    for header in headers {
        // TODO: XXX
        // XXX
        // Verify header.
        // all_valid = all_valid && true;
        // XXX

        // Add to state.
        state.insert_header(header);
    }

    all_valid
}

// Precondition: `state` contains header_id.
// Invariant: if a header is in `their_known`, all the header's ancestors are in `their_known`.
fn mark_as_known<Header: ECGHeader<T>, T: CRDT>(
    state: &ecg::State<Header, T>,
    their_known: &mut BTreeSet<Header::HeaderId>,
    header_id: Header::HeaderId,
) where
    Header::HeaderId: Copy + Ord,
{
    fn go<Header: ECGHeader<T>, T: CRDT>(
        state: &ecg::State<Header, T>,
        their_known: &mut BTreeSet<Header::HeaderId>,
        mut queue: VecDeque<Header::HeaderId>,
    ) where
        Header::HeaderId: Copy + Ord,
    {
        if let Some(header_id) = queue.pop_front() {
            let contains = their_known.insert(header_id);
            if !contains {
                if let Some(parents) = state.get_parents(&header_id) {
                    queue.extend(parents);
                } else {
                    // TODO XXX
                    todo!("unreachable?")
                }
            }

            go(state, their_known, queue);
        }
    }

    let mut queue = VecDeque::new();
    queue.push_back(header_id);
    go(state, their_known, queue);
}

// Build the headers we will send to the peer.
fn prepare_headers<Header: ECGHeader<T>, T: CRDT>(
    state: &ecg::State<Header, T>,
    send_queue: &mut BinaryHeap<(Reverse<u64>, Header::HeaderId)>,
    their_known: &mut BTreeSet<Header::HeaderId>,
    headers: &mut Vec<Header>,
) where
    Header::HeaderId: Copy + Ord,
    Header: Clone,
{
    fn go<Header: ECGHeader<T>, T: CRDT>(
        state: &ecg::State<Header, T>,
        send_queue: &mut BinaryHeap<(Reverse<u64>, Header::HeaderId)>,
        their_known: &mut BTreeSet<Header::HeaderId>,
        headers: &mut Vec<Header>,
    ) where
        Header::HeaderId: Copy + Ord,
        Header: Clone,
    {
        if headers.len() == MAX_DELIVER_HEADERS.into() {
            return;
        }

        if let Some((_depth, header_id)) = send_queue.pop() {
            // Skip if they already know this header.
            let skip = their_known.contains(&header_id);
            if !skip {
                // Send header to peer.
                if let Some(header) = state.get_header(&header_id) {
                    headers.push(header.clone());

                    // Mark header as known by peer.
                    mark_as_known(state, their_known, header_id);
                } else {
                    // TODO XXX
                    todo!("unreachable?")
                }
            }

            // Add children to queue.
            let children = state
                .get_children_with_depth(&header_id)
                .expect("Unreachable since we proposed this header.");
            send_queue.extend(children);

            go(state, send_queue, their_known, headers)
        }
    }

    headers.clear();
    go(state, send_queue, their_known, headers)
}

/// Check if the input is a power of two (inclusive of 0).
fn is_power_of_two(x: u64) -> bool {
    0 == (x & (x.wrapping_sub(1)))
}

fn handle_received_known<Header: ECGHeader<T>, T: CRDT>(
    state: &ecg::State<Header, T>,
    their_known: &mut BTreeSet<Header::HeaderId>,
    sent_haves: &Vec<Header::HeaderId>,
    received_known: &HeaderBitmap,
    send_queue: &mut BinaryHeap<(Reverse<u64>, Header::HeaderId)>,
) where
    Header::HeaderId: Copy + Ord,
{
    for (i, header_id) in sent_haves.iter().enumerate() {
        // Check if they claimed they know this header.
        let they_know = *received_known
            .get(i)
            .expect("Unreachable since we're iterating on the headers we sent.");
        if they_know {
            // Mark header as known by them.
            mark_as_known(state, their_known, *header_id);

            // Send children if they know this node.
            let children = state
                .get_children_with_depth(&header_id)
                .expect("Unreachable since we sent this header.");
            send_queue.extend(children);
        } else {
            let parents = state
                .get_parents(header_id)
                .expect("Unreachable since we sent this header.");
            // Send the node if they don't know it and they know all its parents (including if it's a root node).
            if state.is_root_node(header_id) || parents.iter().all(|p| their_known.contains(p)) {
                let depth = state
                    .get_header_depth(header_id)
                    .expect("Unreachable since we sent this header.");
                send_queue.push((Reverse(depth), *header_id));
            }
        }

        // let is_root_node = state.is_root_node(header_id);
        // match (they_know, is_root_node) {
        //     (false, true) => {
        //         // Send the node if it's a root and they don't know it.
        //         send_queue.push(header_id);
        //     }
        //     (true, true) => {
        //         // Send children if it's a root node and they know it.
        //         send children
        //     }
        //     (true, false) => {
        //         // send children if they know this
        //     }
        //     (false, false) => {
        //         // Do nothing.
        //     }
        // }
    }
}

fn handle_received_ecg_sync<Header: ECGHeader<T>, T: CRDT>(
    sync_msg: MsgECGSyncData<Header, T>,
    state: &mut ecg::State<Header, T>,
    their_tips_remaining: &mut usize,
    their_tips: &mut Vec<Header::HeaderId>,
    their_known: &mut BTreeSet<Header::HeaderId>,
    send_queue: &mut BinaryHeap<(Reverse<u64>, Header::HeaderId)>,
    queue: &mut BinaryHeap<(bool, u64, Header::HeaderId, u64)>,
    haves: &mut Vec<Header::HeaderId>,
    headers: &mut Vec<Header>,
    known_bitmap: &mut HeaderBitmap,
) where
    Header::HeaderId: Copy + Ord,
    Header: Clone,
{
    // TODO: XXX
    // unimplemented!("Define ECGSyncState struct with all these variables");
    // XXX
    // XXX

    // Record which headers they say they already know.
    handle_received_known(state, their_known, haves, &sync_msg.known, send_queue);

    // Receive (and verify) the headers they sent to us
    let all_valid = handle_received_headers(state, sync_msg.headers);
    // TODO: Record and exit if they sent invalid headers? Or tit for tat?

    // TODO: Check for no headers? their_tips_c == 0

    // Handle the haves that the peer sent to us.
    handle_received_have(
        state,
        their_tips_remaining,
        their_tips,
        their_known,
        // send_queue,
        &sync_msg.have,
        known_bitmap,
    );

    // Send the headers we have.
    prepare_headers(state, send_queue, their_known, headers);

    // Propose headers we have.
    prepare_haves(state, queue, &their_known, haves);
}

trait ECGSyncMessage {
    /// Check if we're done based on this message.
    fn is_done(&self) -> bool;
}

impl<Header: ECGHeader<T>, T: CRDT> ECGSyncMessage for MsgECGSyncData<Header, T> {
    fn is_done(&self) -> bool {
        self.have.len() == 0 && self.headers.len() == 0
    }
}

impl<Header: ECGHeader<T>, T: CRDT> ECGSyncMessage for MsgECGSyncRequest<Header, T> {
    fn is_done(&self) -> bool {
        self.have.len() == 0
    }
}

impl<Header: ECGHeader<T>, T: CRDT> ECGSyncMessage for MsgECGSyncResponse<Header, T> {
    fn is_done(&self) -> bool {
        self.sync.is_done()
    }
}

impl<H: ECGHeader<T>, T: CRDT> Into<MsgECGSync<H, T>> for MsgECGSyncRequest<H, T> {
    fn into(self) -> MsgECGSync<H, T> {
        MsgECGSync::Request(self)
    }
}
impl<H: ECGHeader<T>, T: CRDT> Into<MsgECGSync<H, T>> for MsgECGSyncResponse<H, T> {
    fn into(self) -> MsgECGSync<H, T> {
        MsgECGSync::Response(self)
    }
}
impl<H: ECGHeader<T>, T: CRDT> Into<MsgECGSync<H, T>> for MsgECGSyncData<H, T> {
    fn into(self) -> MsgECGSync<H, T> {
        MsgECGSync::Sync(self)
    }
}
impl<H: ECGHeader<T>, T: CRDT> TryInto<MsgECGSyncRequest<H, T>> for MsgECGSync<H, T> {
    type Error = ();
    fn try_into(self) -> Result<MsgECGSyncRequest<H, T>, ()> {
        match self {
            MsgECGSync::Request(r) => Ok(r),
            MsgECGSync::Response(_) => Err(()),
            MsgECGSync::Sync(_) => Err(()),
        }
    }
}
impl<H: ECGHeader<T>, T: CRDT> TryInto<MsgECGSyncResponse<H, T>> for MsgECGSync<H, T> {
    type Error = ();
    fn try_into(self) -> Result<MsgECGSyncResponse<H, T>, ()> {
        match self {
            MsgECGSync::Request(_) => Err(()),
            MsgECGSync::Response(r) => Ok(r),
            MsgECGSync::Sync(_) => Err(()),
        }
    }
}
impl<H: ECGHeader<T>, T: CRDT> TryInto<MsgECGSyncData<H, T>> for MsgECGSync<H, T> {
    type Error = ();
    fn try_into(self) -> Result<MsgECGSyncData<H, T>, ()> {
        match self {
            MsgECGSync::Request(_) => Err(()),
            MsgECGSync::Response(_) => Err(()),
            MsgECGSync::Sync(s) => Ok(s),
        }
    }
}
