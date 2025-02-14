use serde::{Deserialize, Serialize};
use typeable::Typeable;
// use typeable::TypeId;

use crate::protocol;
use crate::util::{generate_nonce, Hash};

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
#[derive(Debug, Deserialize, Serialize)]
pub struct MetadataHeader<TypeId, Hash> {
    /// A random nonce to distinguish the store.
    pub nonce: Nonce,

    /// The protocol version of this store.
    pub protocol_version: protocol::Version,

    /// Type of state that the store holds.
    pub store_type: TypeId, // TODO: Switch to typeable::TypeId

    // TODO:
    // Owner?
    // Encryption options
    // Access control options?
    /// Size in bytes of the metadata body.
    pub body_size: usize,

    /// Hash of the metadata body.
    pub body_hash: Hash,
}

// TODO: Signature of MetadataHeader by `owner`.

impl<H: Hash> MetadataHeader<typeable::TypeId, H> {
    pub fn generate<T: Typeable>(initial_state: &MetadataBody) -> MetadataHeader<typeable::TypeId, H> {
        let nonce = generate_nonce();
        let protocol_version = protocol::LATEST_VERSION;
        let store_type = T::type_ident();
        let body_size = initial_state.initial_state.len();
        let body_hash = initial_state.hash();
        MetadataHeader {
            nonce,
            protocol_version,
            store_type,
            body_size,
            body_hash,
        }
    }

    /// Compute the store id for the `MetadataHeader`.
    /// This function must be updated any time `MetadataHeader` is updated.
    pub fn store_id(&self) -> H {
        let mut h = H::new();
        H::update(&mut h, self.nonce);
        H::update(&mut h, [self.protocol_version.as_byte()]);
        H::update(&mut h, self.store_type);
        H::update(&mut h, self.body_size.to_be_bytes());
        H::update(&mut h, &self.body_hash);
        H::finalize(h)
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct MetadataBody {
    /// Serialized (and encrypted) initial state of the store.
    //  TODO: Eventually merkelize the initial state in chunks.
    initial_state: Vec<u8>,
}

impl MetadataBody {
    pub(crate) fn new<T: Serialize>(initial_state: &T) -> MetadataBody {
        let initial_state = serde_cbor::to_vec(initial_state).expect("TODO");
        MetadataBody {
            initial_state,
        }
    }

    pub fn hash<H: Hash>(&self) -> H {
        let mut h = H::new();
        H::update(&mut h, &self.initial_state);
        H::finalize(h)
    }

    /// Validate a `MetadataBody` given its header.
    pub fn validate<TypeId, H: Hash>(&self, header: &MetadataHeader<TypeId, H>) -> bool {
        self.initial_state.len() == header.body_size && self.hash::<H>() == header.body_hash
    }
}

// pub type StoreId = [u8; 32];
// pub type TypeId = [u8; 32];
// pub type Hash = [u8; 32];
pub type Nonce = [u8; 32];
