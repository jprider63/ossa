
use sha2::{Digest, Sha256};

use crate::protocol;

// pub struct Store<Id, T> {
//     id: Id,
//     state: T,
//     metadata: Metadata<T>,
// }
// 
// impl<Id, T: Clone> Store<Id, T> {
//     pub fn create_new(initial_state:T) -> Store<Id, T> {
//         let meta = Metadata {
//             initial_state: initial_state.clone(),
//             nonce: crate::util::generate_nonce(),
//             protocol_version: protocol::LATEST_VERSION,
//         };
//         let id = unimplemented!();
//         Store {
//             id,
//             state: initial_state,
//             metadata: meta,
//         };
//     }
// }
// 
// /// Metadata for a store.
// pub struct Metadata<T> {
//     protocol_version: protocol::Version,
//     nonce: Nonce,
//     initial_state: T,
// }

/// A store's Metadata header.
pub struct MetadataHeader {
    /// A random nonce to distinguish the store.
    nonce: Nonce,

    /// The protocol version of this store.
    protocol_version: protocol::Version,

    /// Type of state that the store holds.
    store_type: TypeId,

    // TODO:
    // Owner?
    // Encryption options
    // Access control options?

    /// Size in bytes of the metadata body.
    body_size: usize,

    /// Hash of the metadata body.
    body_hash: Hash,
}

impl MetadataHeader {
    /// Compute the `StoreId` for the `MetadataHeader`.
    /// This fucntion must be updated any time `MetadataHeader` is updated.
    fn store_id(&self) -> StoreId {
        let h = Sha256::new();
        h.update(self.nonce);
        h.update([self.protocol_version.as_byte()]);
        h.update(self.store_type);
        h.update(self.body_size.to_be_bytes());
        h.update(self.body_hash);
        h.finalize().into()
    }
}

pub struct MetadataBody {
    /// Serialized (and encrypted) initial state of the store.
    //  TODO: Eventually merkelize the initial state in chunks.
    initial_state: Vec<u8>,
}

impl MetadataBody {
    pub fn hash(&self) -> Hash {
        let h = Sha256::new();
        h.update(self.initial_state);
        h.finalize().into()
    }

    /// Validate a `MetadataBody` given its header.
    pub fn validate(&self, header: &MetadataHeader) -> bool {
           self.initial_state.len() == header.body_size
        && self.hash() == header.body_hash
    }
}

pub type StoreId = [u8; 32];
pub type TypeId = [u8; 32];
pub type Hash = [u8; 32];
pub type Nonce = [u8; 32];

