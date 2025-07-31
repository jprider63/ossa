use async_session_types::{Eps, Recv, Send};

pub mod client;
pub mod server;

/// TODO:
/// The session type for the protocol.
pub type KeepAlive = Send<MsgKeepAliveRequest, Recv<MsgKeepAliveResponse, Eps>>;

// TODO:
// impl Protocol for KeepAlive {}

pub enum MsgKeepAlive {
    Request(MsgKeepAliveRequest),
    Done(MsgKeepAliveDone),
    Response(MsgKeepAliveResponse),
}

// Messages for the protocol.
#[derive(Debug)]
pub struct MsgKeepAliveRequest {
    heartbeat: u16,
}
#[derive(Debug)]
pub struct MsgKeepAliveDone {}
#[derive(Debug)]
pub struct MsgKeepAliveResponse {
    heartbeat: u16,
}

// Protocol error
pub enum KeepAliveError {
    MismatchedHeartbeats { expected: u16, response: u16 },
}

impl Into<MsgKeepAlive> for MsgKeepAliveRequest {
    fn into(self) -> MsgKeepAlive {
        MsgKeepAlive::Request(self)
    }
}
impl Into<MsgKeepAlive> for MsgKeepAliveDone {
    fn into(self) -> MsgKeepAlive {
        MsgKeepAlive::Done(self)
    }
}
impl Into<MsgKeepAlive> for MsgKeepAliveResponse {
    fn into(self) -> MsgKeepAlive {
        MsgKeepAlive::Response(self)
    }
}
impl TryInto<MsgKeepAliveRequest> for MsgKeepAlive {
    type Error = ();
    fn try_into(self) -> Result<MsgKeepAliveRequest, ()> {
        match self {
            MsgKeepAlive::Request(r) => Ok(r),
            MsgKeepAlive::Done(_) => Err(()),
            MsgKeepAlive::Response(_) => Err(()),
        }
    }
}
impl TryInto<MsgKeepAliveDone> for MsgKeepAlive {
    type Error = ();
    fn try_into(self) -> Result<MsgKeepAliveDone, ()> {
        match self {
            MsgKeepAlive::Request(_) => Err(()),
            MsgKeepAlive::Done(r) => Ok(r),
            MsgKeepAlive::Response(_) => Err(()),
        }
    }
}
impl TryInto<MsgKeepAliveResponse> for MsgKeepAlive {
    type Error = ();
    fn try_into(self) -> Result<MsgKeepAliveResponse, ()> {
        match self {
            MsgKeepAlive::Request(_) => Err(()),
            MsgKeepAlive::Done(_) => Err(()),
            MsgKeepAlive::Response(r) => Ok(r),
        }
    }
}
