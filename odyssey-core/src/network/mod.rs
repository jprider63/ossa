pub mod multiplexer;
pub mod protocol;

use std::fmt::Debug;

use crate::network::protocol::{receive, send};
use crate::util::Stream;

// Manage a connection with a peer.
// TODO: Delete ConnectionManager.
pub struct ConnectionManager<S> {
    // }:Stream> {
    connection: S,
}

impl<S> ConnectionManager<S> {
    pub fn new(connection: S) -> ConnectionManager<S> {
        ConnectionManager { connection }
    }
    /// Retrieve the connection status.
    pub async fn connection_status(&self) -> ConnectionStatus {
        ConnectionStatus::Active
    }

    pub async fn send<T, U>(&mut self, val: U)
    where
        S: Stream<T>,
        U: Into<T>,
    {
        send(&mut self.connection, val).await.expect("TODO")
    }

    pub async fn receive<T, U>(&mut self) -> U
    where
        S: Stream<T>,
        T: TryInto<U>,
        U: Debug,
    {
        receive(&mut self.connection).await.expect("TODO")
    }
}

#[derive(PartialEq)]
pub enum ConnectionStatus {
    Active,
    Done,
}
