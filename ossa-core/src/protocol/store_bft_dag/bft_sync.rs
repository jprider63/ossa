//! BFT Synchronization Protocol
//!
//! This module implements the peer synchronization protocol for the asynchronous
//! Byzantine fault-tolerant consensus layer. It enables parties to:
//! - Catch up to the current committed state
//! - Receive live updates as new blocks/certificates are created
//! - Collect partial signatures for threshold aggregation

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Debug;
use std::marker::PhantomData;

use serde::{Deserialize, Serialize};

use crate::auth::DeviceId;
use crate::store::bft::{
    self, BlockId, Certificate, CertificateId, PartialSignature, Round, RoundComplete, Signed,
    ThresholdSigned,
};
use crate::store::dag::{self, DAGHeader, Frontier};
use crate::util::Sha256Hash;

/// Maximum rounds to request in a single message.
pub const MAX_ROUNDS_REQUEST: u16 = 32;

/// Maximum blocks to return in a single response.
pub const MAX_BLOCKS_RESPONSE: u16 = 64;

/// Maximum certificates to return in a single response.
pub const MAX_CERTS_RESPONSE: u16 = 64;

/// Threshold for switching to fast sync (snapshot-based).
pub const FAST_SYNC_THRESHOLD: u64 = 100;

/// Timeout for sync requests in milliseconds.
pub const SYNC_REQUEST_TIMEOUT_MS: u64 = 5000;

/// Maximum pending partial signatures to track.
pub const MAX_PENDING_PARTIAL_SIGS: usize = 1000;

// ============================================================================
// Sync Request Messages
// ============================================================================

/// Requests sent by the initiator (syncing party) to the responder.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MsgBFTSyncRequest<SHeaderId> {
    /// Request current state summary from peer.
    StatusRequest,

    /// Request blocks for specific rounds.
    BlocksRequest {
        /// Rounds to fetch blocks for.
        rounds: Vec<Round>,
        /// Optional filter by proposers. None means all proposers.
        proposers: Option<Vec<DeviceId>>,
    },

    /// Request certificates for specific blocks.
    CertificatesRequest {
        /// Block IDs to fetch certificates for.
        block_ids: Vec<BlockId>,
    },

    /// Request round completion proofs.
    RoundCompleteRequest {
        /// Rounds to fetch completion proofs for.
        rounds: Vec<Round>,
    },

    /// Request partial signatures for pending items.
    PartialSignaturesRequest {
        /// Request partial signatures for these certificate block IDs.
        certificate_block_ids: Vec<BlockId>,
        /// Request partial signatures for these round completion rounds.
        round_completions: Vec<Round>,
    },

    /// Request missing DAG operations that blocks reference.
    DAGOperationsRequest {
        /// DAG header IDs that we need.
        header_ids: Vec<SHeaderId>,
    },

    /// Subscribe to live updates starting from a round.
    Subscribe {
        /// Round to start receiving updates from.
        from_round: Round,
    },

    /// Unsubscribe from live updates.
    Unsubscribe,
}

// ============================================================================
// Sync Response Messages
// ============================================================================

/// Responses sent by the responder to the initiator.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MsgBFTSyncResponse<SHeader: DAGHeader> {
    /// Status response with summary of peer's BFT state.
    Status(BFTStatus),

    /// Blocks response.
    Blocks {
        blocks: Vec<Signed<bft::Block<SHeader>>>,
    },

    /// Certificates response.
    Certificates {
        certificates: Vec<ThresholdSigned<Certificate>>,
    },

    /// Round completion proofs.
    RoundComplete {
        proofs: Vec<ThresholdSigned<RoundComplete>>,
    },

    /// Partial signatures response.
    PartialSignatures {
        /// Partial signatures for certificates: (BlockId, Signer, Signature).
        certificate_sigs: Vec<(BlockId, DeviceId, PartialSignature)>,
        /// Partial signatures for round completions: (Round, Signer, Signature).
        round_complete_sigs: Vec<(Round, DeviceId, PartialSignature)>,
    },

    /// DAG operations response.
    DAGOperations {
        operations: Vec<(SHeader, dag::RawDAGBody)>,
    },

    /// Live update notification (pushed to subscribers).
    Update {
        /// Newly created blocks.
        new_blocks: Vec<Signed<bft::Block<SHeader>>>,
        /// Newly completed certificates.
        new_certificates: Vec<ThresholdSigned<Certificate>>,
        /// Newly completed round (if any).
        new_round_complete: Option<ThresholdSigned<RoundComplete>>,
    },

    /// Tell initiator to wait (no new data available).
    Wait,

    /// Error response.
    Error(BFTSyncError),
}

/// Summary of a peer's BFT state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BFTStatus {
    /// Highest committed round.
    pub committed_round: Round,
    /// Current round being worked on.
    pub current_round: Round,
    /// Number of certificates completed in current round.
    pub current_round_certs: u32,
    /// Total number of validators.
    pub validator_count: u32,
    /// Digest of committed state for verification.
    pub committed_state_digest: Sha256Hash,
}

/// Errors that can occur during BFT sync.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BFTSyncError {
    /// Requested rounds are not available (too old).
    RoundsTooOld { oldest_available: Round },
    /// Requested blocks not found.
    BlocksNotFound { missing: Vec<BlockId> },
    /// Invalid request parameters.
    InvalidRequest { reason: String },
    /// Rate limit exceeded.
    RateLimitExceeded,
    /// Internal error.
    InternalError,
}

// ============================================================================
// Combined Protocol Message
// ============================================================================

/// Combined message type for the BFT sync miniprotocol.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MsgBFTSync<SHeaderId, SHeader: DAGHeader> {
    Request(MsgBFTSyncRequest<SHeaderId>),
    Response(MsgBFTSyncResponse<SHeader>),
}

impl<SHeaderId, SHeader: DAGHeader> From<MsgBFTSyncRequest<SHeaderId>>
    for MsgBFTSync<SHeaderId, SHeader>
{
    fn from(req: MsgBFTSyncRequest<SHeaderId>) -> Self {
        MsgBFTSync::Request(req)
    }
}

impl<SHeaderId, SHeader: DAGHeader> From<MsgBFTSyncResponse<SHeader>>
    for MsgBFTSync<SHeaderId, SHeader>
{
    fn from(resp: MsgBFTSyncResponse<SHeader>) -> Self {
        MsgBFTSync::Response(resp)
    }
}

// ============================================================================
// Sync State Tracking
// ============================================================================

/// State of the BFT sync initiator.
#[derive(Debug)]
pub struct BFTSyncInitiator<SHeader: DAGHeader> {
    /// Current sync phase.
    phase: SyncPhase,
    /// Rounds we've requested but not yet received.
    pending_rounds: BTreeSet<Round>,
    /// Block IDs we've requested but not yet received.
    pending_blocks: BTreeSet<BlockId>,
    /// Blocks received during this sync session.
    received_blocks: BTreeMap<BlockId, Signed<bft::Block<SHeader>>>,
    /// Certificates received during this sync session.
    received_certs: BTreeMap<BlockId, ThresholdSigned<Certificate>>,
    /// Round completion proofs received.
    received_round_complete: BTreeMap<Round, ThresholdSigned<RoundComplete>>,
    /// Partial signatures we're collecting for certificates.
    pending_cert_sigs: BTreeMap<BlockId, BTreeMap<DeviceId, PartialSignature>>,
    /// Partial signatures we're collecting for round completions.
    pending_round_sigs: BTreeMap<Round, BTreeMap<DeviceId, PartialSignature>>,
    phantom: PhantomData<SHeader>,
}

/// Phases of the sync protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncPhase {
    /// Initial phase - requesting status.
    RequestingStatus,
    /// Catching up - fetching historical rounds.
    CatchingUp { target_round: Round },
    /// Subscribed to live updates.
    Subscribed { from_round: Round },
    /// Collecting partial signatures.
    CollectingSignatures,
    /// Sync complete.
    Complete,
    /// Error state.
    Error,
}

impl<SHeader: DAGHeader> BFTSyncInitiator<SHeader> {
    /// Create a new sync initiator.
    pub fn new() -> Self {
        BFTSyncInitiator {
            phase: SyncPhase::RequestingStatus,
            pending_rounds: BTreeSet::new(),
            pending_blocks: BTreeSet::new(),
            received_blocks: BTreeMap::new(),
            received_certs: BTreeMap::new(),
            received_round_complete: BTreeMap::new(),
            pending_cert_sigs: BTreeMap::new(),
            pending_round_sigs: BTreeMap::new(),
            phantom: PhantomData,
        }
    }

    /// Get the current sync phase.
    pub fn phase(&self) -> SyncPhase {
        self.phase
    }

    /// Generate the initial status request.
    pub fn create_status_request<SHeaderId>(&self) -> MsgBFTSyncRequest<SHeaderId> {
        MsgBFTSyncRequest::StatusRequest
    }

    /// Process a status response and determine next steps.
    pub fn handle_status<SHeaderId>(
        &mut self,
        local_committed: Round,
        status: BFTStatus,
    ) -> Option<MsgBFTSyncRequest<SHeaderId>> {
        let rounds_behind = status.committed_round.saturating_sub(local_committed);

        if rounds_behind > FAST_SYNC_THRESHOLD {
            // Too far behind - would need fast sync (snapshot)
            // For now, just catch up incrementally
            tracing::warn!("Far behind ({} rounds), using incremental sync", rounds_behind);
        }

        if rounds_behind == 0 && status.current_round <= local_committed + 1 {
            // Already up to date, subscribe for live updates
            self.phase = SyncPhase::Subscribed {
                from_round: local_committed + 1,
            };
            return Some(MsgBFTSyncRequest::Subscribe {
                from_round: local_committed + 1,
            });
        }

        // Need to catch up
        self.phase = SyncPhase::CatchingUp {
            target_round: status.current_round,
        };

        // Request missing rounds
        let start_round = local_committed.saturating_sub(1); // Include one extra for parent refs
        let rounds: Vec<_> = (start_round..=status.current_round)
            .take(MAX_ROUNDS_REQUEST as usize)
            .collect();

        for &r in &rounds {
            self.pending_rounds.insert(r);
        }

        Some(MsgBFTSyncRequest::BlocksRequest {
            rounds,
            proposers: None,
        })
    }

    /// Process received blocks and determine next request.
    pub fn handle_blocks<SHeaderId>(
        &mut self,
        blocks: Vec<Signed<bft::Block<SHeader>>>,
    ) -> Option<MsgBFTSyncRequest<SHeaderId>>
    where
        SHeader: Clone,
    {
        // Collect block IDs to request certificates for
        let mut block_ids = Vec::new();

        for block in blocks {
            let block_id = compute_block_id(&block);
            block_ids.push(block_id.clone());
            self.received_blocks.insert(block_id, block);
        }

        // Clear pending rounds (simplified - real impl should track more carefully)
        self.pending_rounds.clear();

        if block_ids.is_empty() {
            return None;
        }

        // Request certificates for received blocks
        for id in &block_ids {
            self.pending_blocks.insert(id.clone());
        }

        Some(MsgBFTSyncRequest::CertificatesRequest { block_ids })
    }

    /// Process received certificates and determine next request.
    pub fn handle_certificates<SHeaderId>(
        &mut self,
        certificates: Vec<ThresholdSigned<Certificate>>,
    ) -> Option<MsgBFTSyncRequest<SHeaderId>> {
        // Collect rounds that have certificates
        let mut rounds_with_certs: BTreeSet<Round> = BTreeSet::new();

        for cert in certificates {
            let (block_id, round) = match &cert {
                ThresholdSigned::ThresholdSignature { value, .. } => {
                    (value.block.clone(), value.round)
                }
                ThresholdSigned::PartialSignatures { value, .. } => {
                    (value.block.clone(), value.round)
                }
            };

            self.pending_blocks.remove(&block_id);
            rounds_with_certs.insert(round);
            self.received_certs.insert(block_id, cert);
        }

        // Request round completion proofs for rounds with enough certificates
        let rounds: Vec<_> = rounds_with_certs.into_iter().collect();

        if rounds.is_empty() {
            // Move to subscribed state if we have nothing pending
            if let SyncPhase::CatchingUp { target_round } = self.phase {
                self.phase = SyncPhase::Subscribed {
                    from_round: target_round,
                };
                return Some(MsgBFTSyncRequest::Subscribe {
                    from_round: target_round,
                });
            }
            return None;
        }

        Some(MsgBFTSyncRequest::RoundCompleteRequest { rounds })
    }

    /// Process received round completion proofs.
    pub fn handle_round_complete<SHeaderId>(
        &mut self,
        proofs: Vec<ThresholdSigned<RoundComplete>>,
    ) -> Option<MsgBFTSyncRequest<SHeaderId>> {
        let mut max_round = 0;

        for proof in proofs {
            let round = match &proof {
                ThresholdSigned::ThresholdSignature { value, .. } => value.round,
                ThresholdSigned::PartialSignatures { value, .. } => value.round,
            };
            max_round = max_round.max(round);
            self.received_round_complete.insert(round, proof);
        }

        // Transition to subscribed
        self.phase = SyncPhase::Subscribed {
            from_round: max_round + 1,
        };

        Some(MsgBFTSyncRequest::Subscribe {
            from_round: max_round + 1,
        })
    }

    /// Process a live update.
    pub fn handle_update<SHeaderId>(
        &mut self,
        new_blocks: Vec<Signed<bft::Block<SHeader>>>,
        new_certificates: Vec<ThresholdSigned<Certificate>>,
        new_round_complete: Option<ThresholdSigned<RoundComplete>>,
    ) -> Option<MsgBFTSyncRequest<SHeaderId>>
    where
        SHeader: Clone,
    {
        // Store received data
        for block in new_blocks {
            let block_id = compute_block_id(&block);
            self.received_blocks.insert(block_id, block);
        }

        for cert in new_certificates {
            let block_id = match &cert {
                ThresholdSigned::ThresholdSignature { value, .. } => value.block.clone(),
                ThresholdSigned::PartialSignatures { value, .. } => value.block.clone(),
            };
            self.received_certs.insert(block_id, cert);
        }

        if let Some(proof) = new_round_complete {
            let round = match &proof {
                ThresholdSigned::ThresholdSignature { value, .. } => value.round,
                ThresholdSigned::PartialSignatures { value, .. } => value.round,
            };
            self.received_round_complete.insert(round, proof);
        }

        // No immediate follow-up needed for updates
        None
    }

    /// Get blocks ready to be applied (have certificates and all dependencies).
    pub fn drain_ready_blocks(
        &mut self,
    ) -> Vec<(Signed<bft::Block<SHeader>>, ThresholdSigned<Certificate>)> {
        let mut ready = Vec::new();

        let block_ids: Vec<_> = self.received_blocks.keys().cloned().collect();
        for block_id in block_ids {
            if let Some(cert) = self.received_certs.get(&block_id) {
                // Check if certificate is complete
                let is_complete = matches!(cert, ThresholdSigned::ThresholdSignature { .. });
                if is_complete {
                    if let Some(block) = self.received_blocks.remove(&block_id) {
                        let cert = self.received_certs.remove(&block_id).unwrap();
                        ready.push((block, cert));
                    }
                }
            }
        }

        ready
    }

    /// Get round completion proofs ready to be applied.
    pub fn drain_ready_round_proofs(&mut self) -> Vec<ThresholdSigned<RoundComplete>> {
        let mut ready = Vec::new();

        let rounds: Vec<_> = self.received_round_complete.keys().cloned().collect();
        for round in rounds {
            if let Some(proof) = self.received_round_complete.get(&round) {
                let is_complete = matches!(proof, ThresholdSigned::ThresholdSignature { .. });
                if is_complete {
                    if let Some(proof) = self.received_round_complete.remove(&round) {
                        ready.push(proof);
                    }
                }
            }
        }

        ready
    }
}

impl<SHeader: DAGHeader> Default for BFTSyncInitiator<SHeader> {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Sync Responder
// ============================================================================

/// State of the BFT sync responder.
#[derive(Debug)]
pub struct BFTSyncResponder<SHeader: DAGHeader> {
    /// Peers subscribed to live updates, with their starting round.
    subscribers: BTreeMap<DeviceId, Round>,
    /// Rate limiting: requests per peer in current window.
    request_counts: BTreeMap<DeviceId, u32>,
    phantom: PhantomData<SHeader>,
}

impl<SHeader: DAGHeader> BFTSyncResponder<SHeader> {
    /// Create a new sync responder.
    pub fn new() -> Self {
        BFTSyncResponder {
            subscribers: BTreeMap::new(),
            request_counts: BTreeMap::new(),
            phantom: PhantomData,
        }
    }

    /// Handle a status request.
    pub fn handle_status_request(
        &self,
        bft_state: &bft::State<SHeader, impl Clone>,
    ) -> MsgBFTSyncResponse<SHeader> {
        let committed_round = bft_state.committed_round();
        let current_round = bft_state.current_round();
        let current_round_certs = bft_state.certificate_count(current_round);

        MsgBFTSyncResponse::Status(BFTStatus {
            committed_round,
            current_round,
            current_round_certs,
            validator_count: 0, // TODO: Get from config
            committed_state_digest: Sha256Hash::default(), // TODO: Compute
        })
    }

    /// Handle a blocks request.
    pub fn handle_blocks_request(
        &self,
        bft_state: &bft::State<SHeader, impl Clone>,
        rounds: &[Round],
        proposers: Option<&[DeviceId]>,
    ) -> MsgBFTSyncResponse<SHeader>
    where
        SHeader: Clone,
    {
        let mut blocks = Vec::new();

        for &round in rounds.iter().take(MAX_ROUNDS_REQUEST as usize) {
            let round_blocks = bft_state.get_blocks_for_round(round);
            for block in round_blocks {
                // Filter by proposer if specified
                if let Some(proposers) = proposers {
                    if !proposers.contains(&block.value.proposer) {
                        continue;
                    }
                }
                blocks.push(block);

                if blocks.len() >= MAX_BLOCKS_RESPONSE as usize {
                    break;
                }
            }
        }

        MsgBFTSyncResponse::Blocks { blocks }
    }

    /// Handle a certificates request.
    pub fn handle_certificates_request(
        &self,
        bft_state: &bft::State<SHeader, impl Clone>,
        block_ids: &[BlockId],
    ) -> MsgBFTSyncResponse<SHeader> {
        let mut certificates = Vec::new();

        for block_id in block_ids.iter().take(MAX_CERTS_RESPONSE as usize) {
            if let Some(cert) = bft_state.get_certificate(block_id) {
                certificates.push(cert);
            }
        }

        MsgBFTSyncResponse::Certificates { certificates }
    }

    /// Handle a round complete request.
    pub fn handle_round_complete_request(
        &self,
        bft_state: &bft::State<SHeader, impl Clone>,
        rounds: &[Round],
    ) -> MsgBFTSyncResponse<SHeader> {
        let mut proofs = Vec::new();

        for &round in rounds.iter().take(MAX_ROUNDS_REQUEST as usize) {
            if let Some(proof) = bft_state.get_round_complete(round) {
                proofs.push(proof);
            }
        }

        MsgBFTSyncResponse::RoundComplete { proofs }
    }

    /// Handle a partial signatures request.
    pub fn handle_partial_sigs_request(
        &self,
        bft_state: &bft::State<SHeader, impl Clone>,
        certificate_block_ids: &[BlockId],
        round_completions: &[Round],
    ) -> MsgBFTSyncResponse<SHeader> {
        let mut certificate_sigs = Vec::new();
        let mut round_complete_sigs = Vec::new();

        // Collect partial sigs for certificates
        for block_id in certificate_block_ids {
            if let Some(sigs) = bft_state.get_certificate_partial_sigs(block_id) {
                for (device_id, sig) in sigs {
                    certificate_sigs.push((block_id.clone(), device_id, sig));
                }
            }
        }

        // Collect partial sigs for round completions
        for &round in round_completions {
            if let Some(sigs) = bft_state.get_round_complete_partial_sigs(round) {
                for (device_id, sig) in sigs {
                    round_complete_sigs.push((round, device_id, sig));
                }
            }
        }

        MsgBFTSyncResponse::PartialSignatures {
            certificate_sigs,
            round_complete_sigs,
        }
    }

    /// Register a subscriber for live updates.
    pub fn add_subscriber(&mut self, peer: DeviceId, from_round: Round) {
        self.subscribers.insert(peer, from_round);
    }

    /// Remove a subscriber.
    pub fn remove_subscriber(&mut self, peer: &DeviceId) {
        self.subscribers.remove(peer);
    }

    /// Get subscribers that should receive an update for a given round.
    pub fn get_subscribers_for_round(&self, round: Round) -> Vec<DeviceId> {
        self.subscribers
            .iter()
            .filter(|(_, &from)| from <= round)
            .map(|(peer, _)| *peer)
            .collect()
    }

    /// Create an update message for subscribers.
    pub fn create_update(
        &self,
        new_blocks: Vec<Signed<bft::Block<SHeader>>>,
        new_certificates: Vec<ThresholdSigned<Certificate>>,
        new_round_complete: Option<ThresholdSigned<RoundComplete>>,
    ) -> MsgBFTSyncResponse<SHeader> {
        MsgBFTSyncResponse::Update {
            new_blocks,
            new_certificates,
            new_round_complete,
        }
    }
}

impl<SHeader: DAGHeader> Default for BFTSyncResponder<SHeader> {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Compute the BlockId from a signed block.
fn compute_block_id<SHeader: DAGHeader>(_block: &Signed<bft::Block<SHeader>>) -> BlockId {
    // TODO: Implement actual hash computation
    BlockId::default()
}

// ============================================================================
// Extensions to bft::State for sync support
// ============================================================================

impl<SHeader: DAGHeader, S> bft::State<SHeader, S> {
    /// Get the highest committed round.
    pub fn committed_round(&self) -> Round {
        // Find highest round with a complete round proof
        self.round_states
            .iter()
            .enumerate()
            .rev()
            .find_map(|(i, rs)| {
                if matches!(
                    rs.commit_round,
                    ThresholdSigned::ThresholdSignature { .. }
                ) {
                    Some(i as Round)
                } else {
                    None
                }
            })
            .unwrap_or(0)
    }

    /// Get the current round being worked on.
    pub fn current_round(&self) -> Round {
        self.round_states.len() as Round
    }

    /// Get the number of certificates for a round.
    pub fn certificate_count(&self, round: Round) -> u32 {
        self.round_states
            .get(round as usize)
            .map(|rs| rs.certificates.len() as u32)
            .unwrap_or(0)
    }

    /// Get all blocks for a given round.
    pub fn get_blocks_for_round(&self, round: Round) -> Vec<Signed<bft::Block<SHeader>>>
    where
        SHeader: Clone,
    {
        self.round_states
            .get(round as usize)
            .map(|rs| rs.blocks.values().cloned().collect())
            .unwrap_or_default()
    }

    /// Get a certificate by block ID.
    pub fn get_certificate(&self, _block_id: &BlockId) -> Option<ThresholdSigned<Certificate>> {
        // Search through round states
        for rs in &self.round_states {
            if let Some(cert) = rs.certificates.get(_block_id) {
                return Some(cert.clone());
            }
        }
        None
    }

    /// Get a round completion proof.
    pub fn get_round_complete(&self, round: Round) -> Option<ThresholdSigned<RoundComplete>> {
        self.round_states
            .get(round as usize)
            .map(|rs| rs.commit_round.clone())
    }

    /// Get partial signatures for a certificate.
    pub fn get_certificate_partial_sigs(
        &self,
        block_id: &BlockId,
    ) -> Option<Vec<(DeviceId, PartialSignature)>> {
        for rs in &self.round_states {
            if let Some(cert) = rs.certificates.get(block_id) {
                if let ThresholdSigned::PartialSignatures { signatures, .. } = cert {
                    return Some(signatures.iter().map(|(k, v)| (*k, v.clone())).collect());
                }
            }
        }
        None
    }

    /// Get partial signatures for a round completion.
    pub fn get_round_complete_partial_sigs(
        &self,
        round: Round,
    ) -> Option<Vec<(DeviceId, PartialSignature)>> {
        self.round_states.get(round as usize).and_then(|rs| {
            if let ThresholdSigned::PartialSignatures { signatures, .. } = &rs.commit_round {
                Some(signatures.iter().map(|(k, v)| (*k, v.clone())).collect())
            } else {
                None
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // TODO: Add tests for sync protocol
}
