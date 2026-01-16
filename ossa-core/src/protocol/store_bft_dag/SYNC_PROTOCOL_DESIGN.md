# BFT Synchronization Protocol Design

This document describes the peer synchronization protocol for the asynchronous Byzantine fault-tolerant (BFT) consensus layer.

## Overview

The BFT sync protocol enables a party to retrieve and validate state from peers while tolerating up to f Byzantine faults in a system of n = 3f + 1 validators. The protocol must handle:

1. **SCG DAG Sync** - Operations proposed for strong consistency (already implemented)
2. **Block Sync** - Signed blocks from proposers
3. **Certificate Sync** - Threshold-signed availability certificates
4. **Round Sync** - Round completion proofs and committed state

## Data Structures

```
┌─────────────────────────────────────────────────────────────────┐
│                        BFT State Structure                       │
├─────────────────────────────────────────────────────────────────┤
│                                                                  │
│  Round r-1          Round r            Round r+1                 │
│  ┌─────────┐       ┌─────────┐        ┌─────────┐               │
│  │ Block A │──────▶│ Block D │───────▶│ Block G │               │
│  │ Cert A  │       │ Cert D  │        │ Cert G  │               │
│  └─────────┘       └─────────┘        └─────────┘               │
│  ┌─────────┐       ┌─────────┐        ┌─────────┐               │
│  │ Block B │──────▶│ Block E │───────▶│ Block H │               │
│  │ Cert B  │       │ Cert E  │        │ Cert H  │               │
│  └─────────┘       └─────────┘        └─────────┘               │
│  ┌─────────┐       ┌─────────┐        ┌─────────┐               │
│  │ Block C │──────▶│ Block F │───────▶│ Block I │               │
│  │ Cert C  │       │ Cert F  │        │ (pending)│              │
│  └─────────┘       └─────────┘        └─────────┘               │
│       │                 │                                        │
│       ▼                 ▼                                        │
│  ┌──────────┐     ┌──────────┐                                  │
│  │RoundDone │     │RoundDone │                                  │
│  │  r-1     │     │   r      │                                  │
│  └──────────┘     └──────────┘                                  │
│                                                                  │
│  Each block points to:                                          │
│    - dag_frontier: tips of SCG DAG operations                   │
│    - parents: certificates from previous round                  │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

### Type Definitions (from bft.rs)

```rust
struct Block<SHeader: DAGHeader> {
    round: u64,
    dag_frontier: Frontier<SHeader::HeaderId>,  // Tips of DAG operations
    parents: Vec<CertificateId>,                 // Previous round certs (≥2/3)
    proposer: DeviceId,
}

struct Certificate {
    block: BlockId,
    round: u64,
    proposer: DeviceId,
}

struct RoundComplete {
    store_id: Sha256Hash,
    round: u64,
}

enum ThresholdSigned<A> {
    ThresholdSignature { value: A, signature: ThresholdSignature },
    PartialSignatures { value: A, signatures: BTreeMap<DeviceId, PartialSignature> },
}
```

## Protocol Messages

### Sync Request Types

```rust
enum MsgBFTSyncRequest {
    /// Initial sync - request current state summary
    StatusRequest,

    /// Request blocks for specific rounds
    BlocksRequest {
        rounds: Vec<Round>,
        proposers: Option<Vec<DeviceId>>,  // None = all proposers
    },

    /// Request certificates for specific blocks
    CertificatesRequest {
        block_ids: Vec<BlockId>,
    },

    /// Request round completion proofs
    RoundCompleteRequest {
        rounds: Vec<Round>,
    },

    /// Request partial signatures for pending items
    PartialSignaturesRequest {
        /// Request partial sigs for these certificates
        certificate_block_ids: Vec<BlockId>,
        /// Request partial sigs for these round completions
        round_completions: Vec<Round>,
    },

    /// Subscribe to new blocks/certificates (live sync)
    Subscribe {
        from_round: Round,
    },
}
```

### Sync Response Types

```rust
enum MsgBFTSyncResponse {
    /// Status response with summary of peer's state
    Status {
        /// Highest committed round
        committed_round: Round,
        /// Current round being worked on
        current_round: Round,
        /// Number of certificates in current round
        current_round_certs: u32,
        /// Digest of committed state (for verification)
        committed_state_digest: Hash,
    },

    /// Blocks response
    Blocks {
        blocks: Vec<Signed<Block>>,
    },

    /// Certificates response
    Certificates {
        certificates: Vec<ThresholdSigned<Certificate>>,
    },

    /// Round completion proofs
    RoundComplete {
        proofs: Vec<ThresholdSigned<RoundComplete>>,
    },

    /// Partial signatures
    PartialSignatures {
        certificate_sigs: Vec<(BlockId, DeviceId, PartialSignature)>,
        round_complete_sigs: Vec<(Round, DeviceId, PartialSignature)>,
    },

    /// Live update notification
    Update {
        new_blocks: Vec<Signed<Block>>,
        new_certificates: Vec<ThresholdSigned<Certificate>>,
        new_round_complete: Option<ThresholdSigned<RoundComplete>>,
    },

    /// Tell initiator to wait (no new data)
    Wait,
}
```

## Sync Protocol Flow

### Phase 1: Initial Catch-Up

When a party joins or reconnects, it needs to catch up to the current state.

```
Initiator (Syncing)              Responder (Up-to-date)
─────────────────                ──────────────────────

1. Request Status
   ──────────────────────────────▶

                                 2. Status Response
   ◀──────────────────────────────
   committed_round: r
   current_round: r+1
   current_round_certs: 5

3. Compare with local state
   If behind by > FAST_SYNC_THRESHOLD rounds:
     → Use fast sync (request state snapshot)
   Else:
     → Use incremental sync

4. Request missing rounds
   BlocksRequest { rounds: [r-2, r-1, r, r+1] }
   ──────────────────────────────▶

                                 5. Blocks Response
   ◀──────────────────────────────
   blocks: [...]

6. Request certificates for blocks
   CertificatesRequest { block_ids: [...] }
   ──────────────────────────────▶

                                 7. Certificates Response
   ◀──────────────────────────────
   certificates: [...]

8. Request round completion proofs
   RoundCompleteRequest { rounds: [...] }
   ──────────────────────────────▶

                                 9. Round Complete Response
   ◀──────────────────────────────
   proofs: [...]

10. Validate and apply state
    - Verify all signatures
    - Check certificate has ≥2/3 signatures
    - Check round complete has required threshold
    - Update local state
```

### Phase 2: Steady-State Sync

Once caught up, parties subscribe to live updates.

```
Initiator                        Responder
─────────                        ─────────

1. Subscribe { from_round: r+1 }
   ──────────────────────────────▶

   [Responder holds connection open]

                                 2. On new block/certificate:
   ◀──────────────────────────────
   Update {
     new_blocks: [...],
     new_certificates: [...],
     new_round_complete: Some(...)
   }

3. Process update, request missing data if needed
   PartialSignaturesRequest { ... }
   ──────────────────────────────▶

                                 4. PartialSignatures response
   ◀──────────────────────────────
```

### Phase 3: Signature Collection

For threshold signatures, parties need to collect partial signatures from peers.

```
Initiator (has partial sigs)     Responder (has other partial sigs)
────────────────────────         ──────────────────────────────────

1. PartialSignaturesRequest {
     certificate_block_ids: [B1, B2],
     round_completions: [r]
   }
   ──────────────────────────────▶

                                 2. Check local partial signatures
                                    for requested items

   ◀──────────────────────────────
   PartialSignatures {
     certificate_sigs: [(B1, V3, sig), (B2, V3, sig)],
     round_complete_sigs: [(r, V3, sig)]
   }

3. Aggregate partial signatures
   If threshold met → create ThresholdSignature
```

## Validation Rules

### Block Validation
1. Verify proposer's signature
2. Check round number is valid (≥ local committed round)
3. Verify dag_frontier points to valid DAG headers
4. If round > 0: verify parents contain ≥2/3 previous round certificates
5. Check proposer hasn't already proposed a different block this round

### Certificate Validation
1. Verify threshold signature or accumulate partial signatures
2. Check referenced block exists and is valid
3. Verify certificate round matches block round
4. Verify proposer matches block proposer

### Round Complete Validation
1. Verify threshold signature or accumulate partial signatures
2. Check store_id matches local store
3. Check round is the expected next committed round
4. Verify ≥2/3 certificates exist for this round before accepting

## Byzantine Resistance

### Attack Mitigation

| Attack | Mitigation |
|--------|------------|
| Equivocation (multiple blocks) | Track seen blocks per validator per round |
| Invalid signatures | Verify all signatures before accepting |
| Missing data | Request from multiple peers |
| Slow/non-responsive peer | Timeout and try other peers |
| Invalid chain of certificates | Verify parent certificate references |

### Consistency Guarantees

1. **Availability**: A committed block's operations are available from ≥2/3 of validators
2. **Uniqueness**: Only one block per validator per round can be certified
3. **Ordering**: Committed operations have deterministic total order

## State Machine

```
┌─────────────────┐
│  Disconnected   │
└────────┬────────┘
         │ connect
         ▼
┌─────────────────┐
│ StatusRequest   │──────────────┐
└────────┬────────┘              │
         │ status received       │ timeout
         ▼                       │
┌─────────────────┐              │
│ CatchingUp      │◀─────────────┘
│ (fetch rounds)  │
└────────┬────────┘
         │ caught up
         ▼
┌─────────────────┐
│ Subscribed      │
│ (live updates)  │
└────────┬────────┘
         │ new round complete
         ▼
┌─────────────────┐
│ ProcessUpdate   │
│ (validate)      │
└────────┬────────┘
         │ valid
         ▼
┌─────────────────┐
│ Subscribed      │ (loop)
└─────────────────┘
```

## Implementation Plan

### Required Components

1. **BFTSyncInitiator** - Client side that requests state
   - Track sync progress
   - Manage request/response lifecycle
   - Validate received data

2. **BFTSyncResponder** - Server side that provides state
   - Respond to sync requests
   - Push live updates to subscribers
   - Rate limit to prevent DoS

3. **SignatureAggregator** - Collect and aggregate partial signatures
   - Track partial signatures per item
   - Detect when threshold is met
   - Handle duplicate/conflicting signatures

4. **BFTStateManager** - Manage BFT state
   - Store blocks, certificates, round proofs
   - Track committed vs pending rounds
   - Compute state digests for verification

### Integration with Existing Code

The BFT sync protocol integrates with:

1. `StoreDAGSync` (store_bft_dag/v0.rs) - For SCG DAG sync
2. `dag_sync.rs` - Reuse DAG sync primitives
3. `bft.rs` - BFT state types
4. `mod.rs` - Store state machine

### Message Flow Diagram

```
┌────────────┐          ┌─────────────────┐          ┌────────────┐
│  Store     │          │  BFT Sync       │          │   Peer     │
│  Handler   │          │  Miniprotocol   │          │   Store    │
└─────┬──────┘          └────────┬────────┘          └─────┬──────┘
      │                          │                          │
      │ BFTSyncRequest           │                          │
      │─────────────────────────▶│                          │
      │                          │ MsgBFTSyncRequest        │
      │                          │─────────────────────────▶│
      │                          │                          │
      │                          │ MsgBFTSyncResponse       │
      │                          │◀─────────────────────────│
      │ ReceivedBFTData          │                          │
      │◀─────────────────────────│                          │
      │                          │                          │
      │ [Validate & Apply]       │                          │
      │                          │                          │
```

## Constants

```rust
/// Maximum rounds to request in a single message
const MAX_ROUNDS_REQUEST: u16 = 32;

/// Maximum blocks to return in a single response
const MAX_BLOCKS_RESPONSE: u16 = 64;

/// Maximum certificates to return in a single response
const MAX_CERTS_RESPONSE: u16 = 64;

/// Threshold for switching to fast sync (snapshot)
const FAST_SYNC_THRESHOLD: u64 = 100;

/// Timeout for sync requests (milliseconds)
const SYNC_REQUEST_TIMEOUT_MS: u64 = 5000;

/// Maximum pending partial signatures to track
const MAX_PENDING_PARTIAL_SIGS: usize = 1000;
```

## Security Considerations

1. **Signature Verification**: Always verify signatures before trusting any data
2. **Rate Limiting**: Limit requests per peer to prevent DoS
3. **Memory Bounds**: Cap data structures to prevent memory exhaustion
4. **Timeout Handling**: Use timeouts to prevent stuck connections
5. **Peer Rotation**: Fetch from multiple peers to avoid single point of failure
