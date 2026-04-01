use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use tokio::sync::watch;
use hints_bls12381;
use ark_serialize::CanonicalSerialize;
use tracing::{info, warn};

use crate::{auth::DeviceId, protocol::store_bft_sync::v0::{BFTSyncResponse, SignatureId}, store::dag::{self, Frontier}, util::{Hash as _, Sha256Hash}};

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

pub(crate) struct BFTState<StoreId, SHeaderId> {
    // Current round. Starts at 0.
    current_round: Round,
    /// Round states of BFT sync.
    round_states: Vec<RoundState<StoreId, SHeaderId>>,
    /// Tips that are blocks from old rounds (more than 2 old?).
    /// These should all have fully aggregated signatures?
    // JP: Are they all from the previous round?
    previous_tips: Vec<(Round, BlockId)>,
}

impl<StoreId, SHeaderId: std::fmt::Debug> BFTState<StoreId, SHeaderId> {
    pub(crate) fn new(store_id: StoreId) -> Self {
        let round0 = RoundState::new(store_id, 0);
        Self {
            current_round: 0,
            round_states: vec![round0],
            previous_tips: vec![],
        }
    }

    pub(crate) fn current_round(&self) -> Round {
        self.current_round
    }

    pub(crate) fn round_states(&self) -> &[RoundState<StoreId, SHeaderId>] {
        &self.round_states
    }

    /// Get the blocks from the last two rounds and any tips from before that.
    pub(crate) fn get_latest(&self) -> Vec<(Round, BlockId)> {
        let mut latest = self.previous_tips.clone();
        
        if self.current_round >= 1 {
            let prev_round = self.current_round - 1;
            let round_state = &self.round_states[prev_round as usize];
            let blocks = round_state.blocks.keys().map(|block_id| (prev_round, *block_id));
            latest.extend(blocks);
        }

        let round_state = &self.round_states[self.current_round as usize];
        let blocks = round_state.blocks.keys().map(|block_id| (self.current_round, *block_id));
        latest.extend(blocks);

        latest
    }

    pub(crate) fn get_block(&self, round: Round, block_id: &BlockId) -> Option<&Signed<Block<SHeaderId>>> {
        let round_state = self.round_states().get(round as usize)?;
        round_state.blocks.get(block_id)
    }

    pub(crate) fn get_block_signature(&self, round: Round, block_id: BlockId, sig_id: SignatureId) -> Option<Result<ThresholdSignature, (DeviceId, PartialSignature)>> {
        todo!()
    }

    pub(crate) fn get_round_signature(&self, round: Round, sig_id: SignatureId) -> Option<Result<ThresholdSignature, (DeviceId, PartialSignature)>> {
        todo!()
    }

    pub(crate) fn handle_update<S>(&mut self, our_peer_id: &DeviceId, update: BFTSyncResponse<SHeaderId>) -> bool {
        match update {
            BFTSyncResponse::Block(signed) => {
                let frontier = signed.value.dag_frontier;
                // TODO: Check if we know everything in the frontier. Otherwise cache everything remaining.
                warn!("TODO: Check if we know everything in the frontier. Otherwise cache everything remaining.");

                let round = signed.value.round;
                let Some(state) = self.round_states.get_mut(round as usize) else {
                    info!("They sent us a block for a round ({}) that we don't have: {:?}", round, signed);
                    return false;
                };

                // Check if we already know this block.
                let block_id = signed.value.block_id();
                if state.blocks.contains_key(&block_id) {
                    info!("They sent us a block (for round {}) that we already have: {:?}", round, signed);
                    return true;
                }

                // Get the active state for this round.
                let active_state = self.get_active_state_for_round::<S>(round);

                // Validate signer.
                let is_valid_signer = active_state.is_peer_validator(&signed.value.proposer);
                if !is_valid_signer {
                    info!("Proposer cannot sign blocks in round ({}): {:?}", round, signed);
                    return false;
                }

                // Validate signature for signer.
                let signer_key = active_state.get_signing_key(&signed.value.proposer);
                let is_valid_signature = signed.verify(&signer_key);
                if !is_valid_signature {
                    info!("Invalid signed block: {:?}", signed);
                    return false;
                }

                // Check if the peer has already signed a block this round.
                if let Some(other_block_id) = state.block_for_validator.get(&signed.value.proposer) {
                    warn!("TODO: Malicious behavior detected. Peer signed two different blocks this round.\n{:?}\n{:?}", other_block_id, signed);
                    todo!("TODO: Properly handle this.");
                }

                // TODO:
                // Validate block.
                let is_valid_block = self.validate_block(&signed.value); // Likely check that we have all parents (if not the first round)?
                if !is_valid_block {
                    info!("Invalid block: {:?}", signed.value);
                    return false;
                }

                // Add block to state.
                let res = state.blocks.insert(block_id, signed);
                assert!(res.is_none(), "Already checked that we didn't know this block");
                let res = state.block_for_validator.insert(signed.value.proposer, block_id);
                assert!(res.is_none(), "Already checked that the proposer hasn't signed another block");

                // If we're authorized:
                if active_state.is_peer_validator(&our_peer_id) {
                    // Sign block.
                    let certificate = Certificate {
                        store_id,
                        block_id,
                        // block: todo!(),
                        // round,
                        // proposer: todo!(),
                    };
                    let mut signed_block = ThresholdSigned::new(certificate);
                    let is_aggregated = signed_block.sign(&our_threshold_secret_key); // TODO: If certificate is now complete (and it wasn't before), aggregate signature

                    let res = state.certificates.insert(block_id, signed_block);
                    assert!(res.is_none(), "We haven't received any other certificate sigs yet.");

                    // If round is now complete (and it wasn't before), sign round complete.
                    if state.is_round_complete() {
                        state.commit_round.sign(&our_threshold_secret_key)
                    }
                }
            }
            BFTSyncResponse::CertificateSignature(block_id, threshold_signature) => todo!(),
            BFTSyncResponse::CertificatePartialSignature(block_id, device_id, partial_signature) => todo!(),
            BFTSyncResponse::RoundCompleteSignature(round, threshold_signature) => todo!(),
            BFTSyncResponse::RoundCompletePartialSignature(round, device_id, partial_signature) => todo!(),
        }

        true
    }

    fn get_active_state_for_round<S>(&self, round: Round) -> S {
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
#[derive(Clone, Debug, Serialize, Deserialize)]
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

pub(crate) struct RoundState<StoreId, SHeaderId> {
    // Blocks in this round for each validator.
    // A validator can only sign a single block in each round (otherwise, they are detected to be malicious).
    blocks: BTreeMap<BlockId, Signed<Block<SHeaderId>>>,
    // Used to quickly check if a peer already signed a block. 
    // JP: Maybe this should be transient?
    block_for_validator: BTreeMap<DeviceId, BlockId>,
    // 2/3 (?) of (weighted) validators promise to make the block available and validated/approve of operations.
    certificates: BTreeMap<BlockId, ThresholdSigned<Certificate>>,
    // 2/3 (1/3?) of (weighted) validators have seen 2/3 (?) of the certificates.
    commit_round: ThresholdSigned<RoundComplete<StoreId>>,
}

impl<StoreId, SHeaderId> RoundState<StoreId, SHeaderId> {
    pub(crate) fn new(store_id: StoreId, round: Round) -> Self {
        let blocks = BTreeMap::new();
        let block_for_validator = BTreeMap::new();
        let certificates = BTreeMap::new();
        let commit_round = ThresholdSigned::new(RoundComplete {
            store_id,
            round,
        });
        Self {
            blocks,
            block_for_validator,
            certificates,
            commit_round
        }
    }

    pub(crate) fn commit_round(&self) -> &ThresholdSigned<RoundComplete<StoreId>> {
        &self.commit_round
    }

    pub(crate) fn blocks(&self) -> &BTreeMap<BlockId, Signed<Block<SHeaderId>>> {
        &self.blocks
    }

    pub(crate) fn certificates(&self) -> &BTreeMap<BlockId, ThresholdSigned<Certificate>> {
        &self.certificates
    }
}

#[derive(Debug, Clone, Copy, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub(crate) struct BlockId(Sha256Hash);

// A BFT block points to the tips of the DAG and the previous round's certificates.
#[derive(Clone, Debug, Serialize)]
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

#[derive(Debug)]
pub(crate) struct ThresholdSignature(hints_bls12381::hints::ThresholdSignature);
impl ThresholdSignature {
    fn signature_id(&self) -> SignatureId {
        type H = Sha256Hash;

        let mut h = H::new();
        // JP: Should we use HashMarshaller here (CanonicalSerializeHashExt)?
        self.0.serialize_compressed(&mut h).expect("Failed to hash threshold signature");
        SignatureId(H::finalize(h))
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct PartialSignature(hints_bls12381::hints::PartialSignature);
impl PartialSignature {
    fn signature_id(&self, signer: &DeviceId) -> SignatureId {
        type H = Sha256Hash;

        let mut h = H::new();
        H::update(&mut h, signer);
        self.0.serialize_compressed(&mut h).expect("Failed to hash partial signature");
        SignatureId(H::finalize(h))
    }
}

impl<'de> Deserialize<'de> for ThresholdSignature {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de> {
        todo!("TODO: Implement this. Arkworks provides CanonicalDeserialize.")
    }
}

impl Serialize for PartialSignature {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer {
        todo!("TODO: Implement this. Arkworks provides CanonicalSerialize.")
    }
}

impl<'de> Deserialize<'de> for PartialSignature {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de> {
        todo!("TODO: Implement this. Arkworks provides CanonicalDeserialize.")
    }
}

impl Serialize for ThresholdSignature {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer {
        todo!("TODO: Implement this. Arkworks provides CanonicalSerialize.")
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub(crate) struct CertificateId(Sha256Hash);

// JP: Do we need this type? Just use the block id?
pub(crate) struct Certificate {
    /// The block's id (hash).
    block_id: BlockId,
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
        match self {
            ThresholdSigned::ThresholdSignature { value: _, signature } => vec![signature.signature_id()],
            ThresholdSigned::PartialSignatures { value: _, signatures } => signatures.iter().map(|s| s.1.signature_id(s.0)).collect()
        }
    }

    fn new(value: A) -> Self {
        ThresholdSigned::PartialSignatures {
            value,
            signatures: BTreeMap::new(),
        }
    }
}

pub(crate) struct RoundComplete<StoreId> {
    store_id: StoreId,
    round: Round,
}
// JP: Could include hash of previous round's leader block?? Probably not helpful

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_hash_of_partial_sig_distinct() {
        use ark_ec::AffineRepr;
        let i = hints_bls12381::hints::PartialSignature::generator();
        let i_ = (i + i).into();
        let public_key_bytes = [
           215,  90, 152,   1, 130, 177,  10, 183, 213,  75, 254, 211, 201, 100,   7,  58,
            14, 225, 114, 243, 218, 166,  35,  37, 175,   2,  26, 104, 247,   7,   81, 26];
        let auth_key = ed25519_dalek::VerifyingKey::from_bytes(&public_key_bytes).unwrap();
        let peer = DeviceId::new(auth_key);
        // println!("{}", i);
        // println!("{}", i_);
        let s0 = PartialSignature(i);
        let s1 = PartialSignature(i_);
        let sid0 = s0.signature_id(&peer);
        let sid1 = s1.signature_id(&peer);
        // println!("{}", s0.signature_id(&peer));
        // println!("{}", s1.signature_id(&peer));
        assert!(sid0 != sid1);
    }
}
