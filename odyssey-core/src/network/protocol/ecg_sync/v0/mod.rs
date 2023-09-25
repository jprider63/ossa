
use async_session_types::{Eps, Send, Recv};
use bitvec::{BitArr, order::Msb0};
use std::num::TryFromIntError;
use crate::store::ecg;

pub mod client;
pub mod server;
mod test;

// TODO: Move this to the right location.
#[derive(Clone)]
pub struct Header {}

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
pub const MAX_HAVE_HEADERS : u16 = 32;
/// The maximum number of headers that can be sent in each message.
pub const MAX_DELIVER_HEADERS : u16 = 32;

pub enum ECGSyncError {
    // We have too many tips to run the sync protocol.
    TooManyTips(TryFromIntError),
    // TODO: Timeout, IO error, connection terminated, etc...
}

pub struct MsgECGSyncRequest<HeaderId> {
    /// Number of tips the client has.
    tip_count: u16,
    /// Hashes of headers the client has.
    /// The first `tip_count` hashes (potentially split across multiple messages) are tip headers.
    /// The maximum length is `MAX_HAVE_HEADERS`.
    have: Vec<HeaderId>, // Should this include ancestors? Yes.
}

pub struct MsgECGSyncResponse<HeaderId> {
    /// Number of tips the server has.
    tip_count: u16,
    /// `MsgECGSync` sync response.
    sync: MsgECGSync<HeaderId>,
}

pub type HeaderBitmap = BitArr!(for MAX_HAVE_HEADERS as usize, in u8, Msb0);
pub struct MsgECGSync<HeaderId> {
    /// Hashes of headers the server has.
    /// The first `tip_count` hashes (potentially split across multiple messages) are tip headers.
    /// The maximum length is `MAX_HAVE_HEADERS`.
    have: Vec<HeaderId>,
    /// Bitmap of the hashes that the server knows from the previously sent headers `prev.have`.
    known: HeaderBitmap,
    /// Headers being delivered to the other party.
    /// The maximum length is `MAX_DELIVER_HEADERS`.
    headers: Vec<Header>,
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
use std::collections::{BinaryHeap, BTreeSet, VecDeque};
fn prepare_haves<HeaderId:Copy + Ord>(state: &ecg::State<HeaderId>, queue: &mut BinaryHeap<(bool, u64, HeaderId, u64)>, their_known: &BTreeSet<HeaderId>, haves: &mut Vec<HeaderId>)
{
    fn go<HeaderId:Copy + Ord>(state: &ecg::State<HeaderId>, queue: &mut BinaryHeap<(bool, u64, HeaderId, u64)>, their_known: &BTreeSet<HeaderId>, haves: &mut Vec<HeaderId>)
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
                let parents = state.get_parents_with_depth(&header_id);
                for (depth, parent_id) in parents {
                    queue.push((false, depth, parent_id, distance + 1));
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
fn handle_received_have<HeaderId:Copy + Ord>(state: &ecg::State<HeaderId>, their_tips_remaining: &mut usize, their_tips: &mut Vec<HeaderId>, their_known: &mut BTreeSet<HeaderId>, send_queue: &mut BinaryHeap<(u64,HeaderId)>, have: &Vec<HeaderId>, known_bitmap: &mut HeaderBitmap) {
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

            // If we know the header, potentially send the children of that header.
            send_queue.extend(state.get_children_with_depth(&header_id));
        }
    }
}

// Handle (and verify) headers they sent to us.
// Returns if all the headers were valid.
fn handle_received_headers<HeaderId:Copy + Ord, Header>(state: &ecg::State<HeaderId>, headers: Vec<Header>) -> bool {
    // TODO: 
    // Verify header.
    // Add to state.
    unimplemented!{}
}

// Precondition: `state` contains header_id.
// Invariant: if a header is in `their_known`, all the header's ancestors are in `their_known`.
fn mark_as_known<HeaderId:Copy + Ord>(state: &ecg::State<HeaderId>, their_known: &mut BTreeSet<HeaderId>, header_id: HeaderId) {
    fn go<HeaderId:Copy + Ord>(state: &ecg::State<HeaderId>, their_known: &mut BTreeSet<HeaderId>, mut queue: VecDeque<HeaderId>) {
        if let Some(header_id) = queue.pop_front() {
            let contains = their_known.insert(header_id);
            if !contains {
                let parents = state.get_parents(&header_id);
                queue.extend(parents);
            }
            
            go(state, their_known, queue);
        }
    }

    let mut queue = VecDeque::new();
    queue.push_back(header_id);
    go(state, their_known, queue);
}

// Build the headers we will send to the peer.
fn prepare_headers<HeaderId:Copy + Ord, Header>(state: &ecg::State<HeaderId>, send_queue: &mut BinaryHeap<(u64,HeaderId)>, their_known: &mut BTreeSet<HeaderId>, headers: &mut Vec<Header>) {
    fn go<HeaderId:Copy + Ord, Header>(state: &ecg::State<HeaderId>, send_queue: &mut BinaryHeap<(u64,HeaderId)>, their_known: &mut BTreeSet<HeaderId>, headers: &mut Vec<Header>) {
        if headers.len() == MAX_DELIVER_HEADERS.into() {
            return;
        }

        if let Some((_depth,header_id)) = send_queue.pop() {
            // Skip if they already know this header.
            let skip = their_known.contains(&header_id);
            if !skip {
                // Send header to peer.
                let header = state.get_header(&header_id);
                headers.push(header);

                // Mark header as known by peer.
                mark_as_known(state, their_known, header_id);
            }

            // Add children to queue.
            let children = state.get_children_with_depth(&header_id);
            send_queue.extend(children);

            go(state, send_queue, their_known, headers)
        }
    }

    headers.clear();
    go(state, send_queue, their_known, headers)
}

/// Check if the input is a power of two (inclusive of 0).
fn is_power_of_two(x:u64) -> bool {
    0 == (x & (x-1))
}

fn handle_received_known<HeaderId:Copy + Ord>(state: &ecg::State<HeaderId>, their_known: &mut BTreeSet<HeaderId>, sent_haves: &Vec<HeaderId>, received_known: &HeaderBitmap) {
    for (i, header_id) in sent_haves.iter().enumerate() {
        // Check if they claimed they know this header.
        if *received_known.get(i).expect("Unreachable since we're iterating of the headers we sent.") {
            // Mark header as known by them.
            mark_as_known(state, their_known, *header_id);
        }
    }
}

fn handle_received_ecg_sync<HeaderId:Copy + Ord, Header>(sync_msg: MsgECGSync<HeaderId>, state: &ecg::State<HeaderId>, their_tips_remaining: &mut usize, their_tips: &mut Vec<HeaderId>, their_known: &mut BTreeSet<HeaderId>, send_queue: &mut BinaryHeap<(u64,HeaderId)>, queue: &mut BinaryHeap<(bool, u64, HeaderId, u64)>, haves: &mut Vec<HeaderId>, headers: &mut Vec<Header>, known_bitmap: &mut HeaderBitmap) {
    // TODO: XXX 
    unimplemented!("Define ECGSyncState struct with all these variables");
    // XXX
    // XXX

    // Record which headers they say they already know.
    handle_received_known(state, &mut their_known, haves, &sync_msg.known);

    // Receive (and verify) the headers they sent to us
    let all_valid = handle_received_headers(state, sync_msg.headers);
    // TODO: Record and exit if they sent invalid headers? Or tit for tat?

    // TODO: Check for no headers? their_tips_c == 0

    // Handle the haves that the peer sent to us.
    handle_received_have(state, &mut their_tips_remaining, &mut their_tips, &mut their_known, &mut send_queue, &sync_msg.have, &mut known_bitmap);

    // Send the headers we have.
    prepare_headers(state, &mut send_queue, &mut their_known, &mut headers);

    // Propose headers we have.
    prepare_haves(state, queue, &their_known, &mut haves);
}

trait ECGSyncMessage {
    /// Check if we're done based on this message.
    fn is_done(&self) -> bool;
}

impl<HeaderId> ECGSyncMessage for MsgECGSync<HeaderId> {
    fn is_done(&self) -> bool {
        self.have.len() == 0 && self.headers.len() == 0
    }
}

impl<HeaderId> ECGSyncMessage for MsgECGSyncRequest<HeaderId> {
    fn is_done(&self) -> bool {
        self.have.len() == 0
    }
}

impl<HeaderId> ECGSyncMessage for MsgECGSyncResponse<HeaderId> {
    fn is_done(&self) -> bool {
        self.sync.is_done()
    }
}
