use rand::{rng, Rng};
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::time::SystemTime;
use tokio::time::{sleep, Duration};
use tracing::debug;

use crate::{
    network::protocol::{receive, send, MiniProtocol},
    util::Stream,
};

/// TODO:
/// The session type for the ecg-sync protocol.
// pub type Heartbeat = Send<(), Eps>; // TODO

// server to client: (Time, u64)
// client to server: (Time, u64, Time)
// server to client: (u64, Time)

#[derive(Debug, Serialize, Deserialize)]
pub(crate) enum MsgHeartbeat {
    Request(MsgHeartbeatRequest),
    ClientResponse(MsgHeartbeatClientResponse),
    ServerResponse(MsgHeartbeatServerResponse),
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct MsgHeartbeatRequest {
    server_time: SystemTime,
    heartbeat: u64,
}
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct MsgHeartbeatClientResponse {
    heartbeat: u64,
    client_time: SystemTime,
}
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct MsgHeartbeatServerResponse {
    heartbeat: u64,
}

impl Into<MsgHeartbeat> for MsgHeartbeatRequest {
    fn into(self) -> MsgHeartbeat {
        MsgHeartbeat::Request(self)
    }
}

impl Into<MsgHeartbeat> for MsgHeartbeatClientResponse {
    fn into(self) -> MsgHeartbeat {
        MsgHeartbeat::ClientResponse(self)
    }
}

impl Into<MsgHeartbeat> for MsgHeartbeatServerResponse {
    fn into(self) -> MsgHeartbeat {
        MsgHeartbeat::ServerResponse(self)
    }
}

impl TryInto<MsgHeartbeatRequest> for MsgHeartbeat {
    type Error = ();
    fn try_into(self) -> Result<MsgHeartbeatRequest, ()> {
        match self {
            MsgHeartbeat::Request(r) => Ok(r),
            MsgHeartbeat::ClientResponse(_) => Err(()),
            MsgHeartbeat::ServerResponse(_) => Err(()),
        }
    }
}
impl TryInto<MsgHeartbeatClientResponse> for MsgHeartbeat {
    type Error = ();
    fn try_into(self) -> Result<MsgHeartbeatClientResponse, ()> {
        match self {
            MsgHeartbeat::Request(_) => Err(()),
            MsgHeartbeat::ClientResponse(r) => Ok(r),
            MsgHeartbeat::ServerResponse(_) => Err(()),
        }
    }
}
impl TryInto<MsgHeartbeatServerResponse> for MsgHeartbeat {
    type Error = ();
    fn try_into(self) -> Result<MsgHeartbeatServerResponse, ()> {
        match self {
            MsgHeartbeat::Request(_) => Err(()),
            MsgHeartbeat::ClientResponse(_) => Err(()),
            MsgHeartbeat::ServerResponse(r) => Ok(r),
        }
    }
}

// TODO: Put these in separate submodules

const HEARTBEAT_SLEEP: u64 = 15;
const HEARTBEAT_RANGE: u64 = 30;

// MiniProtocol instance for Heartbeat.
pub(crate) struct Heartbeat {}
impl MiniProtocol for Heartbeat {
    type Message = MsgHeartbeat;

    fn run_server<S: Stream<Self::Message>>(
        self,
        mut stream: S,
    ) -> impl Future<Output = ()> + Send {
        async move {
            debug!("Heartbeat server started!");

            loop {
                let (sleep_time, heartbeat) = {
                    let mut rng = rng();

                    let sleep_time = HEARTBEAT_SLEEP + rng.random_range(0..HEARTBEAT_RANGE);
                    let heartbeat = rng.random();
                    (sleep_time, heartbeat)
                };

                // Sleep
                debug!("Sleeping for {sleep_time} seconds");
                sleep(Duration::new(sleep_time, 0)).await;

                // Send request.
                let server_time = SystemTime::now();
                let req = MsgHeartbeatRequest {
                    server_time,
                    heartbeat,
                };
                debug!("Sending heartbeat: {req:?}");
                send(&mut stream, req).await.expect("TODO");

                // Get response.
                let client_response: MsgHeartbeatClientResponse =
                    receive(&mut stream).await.expect("TODO");
                let latency = server_time.elapsed();
                debug!("Recieved heartbeat response.\nResponse:{client_response:?}\nLatency: {latency:?}");
                if client_response.heartbeat != heartbeat {
                    todo!("Heartbeat does not match");
                }

                // Send response.
                let server_response = MsgHeartbeatServerResponse { heartbeat };
                debug!("Sending response: {server_response:?}");
                send(&mut stream, server_response).await.expect("TODO");
            }
        }
    }

    fn run_client<S: Stream<Self::Message>>(
        self,
        mut stream: S,
    ) -> impl Future<Output = ()> + Send {
        async move {
            debug!("Heartbeat client started!");

            loop {
                // Wait for request.
                let request: MsgHeartbeatRequest = receive(&mut stream).await.expect("TODO");
                debug!("Received heartbeat request.\n{request:?}");

                // Send response.
                let client_time = SystemTime::now();
                let client_response = MsgHeartbeatClientResponse {
                    heartbeat: request.heartbeat,
                    client_time,
                };
                debug!("Sending response.\n{client_response:?}");
                send(&mut stream, client_response).await.expect("TODO");

                // Wait for response.
                let server_response: MsgHeartbeatServerResponse =
                    receive(&mut stream).await.expect("TODO");
                let latency = client_time.elapsed();
                debug!("Received heartbeat response.\n{server_response:?}\nLatency:{latency:?}");
                if server_response.heartbeat != request.heartbeat {
                    todo!("Heartbeat does not match");
                }
            }
        }
    }
}
