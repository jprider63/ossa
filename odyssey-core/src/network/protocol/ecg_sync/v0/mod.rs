
use async_session_types::{Eps, Send, Recv};
use bitvec::vec::BitVec;

pub mod client;
pub mod server;

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


pub enum ECGSyncError {}

pub struct MsgECGSyncRequest<HeaderId> {
    // Client's tips (hashes of tip headers).
    tips: Vec<HeaderId>,
    // Hashes of headers the client has.
    have: Vec<HeaderId>,
}

pub struct MsgECGSyncResponse<HeaderId> {
    // Server's tips (hashes of tip headers).
    tips: Vec<HeaderId>,
    // Hashes of headers the server has.
    have: Vec<HeaderId>,
    // Bitmap of the hashes that the server has from the previously sent headers `prev.have`.
    bitmap: BitVec,
}

pub struct MsgECGSyncSearch<HeaderId> {
    // Hashes of headers the server has.
    have: Vec<HeaderId>,
    // Bitmap of the hashes that the server has from the previously sent headers `prev.have`.
    bitmap: BitVec,
}
