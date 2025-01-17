
use bytes::BytesMut;
use rand::{thread_rng, Rng};
use tokio::time::{Duration, Instant, sleep};

use crate::{
    network::protocol::{receive, send},
    util::{Channel, Stream},
};

/// TODO:
/// The session type for the ecg-sync protocol.
// pub type Heartbeat = Send<(), Eps>; // TODO

// server to client: (Time, u64)
// client to server: (Time, u64, Time)
// server to client: (u64, Time)

#[derive(Debug)]
enum MsgHeartbeat {
    Request(MsgHeartbeatRequest),
    ClientResponse(MsgHeartbeatClientResponse),
    ServerResponse(MsgHeartbeatServerResponse),
}

#[derive(Debug)]
struct MsgHeartbeatRequest {
    server_time: Instant,
    heartbeat: u64,
}
#[derive(Debug)]
struct MsgHeartbeatClientResponse {
    heartbeat: u64,
    client_time: Instant,
}
#[derive(Debug)]
struct MsgHeartbeatServerResponse {
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

// TODO: Abstract this away
pub(crate) async fn run_server_wrapper(channel: Channel<BytesMut>) {
    // TODO: Convert the channel to a T: Stream<MsgHeartbeat>
    // Serialize/deserialize byte channel
    let stream: Channel<MsgHeartbeat> = todo!();

    run_server_protocol(stream).await;
}

async fn run_server_protocol<S: Stream<MsgHeartbeat>>(mut stream: S) {
    let mut rng = thread_rng();

    loop {
        // Sleep
        let sleep_time = HEARTBEAT_SLEEP + rng.gen_range(0..HEARTBEAT_RANGE);
        println!("Sleeping for {sleep_time} seconds");
        sleep(Duration::new(sleep_time, 0)).await;

        // Send request.
        let heartbeat = rng.gen();
        let server_time = Instant::now();
        let req = MsgHeartbeatRequest {
            server_time,
            heartbeat,
        };
        println!("Sending heartbeat: {req:?}");
        send(&mut stream, req).await;

        // Get response.
        let client_response: MsgHeartbeatClientResponse = receive(&mut stream).await.expect("TODO");
        let latency = server_time.elapsed();
        println!("Recieved heartbeat response.\nResponse:{client_response:?}\nLatency: {latency:?}");
        if client_response.heartbeat != heartbeat {
            todo!("Heartbeat does not match");
        }

        // Send response.
        let server_response = MsgHeartbeatServerResponse {
            heartbeat,
        };
        println!("Sending response: {server_response:?}");
        send(&mut stream, server_response).await;
    }
}

// TODO: Abstract this away
pub(crate) async fn run_client_wrapper(channel: Channel<BytesMut>) {
    // TODO: Convert the channel to a T: Stream<MsgHeartbeat>
    // Serialize/deserialize byte channel
    let stream: Channel<MsgHeartbeat> = todo!();

    run_client_protocol(stream).await
}

async fn run_client_protocol<S: Stream<MsgHeartbeat>>(mut stream: S) {
    loop {
        // Wait for request.
        let request: MsgHeartbeatRequest = receive(&mut stream).await.expect("TODO");

        // Send response.
        let client_time = Instant::now();
        let client_response = MsgHeartbeatClientResponse {
            heartbeat: request.heartbeat,
            client_time,
        };

        // Wait for response.
        let server_response: MsgHeartbeatServerResponse = receive(&mut stream).await.expect("TODO");
        let latency = client_time.elapsed();
        println!("Received heartbeat response.\n{server_response:?}\nLatency:{latency:?}");
        if server_response.heartbeat != request.heartbeat {
            todo!("Heartbeat does not match");
        }
    }
}
