pub mod p2p;
pub mod protocol;

use crate::store;
use crate::util::Stream;

pub struct ManagerSettings {
}

/// Network manager.
pub struct Manager {
    settings: ManagerSettings,
    // Store peer info and store metadata/raw data?
}

impl Manager {
    pub fn initialize(settings: ManagerSettings) -> Manager {
        // Start listening on port.
        // Start DHT.

        Manager {
            settings
        }
    }

    // 
    pub fn createAndConnectToStore<TypeId, H>(store_metadata: store::MetadataHeader<TypeId, H>) -> Result<(), String> { // TODO: async API that pushes errors, applied operations, connection/peer info, etc to a queue?
        unimplemented!{}
    }

    pub fn connectToStoreById<Id>(store_metadata: Id) -> Result<(), String> { // TODO: async API that pushes errors to a queue?
        // TODO: 
        // Lookup id on DHT to retrieve peers
        // Manage peers
        // Retrieve MetadataHeader if we don't have it.
        //   Validate MetadataHeader (matches id, T's invariants, etc)
        // Sync any data
        // Propagate that data asyncronously
        // Store any updates to the file system
        unimplemented!{}
    }
}

// Manage a connection with a peer.
pub struct ConnectionManager<S> { // }:Stream> {
    connection: S,
}

impl<S> ConnectionManager<S> {
    pub fn new(connection: S) -> ConnectionManager<S> {
        ConnectionManager {
            connection,
        }
    }
    /// Retrieve the connection status.
    pub async fn connection_status(&self) -> ConnectionStatus {
        ConnectionStatus::Active
    }

    pub async fn send<T>(&self, val: T) {
        println!("TODO: send");
    }

    pub async fn receive<T>(&self) -> T {
        unimplemented!();
    }
}

#[derive(PartialEq)]
pub enum ConnectionStatus {
    Active,
    Done,
}

