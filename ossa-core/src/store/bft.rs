use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::{auth::DeviceId, store::dag::{self, Frontier}, util::Sha256Hash};

/// A round in the BFT strong consistency protocol.
pub type Round = u64;

/// A partial threshold signature from a single validator.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PartialSignature(pub Vec<u8>); // TODO: Use actual threshold signature type

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

/// A signed value.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Signed<A> {
    pub value: A,
    // JP: Generalize this eventually.
    #[serde(with = "ed25519_signature_serde")]
    pub signature: ed25519_dalek::Signature,
}

/// Serde support for ed25519 signatures.
mod ed25519_signature_serde {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S>(sig: &ed25519_dalek::Signature, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        sig.to_bytes().serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<ed25519_dalek::Signature, D::Error>
    where
        D: Deserializer<'de>,
    {
        let bytes = <[u8; 64]>::deserialize(deserializer)?;
        ed25519_dalek::Signature::from_bytes(&bytes).map_err(serde::de::Error::custom)
    }
}

pub struct RoundState<SHeader: dag::DAGHeader> {
    /// Blocks in this round for each validator.
    /// A validator can only sign a single block in each round (otherwise, they are detected to be malicious).
    pub blocks: BTreeMap<DeviceId, Signed<Block<SHeader>>>,
    /// 2/3 (?) of (weighted) validators promise to make the block available and validated/approve of operations.
    pub certificates: BTreeMap<BlockId, ThresholdSigned<Certificate>>,
    /// 2/3 (1/3?) of (weighted) validators have seen 2/3 (?) of the certificates.
    pub commit_round: ThresholdSigned<RoundComplete>,
}

/// Unique identifier for a block (hash of block contents).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct BlockId(pub Sha256Hash);

impl Default for BlockId {
    fn default() -> Self {
        BlockId(Sha256Hash::default())
    }
}

/// A BFT block points to the tips of the DAG and the previous round's certificates.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Block<SHeader: dag::DAGHeader> {
    pub round: u64,
    /// Tips of DAG operations.
    pub dag_frontier: Frontier<SHeader::HeaderId>,
    /// Must contain 2/3 of previous round's certificates (or be round 0).
    /// Only strong edges - don't need weak edges since blocks point to head of DAG operations anyways.
    pub parents: Vec<CertificateId>,
    /// Who proposed the block.
    pub proposer: DeviceId,
}

/// A complete threshold signature (threshold has been met).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThresholdSignature(pub Vec<u8>); // TODO: Use actual threshold signature type

/// Unique identifier for a certificate (hash of certificate contents).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct CertificateId(pub Sha256Hash);

impl Default for CertificateId {
    fn default() -> Self {
        CertificateId(Sha256Hash::default())
    }
}

/// A certificate proving a block's availability (threshold signed by validators).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Certificate {
    /// The block's id (hash).
    pub block: BlockId,
    /// The block's round.
    pub round: u64,
    /// Who proposed the block.
    pub proposer: DeviceId,
}

/// A threshold signed value (or being signed).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ThresholdSigned<A> {
    /// Weighted threshold of validators have signed the value.
    ThresholdSignature {
        value: A,
        signature: ThresholdSignature,
    },
    /// Weighted threshold hasn't been met yet.
    PartialSignatures {
        value: A,
        signatures: BTreeMap<DeviceId, PartialSignature>,
    },
}

impl<A: Default> Default for ThresholdSigned<A> {
    fn default() -> Self {
        ThresholdSigned::PartialSignatures {
            value: A::default(),
            signatures: BTreeMap::new(),
        }
    }
}

/// Proof that a round is complete (all parties have seen enough certificates).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RoundComplete {
    pub store_id: Sha256Hash, // StoreId,
    pub round: u64,
}
// JP: Could include hash of previous round's leader block?? Probably not helpful

