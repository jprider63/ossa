use std::collections::{BTreeMap, BTreeSet};

use crate::{auth::DeviceId, store::dag::{self, Frontier}, util::Sha256Hash};

/// A round in the BFT strong consistency protocol.
pub type Round = u64;

/// Trait that abstracts over strongly consistent data types that require linearizability.
pub trait SCDT {
    type Op;

    fn update(self, op: Self::Op) -> Self;

    fn is_valid_operation(&self, op: Self::Op) -> bool;
}

pub(crate) struct State<Header: dag::DAGHeader, S> {
    pub(crate) initial_state: S, // JP: Should this go somewhere else? Potentially `DecryptedState`?
    pub(crate) dag_state: dag::State<Header, S>,
    /// Frontier of operations that have been comitted in the DAG state.
    pub(crate) committed_frontier: Frontier<Header::HeaderId>,
    pub(crate) round_states: Vec<RoundState<Header>>,
}

impl<Header: dag::DAGHeader, S> State<Header, S> {
    pub(crate) fn new(initial_state: S) -> Self {
        Self {
            initial_state,
            dag_state: dag::State::new(),
            committed_frontier: BTreeSet::new(),
            round_states: vec![],
        }
    }
}

// A signed value.
pub struct Signed<A> {
    value: A,
    // JP: Generalize this eventually.
    signature: ed25519_dalek::Signature,
}

pub(crate) struct RoundState<SHeader: dag::DAGHeader> {
    // Blocks in this round for each validator.
    // A validator can only sign a single block in each round (otherwise, they are detected to be malicious).
    blocks: BTreeMap<DeviceId, Signed<Block<SHeader>>>,
    // 2/3 (?) of (weighted) validators promise to make the block available and validated/approve of operations.
    certificates: BTreeMap<BlockId, ThresholdSigned<Certificate>>,
    // 2/3 (1/3?) of (weighted) validators have seen 2/3 (?) of the certificates.
    commit_round: ThresholdSigned<RoundComplete>,
}

pub(crate) struct BlockId(Sha256Hash);

// A BFT block points to the tips of the DAG and the previous round's certificates.
pub(crate) struct Block<SHeader: dag::DAGHeader> {
    round: u64,
    // Tips of DAG operations
    dag_frontier: Frontier<SHeader::HeaderId>,
    // Must contain 2/3 of previous round's certificates (or be round 0).
    parents: Vec<CertificateId>, // Only strong edges, don't need weak edges since blocks point to head of DAG operations anyways
    // Who proposed the block.
    proposer: DeviceId,
}

pub(crate) struct ThresholdSignature(); // TODO
pub(crate) struct PartialSignature(); // TODO

pub(crate) struct CertificateId(Sha256Hash);

pub(crate) struct Certificate {
    // The block's id (hash).
    block: BlockId,
    // The block's round.
    round: u64,
    // Who proposed the block.
    proposer: DeviceId,
}

// A threshold signed value (or being signed).
pub(crate) enum ThresholdSigned<A> {
    // Weighted threshold of validators have signed the value.
    ThresholdSignature {
        value: A,
        signature: ThresholdSignature, 
    },
    // Weighted threshold hasn't been met yet.
    PartialSignatures {
        value: A,
        signatures: BTreeMap<DeviceId, PartialSignature>,
    },
}

pub(crate) struct RoundComplete {
    store_id: Sha256Hash, // StoreId,
    round: u64,
}
// JP: Could include hash of previous round's leader block?? Probably not helpful

