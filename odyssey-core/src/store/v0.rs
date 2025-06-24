use serde::{Deserialize, Serialize};
use tracing::warn;
use typeable::{Typeable, TypeId};

use crate::{protocol, util};
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

/// All pieces are 2^14 bytes (16KiB).
pub(crate) const PIECE_SIZE: u64 = 1 << 14;
/// Limit on the number of merkle nodes a peer can request.
pub(crate) const MERKLE_REQUEST_LIMIT: u64 = 16;

/// A store's Metadata header.
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub struct MetadataHeader<Hash> {
    /// A random nonce to distinguish the store.
    pub nonce: Nonce,

    /// The protocol version of this store.
    pub protocol_version: protocol::Version,

    /// Type of state that the store holds.
    pub store_type: TypeId,

    // TODO:
    // Owner?
    // Encryption options
    // Access control options?
    /// Size in bytes of the initial state.
    pub initial_state_size: u64,

    /// Hash (merkle root) of the hashes of the initial state's pieces.
    pub merkle_root: Hash, // TODO: Make this an actual binary (or 512-ary) tree?
}

// TODO: Signature of MetadataHeader by `owner`.

impl<H: Hash> MetadataHeader<H> {
    pub fn generate<T: Typeable>(initial_state: &MetadataBody<H>) -> MetadataHeader<H> {
        let nonce = generate_nonce();
        let protocol_version = protocol::LATEST_VERSION;
        let store_type = T::type_ident();
        let initial_state_size = initial_state.initial_state.len() as u64;
        let body_hash = initial_state.merkle_root();
        MetadataHeader {
            nonce,
            protocol_version,
            store_type,
            initial_state_size,
            merkle_root: body_hash,
        }
    }

    /// Compute the store id for the `MetadataHeader`.
    /// This function must be updated any time `MetadataHeader` is updated.
    pub fn store_id(&self) -> H {
        let mut h = H::new();
        H::update(&mut h, self.nonce);
        H::update(&mut h, [self.protocol_version.as_byte()]);
        H::update(&mut h, self.store_type);
        H::update(&mut h, self.initial_state_size.to_be_bytes());
        H::update(&mut h, self.merkle_root);
        H::finalize(h)
    }

    /// Validate the metadata with respect to the store id.
    pub fn validate_store_id(&self, store_id: H) -> bool {
        warn!("TODO: Check other properties like upper bounds on constants, etc");
        store_id == self.store_id()
    }

    pub fn piece_count(&self) -> u64 {
        self.initial_state_size.div_ceil(PIECE_SIZE)
    }
}

#[derive(Debug, Deserialize, Serialize)]
// TODO: Get rid of this? Or rename it? InitialStateBuilder?
pub struct MetadataBody<Hash> {
    /// Serialized (and encrypted) initial state of the store.
    //  TODO: Eventually merkelize the initial state in chunks.
    initial_state: Vec<u8>,
    piece_size: u32, // TODO: Make this constant a 16 KB (2^14), 
    piece_hashes: Vec<Hash>,
}

impl<H: Hash> MetadataBody<H> {
    pub(crate) fn new<T: Serialize>(initial_state: &T) -> MetadataBody<H> {
        let initial_state = serde_cbor::to_vec(initial_state).expect("TODO");
        let piece_size = 1 << 18;
        let piece_hashes = initial_state.chunks(piece_size as usize).map(|piece| {
            let mut h = H::new();
            H::update(&mut h, piece);
            H::finalize(h)
        }).collect();
        MetadataBody {
            initial_state,
            piece_size,
            piece_hashes,
        }
    }

    pub fn merkle_root(&self) -> H {
        util::merkle_root(&self.piece_hashes)
    }

    /// Validate a `MetadataBody` given its header.
    pub fn validate<TypeId>(&self, header: &MetadataHeader<H>) -> bool {
        panic!("TODO: Delete me?");

        // self.initial_state.len() == header.initial_state_size && self.hash::<H>() == header.merkle_root
    }

    pub fn build(self) -> (Vec<H>, Vec<u8>) {
        (self.piece_hashes, self.initial_state)
    }
}

// pub type StoreId = [u8; 32];
// pub type TypeId = [u8; 32];
// pub type Hash = [u8; 32];
pub type Nonce = [u8; 32];
