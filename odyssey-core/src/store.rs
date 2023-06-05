
use crate::protocol;
use crate::util::Hash;

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
pub struct MetadataHeader<TypeId, Hash> {
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

// TODO: Signature of MetadataHeader by `owner`.

impl<TypeId:AsRef<[u8]>, H: Hash>  MetadataHeader<TypeId, H> {
    /// Compute the store id for the `MetadataHeader`.
    /// This function must be updated any time `MetadataHeader` is updated.
    fn store_id(&self) -> H {
        let h = H::new();
        H::update(&mut h, self.nonce);
        H::update(&mut h, [self.protocol_version.as_byte()]);
        H::update(&mut h, self.store_type);
        H::update(&mut h, self.body_size.to_be_bytes());
        H::update(&mut h, self.body_hash);
        H::finalize(h)
    }
}

pub struct MetadataBody {
    /// Serialized (and encrypted) initial state of the store.
    //  TODO: Eventually merkelize the initial state in chunks.
    initial_state: Vec<u8>,
}

impl MetadataBody {
    pub fn hash<H:Hash>(&self) -> H {
        let h = H::new();
        H::update(&mut h, self.initial_state);
        H::finalize(h)
    }

    /// Validate a `MetadataBody` given its header.
    pub fn validate<TypeId, H:Hash>(&self, header: &MetadataHeader<TypeId, H>) -> bool {
           self.initial_state.len() == header.body_size
        && self.hash::<H>() == header.body_hash
    }
}

// pub type StoreId = [u8; 32];
// pub type TypeId = [u8; 32];
// pub type Hash = [u8; 32];
pub type Nonce = [u8; 32];
