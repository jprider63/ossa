
use async_session_types::{Eps, Send, Recv};

pub mod client;
pub mod server;

/// TODO:
/// The session type for the protocol.
pub type KeepAlive = Send<MsgKeepAliveRequest,Recv<MsgKeepAliveResponse, Eps>>;

// TODO:
// impl Protocol for KeepAlive {}

pub enum MsgKeepAlive {
    Request(MsgKeepAliveRequest),
    Done(MsgKeepAliveDone),
    Response(MsgKeepAliveResponse),
}

// Messages for the protocol.
pub struct MsgKeepAliveRequest {
    heartbeat: u16,
}
pub struct MsgKeepAliveDone {}
pub struct MsgKeepAliveResponse {
    heartbeat: u16,
}

// Protocol error
pub enum KeepAliveError {
    MismatchedHeartbeats {
        expected: u16,
        response: u16,
    },
}
