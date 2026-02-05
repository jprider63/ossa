use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use tokio::sync::watch;

use crate::{auth::DeviceId, protocol::store_bft_sync::v0::{SignatureId, ThresholdSignatureId}, store::dag::{self, Frontier}, util::Sha256Hash};

/// A round in the BFT strong consistency protocol.
pub type Round = u64;

/// Trait that abstracts over strongly consistent data types that require linearizability.
pub trait SCDT {
    type Op;

    fn update(self, op: Self::Op) -> Self;

    fn is_valid_operation(&self, op: Self::Op) -> bool;
}

pub(crate) struct State<SHeader: dag::DAGHeader, S> {
    pub(crate) initial_state: S, // JP: Should this go somewhere else? Potentially `DecryptedState`?
    pub(crate) dag_state: dag::State<SHeader, S>,
    /// Frontier of operations that have been committed in the DAG state.
    pub(crate) committed_frontier: Frontier<SHeader::HeaderId>,
}

pub(crate) struct BFTState<SHeaderId> {
    // Current round. Starts at 0.
    current_round: Round,
    /// Round states of BFT sync.
    round_states: Vec<RoundState<SHeaderId>>,
    /// Tips that are blocks (signed by the given peer) from previous rounds. These should all have fully aggregated signatures.
    // JP: Are they all from the previous round?
    previous_tips: Vec<(Round, DeviceId)>,
}

impl<SHeaderId> BFTState<SHeaderId> {
    pub(crate) fn new() -> Self {
        Self {
            current_round: 0,
            round_states: vec![],
            previous_tips: vec![],
        }
    }

    pub(crate) fn current_round(&self) -> u64 {
        self.current_round
    }

    pub(crate) fn round_states(&self) -> &[RoundState<SHeaderId>] {
        &self.round_states
    }

    pub(crate) fn previous_tips(&self) -> &[(Round, DeviceId)] {
        &self.previous_tips
    }

    pub(crate) fn get_current_tips(&self) -> &[(Round, DeviceId)] {
        // Get all active tips
        todo!()
    }
}

impl<Header: dag::DAGHeader, S> State<Header, S> {
    pub(crate) fn new(initial_state: S) -> Self {
        Self {
            initial_state,
            dag_state: dag::State::new(),
            committed_frontier: BTreeSet::new(),
        }
    }
}

// A signed value.
pub struct Signed<A> {
    value: A,
    // JP: Generalize this eventually.
    signature: ed25519_dalek::Signature,
}

impl<A> Signed<A> {
    pub fn value(&self) -> &A {
        &self.value
    }
}

pub(crate) struct RoundState<SHeaderId> {
    // Blocks in this round for each validator.
    // A validator can only sign a single block in each round (otherwise, they are detected to be malicious).
    blocks: BTreeMap<DeviceId, Signed<Block<SHeaderId>>>,
    // 2/3 (?) of (weighted) validators promise to make the block available and validated/approve of operations.
    certificates: BTreeMap<BlockId, ThresholdSigned<Certificate>>,
    // 2/3 (1/3?) of (weighted) validators have seen 2/3 (?) of the certificates.
    commit_round: ThresholdSigned<RoundComplete>,
}

impl<SHeaderId> RoundState<SHeaderId> {
    pub(crate) fn commit_round(&self) -> &ThresholdSigned<RoundComplete> {
        &self.commit_round
    }

    pub(crate) fn blocks(&self) -> &BTreeMap<DeviceId, Signed<Block<SHeaderId>>> {
        &self.blocks
    }

    pub(crate) fn certificates(&self) -> &BTreeMap<BlockId, ThresholdSigned<Certificate>> {
        &self.certificates
    }
}

#[derive(Debug, Clone, Copy, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub(crate) struct BlockId(Sha256Hash);

// A BFT block points to the tips of the DAG and the previous round's certificates.
#[derive(Debug, Serialize)]
pub(crate) struct Block<SHeaderId> {
    round: Round,
    // Tips of SC DAG operations
    dag_frontier: Frontier<SHeaderId>,
    // Must contain 2/3 of previous round's certificates (or be round 0).
    parents: Vec<CertificateId>, // Only strong edges, don't need weak edges since blocks point to head of DAG operations anyways
    // Who proposed the block.
    proposer: DeviceId,
}

impl<SHeaderId> Block<SHeaderId> {
    pub fn block_id(&self) -> BlockId {
        todo!()
    }
}

impl<'de, SHeaderId> Deserialize<'de> for Block<SHeaderId> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de> {
        todo!("TODO: Manually implement this since derive is broken for generics")
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct ThresholdSignature(); // TODO
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct PartialSignature(); // TODO

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct CertificateId(Sha256Hash);

// JP: Do we need this type? Just use the block id?
pub(crate) struct Certificate {
    /// The block's id (hash).
    block: BlockId,
    /// The block's round.
    // JP: Is this needed?
    round: Round,
    /// Who proposed the block.
    // JP: Is this needed?
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

impl<A> ThresholdSigned<A> {
    // pub fn signature_ids(&self) -> ThresholdSignatureId {
    pub fn signature_ids(&self) -> Vec<SignatureId> {
        todo!()
    }
}

pub(crate) struct RoundComplete {
    store_id: Sha256Hash, // StoreId,
    round: Round,
}
// JP: Could include hash of previous round's leader block?? Probably not helpful

