use async_recursion::async_recursion;
use tokio::time::{sleep, Duration};

use crate::network::protocol::keep_alive::v0::{
    KeepAliveError, MsgKeepAliveDone, MsgKeepAliveRequest, MsgKeepAliveResponse,
};
use crate::network::{ConnectionManager, ConnectionStatus};
use crate::util::Stream;

// TODO: Have this utilize the KeepAlive session type.
#[async_recursion]
pub(crate) async fn keep_alive_client<S: Stream>(
    conn: &ConnectionManager<S>,
) -> Result<(), KeepAliveError> {
    // Check if the connection is ending.
    if conn.connection_status().await == ConnectionStatus::Done {
        conn.send(MsgKeepAliveDone {}).await;
        return Ok(());
    }

    // TODO: Read delay in seconds from config
    let delay_s = 5 * 60;
    sleep(Duration::new(delay_s, 0)).await;

    // TODO: Get a random heartbeat
    let heartbeat = 63;

    // Send the keep-alive request.
    conn.send(MsgKeepAliveRequest { heartbeat }).await;

    // Wait for and check the keep-alive response.
    // TODO: Or connection terminated
    let response: MsgKeepAliveResponse = conn.receive().await;
    if heartbeat != response.heartbeat {
        // TODO
        unimplemented!();
    }

    keep_alive_client(conn).await
}
