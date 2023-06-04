
/// Network manager.
pub struct Manager {
    // Store peer info and store metadata/raw data?
}

impl Manager {
    pub fn initialize(settings: ManagerSettings) -> Manager {
        // Start listening on port.
        // Store settings.

        Manager {
        }
    }

    // 
    pub fn connectToStore<T>(store_metadata: MetadataHeader<T>) -> Either<String, ()> { // TODO: async API that pushes errors, applied operations, connection/peer info, etc to a queue?
        unimplemented!{}
    }

    pub fn connectToStoreById<Id>(store_metadata: Id) -> Either<String, ()> { // TODO: async API that pushes errors to a queue?
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


// Network level protocols between peers
//
// TODO: Do we need to do a handshake to establish a MAC key (is an encryption needed)?.
//
// pub struct RequestStoreMetadataHeaderV0 {
//   store_id: store::Id,
//   body_request: RequestStoreMetadataBodyV0,
// }
//
// pub struct RequestStoreMetadataBodyV0 {
// }
//
// request_store_metadata_header :: RequestStoreMetadataHeaderV0 -> Either<ProtocolError, ResponseStoreMetadataHeaderV0>
