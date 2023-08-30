
use async_session_types::{Eps, Send, Recv};
use bitvec::{BitArr, order::Msb0};
use std::num::TryFromIntError;

pub mod client;
pub mod server;

// TODO: Move this to the right location.
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
pub const MAX_HAVE_HEADERS : usize = 32;
/// The maximum number of headers that can be sent in each message.
pub const MAX_DELIVER_HEADERS : usize = 32;

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

pub struct MsgECGSync<HeaderId> {
    /// Hashes of headers the server has.
    /// The first `tip_count` hashes (potentially split across multiple messages) are tip headers.
    /// The maximum length is `MAX_HAVE_HEADERS`.
    have: Vec<HeaderId>,
    /// Bitmap of the hashes that the server knows from the previously sent headers `prev.have`.
    known: BitArr!(for MAX_HAVE_HEADERS, in u8, Msb0),
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
