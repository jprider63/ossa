use itertools::Itertools as _;
use ossa_typeable::{TypeId, Typeable};
use serde::{Deserialize, Serialize};
use std::fmt::Debug;
use tracing::warn;

use crate::protocol;
use crate::util::merkle_tree::MerkleTree;
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

/// All block are 2^14 bytes (16KiB) (or less if last block).
pub(crate) const BLOCK_SIZE: u64 = 1 << 14;
/// Limit on the number of merkle nodes a peer can request.
pub(crate) const MERKLE_REQUEST_LIMIT: u64 = 16;
/// Limit on the number of blocks a peer can request.
pub(crate) const BLOCK_REQUEST_LIMIT: u64 = 16;

/// A store's Metadata header.
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub struct MetadataHeader<Hash> {
    /// A random nonce to distinguish the store.
    pub nonce: Nonce,

    /// The protocol version of this store.
    pub protocol_version: protocol::Version,

    /// Type of strongly consistent state that the store holds.
    pub sc_store_type: TypeId,

    /// Type of CRDT state that the store holds.
    pub ec_store_type: TypeId,

    /// Size in bytes of the strongly consistent initial state.
    pub initial_sc_state_size: u64,

    /// Size in bytes of the initial CRDT state.
    pub initial_ec_state_size: u64,

    /// Hash (merkle root) of the hashes of the initial state's blocks.
    /// The initial strongly consistent state is appended to the CRDT state.
    pub merkle_root: Hash, // TODO: Make this an actual binary (or 512-ary) tree?
                           //
                           // TODO:
                           // Owner?
                           // Encryption options
                           // Access control options?
}

// TODO: Signature of MetadataHeader by `owner`.

impl<H: Hash + Debug> MetadataHeader<H> {
    pub fn generate<S: Typeable, T: Typeable>(initial_state: &MetadataBody<H>) -> MetadataHeader<H> {
        let nonce = generate_nonce();
        let protocol_version = protocol::LATEST_VERSION;
        let sc_store_type = S::type_ident();
        let ec_store_type = T::type_ident();
        let initial_sc_state_size = initial_state.initial_sc_state.len() as u64;
        let initial_ec_state_size = initial_state.initial_ec_state.len() as u64;
        let merkle_root = initial_state.merkle_root();
        MetadataHeader {
            nonce,
            protocol_version,
            sc_store_type,
            ec_store_type,
            initial_sc_state_size,
            initial_ec_state_size,
            merkle_root,
        }
    }

    /// Compute the store id for the `MetadataHeader`.
    /// This function must be updated any time `MetadataHeader` is updated.
    pub fn store_id<StoreId>(&self) -> StoreId
    where
        H: Into<StoreId>,
    {
        let mut h = H::new();
        H::update(&mut h, self.nonce);
        H::update(&mut h, [self.protocol_version.as_byte()]);
        H::update(&mut h, self.ec_store_type);
        H::update(&mut h, self.initial_sc_state_size.to_be_bytes());
        H::update(&mut h, self.initial_ec_state_size.to_be_bytes());
        H::update(&mut h, self.merkle_root);
        H::finalize(h).into()
    }

    /// Validate the metadata with respect to the store id.
    pub fn validate_store_id<StoreId: Eq>(&self, store_id: StoreId) -> bool
    where
        H: Into<StoreId>,
    {
        warn!("TODO: Check other properties like upper bounds on constants, etc");
        store_id == self.store_id()
    }

    pub fn block_count(&self) -> u64 {
        self.merkle_size().div_ceil(BLOCK_SIZE as u128).try_into().expect("TODO")
    }

    /// Size of the contents of the merkle tree in bytes.
    pub(crate) fn merkle_size(&self) -> u128 {
        self.initial_ec_state_size as u128 + self.initial_sc_state_size as u128
    }
}

#[derive(Debug)] // , Deserialize, Serialize)]
                 // TODO: Get rid of this? Or rename it? InitialStateBuilder?
pub struct MetadataBody<Hash> {
    /// Serialized (and encrypted) initial strongly consistent state of the store.
    initial_sc_state: Vec<u8>,
    /// Serialized (and encrypted) initial causally consistent state of the store.
    initial_ec_state: Vec<u8>,
    merkle_tree: MerkleTree<Hash>,
}

impl<H: Hash + Debug> MetadataBody<H> {
    pub(crate) fn new<S: Serialize, T: Serialize>(initial_sc_state: &S, initial_ec_state: &T) -> MetadataBody<H> {
        let mut initial_sc_state = serde_cbor::to_vec(initial_sc_state).expect("TODO");
        let mut initial_ec_state = serde_cbor::to_vec(initial_ec_state).expect("TODO");
        // let appended = initial_sc_state.iter().chain(initial_ec_state.iter()).chunks(BLOCK_SIZE as usize);
        initial_sc_state.append(&mut initial_ec_state);
        let merkle_tree = MerkleTree::from_chunks(initial_sc_state.chunks(BLOCK_SIZE as usize));
        MetadataBody {
            initial_sc_state,
            initial_ec_state,
            merkle_tree,
        }
    }

    pub fn merkle_root(&self) -> H {
        self.merkle_tree.merkle_root()
    }

    pub fn build(self) -> (MerkleTree<H>, Vec<u8>) {
        (self.merkle_tree, self.initial_ec_state)
    }
}

// pub type StoreId = [u8; 32];
// pub type TypeId = [u8; 32];
// pub type Hash = [u8; 32];
pub type Nonce = [u8; 32];
