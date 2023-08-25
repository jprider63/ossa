
use async_recursion::async_recursion;

use crate::network::protocol::keep_alive::v0::{MsgKeepAliveRequest, MsgKeepAliveResponse, MsgKeepAliveDone,KeepAliveError};
use crate::network::{ConnectionManager, ConnectionStatus};

// TODO: Have this utilize the KeepAlive session type.
#[async_recursion]
pub(crate) async fn keep_alive_server(conn: &ConnectionManager) -> Result<(), KeepAliveError> {
    // Check if the connection is ending.
    if conn.connection_status().await == ConnectionStatus::Done {
        conn.send(MsgKeepAliveDone {}).await;
        return Ok(());
    }

    // Wait for keep-alive request.
    // TODO: Or connection terminated
    let MsgKeepAliveRequest {heartbeat} = conn.receive().await;

    conn.send(MsgKeepAliveResponse {heartbeat}).await;

    keep_alive_server(conn).await
}
