# Store SC Sync (StoreDAGSync) Miniprotocol

## Overview

The Store SC Sync miniprotocol synchronizes the strongly consistent (SC) DAG between peers. It reuses the generic DAGSync algorithm — the same framework used by the ECG sync in Phase 4 of Store Sync — but operates on the SC operation graph rather than the EC operation graph.

| Property | Value |
|----------|-------|
| **Version** | V0 |
| **Stream IDs** | Allocated by the Manager miniprotocol alongside EC and BFT streams |
| **Initiator** | Server (the party that requested the stream) |
| **Source** | `protocol/store_sc_dag/v0.rs`, `protocol/store_peer/dag_sync.rs` |

Store SC Sync streams are created by the Manager miniprotocol via `CreateStoreStream` alongside EC Sync and BFT Sync streams (see `protocol/manager/v0.rs`). The Manager allocates three sequential stream IDs per store — one for EC, one for SC, and one for BFT — via `next_stream_id()`.

### Relationship to Store Sync

Store Sync handles the full bootstrap sequence (metadata, merkle tree, initial state, ECG operations). Store SC Sync only activates once the store reaches the `Syncing` state. The SC DAG is independent of the ECG DAG — it tracks operations that require BFT consensus for linearizability.

## Roles

- **Server** (has initiative) — drives the sync by sending requests. Created via `StoreDAGSync::new_server(peer, recv_chan, send_chan)`. Receives `StoreSCGSyncCommand`s from the store task via an unbounded channel and translates them into DAGSync wire messages. Wraps a `DAGSyncInitiator`.
- **Client** (no initiative) — receives requests and responds. Created via `StoreDAGSync::new_client(peer, send_chan)`. Does not receive a command channel. Wraps a `DAGSyncResponder`.

> **Note**: "Server" here means the party that requested the stream, not necessarily the TCP server. Both sides of a connection can be the server on different Store SC Sync streams.

## Message Types

All messages are serialized using CBOR via serde.

### MsgStoreDAGSync (Envelope)

Top-level message enum that wraps all Store SC Sync messages. Generic over `HeaderId` and `Header`.

| Variant | Payload |
|---------|---------|
| `Request` | `MsgDAGSyncRequest<HeaderId>` |
| `SCGResponse` | `MsgDAGSyncResponse<HeaderId, Header>` |

Conversions between `MsgStoreDAGSync` and the inner types are provided via `Into` / `TryInto` implementations.

### MsgDAGSyncRequest

Sent by the server (initiator) to request data from the client (responder). Defined in `protocol/store_peer/dag_sync.rs`.

| Variant | Fields | Description |
|---------|--------|-------------|
| `DAGInitialSync` | `tips: Vec<HeaderId>` | Start DAG sync by sending the initiator's tip header IDs |
| `DAGSync` | `tips: Vec<HeaderId>`, `known: HeaderBitmap` | Continue DAG sync with updated tips and acknowledgment bitmap |

### MsgDAGSyncResponse

Sent by the client (responder) back to the server (initiator). Defined in `protocol/store_peer/dag_sync.rs`.

| Variant | Fields | Description |
|---------|--------|-------------|
| `Response` | `have: Vec<HeaderId>`, `operations: Vec<(Header, RawDAGBody)>` | Headers the responder wants the initiator to acknowledge, plus operations to deliver |
| `Wait` | — | Nothing to send yet; a `Response` will follow when updates arrive |

### HeaderBitmap

`BitArr!(for MAX_HAVE_HEADERS as usize, in u8, Msb0)` — a 32-bit bitmap where each bit corresponds to a header in the previous `have` list. The initiator sets bit `i` to `true` if it already has the header at index `i`.

## Protocol Flow

```
Initiator (Server)                          Responder (Client)
    │                                           │
    │  [Store sends SCGSyncRequest command]     │
    │  [Collect SC DAG tips]                    │
    │                                           │
    ├── DAGInitialSync {tips} ─────────────────>│
    │                                           │
    │               [Request DAG state from store]
    │               [Mark their tips + ancestors as known]
    │               [Queue children of known tips for sending]
    │               [Prepare operations (BFS by depth)]
    │               [Prepare haves (exponential backoff)]
    │                                           │
    │<── Response {have, operations} ───────────┤
    │    (or Wait, then Response)               │
    │                                           │
    │  [Forward operations to store]            │
    │  [Build known bitmap from have]           │
    │                                           │
    │  [Store sends another SCGSyncRequest]     │
    │                                           │
    ├── DAGSync {tips, known} ─────────────────>│
    │                                           │
    │               [Process tips + known bitmap]
    │               [Prepare next batch]
    │                                           │
    │<── Response {have, operations} ───────────┤
    │                                           │
    │  [Repeat until synchronized]              │
    ▼                                           ▼
```

### Server Behavior

The server maintains a `DAGSyncInitiator` across rounds:

1. Wait for a `StoreSCGSyncCommand::SCGSyncRequest { dag_state }` from the store task
2. **First round**: Call `DAGSyncInitiator::run_new()` — sends `DAGInitialSync` with current tips, receives the response
3. **Subsequent rounds**: Call `DAGSyncInitiator::run_round()` — sends `DAGSync` with updated tips and a `known` bitmap acknowledging the previous `have` list
4. Forward received operations to the store via `UntypedStoreCommand::ReceivedSCGOperations`
5. Loop back to step 1

### Client Behavior

The client maintains a `DAGSyncResponder` across rounds:

1. Receive a request from the stream
2. **`DAGInitialSync`**: Create a new `DAGSyncResponder`, request the current DAG state from the store via `UntypedStoreCommand::SubscribeSCG`, then call `run_initial()`
3. **`DAGSync`**: Call `run_round()` with the received tips and `known` bitmap
4. If a `DAGInitialSync` arrives when already initialized: currently `todo!()` panic
5. If a `DAGSync` arrives before initialization: currently `todo!()` panic
6. Loop back to step 1

## DAG Sync Algorithm

The DAG sync algorithm is defined in `protocol/store_peer/dag_sync.rs` and is shared between ECG and SC sync. It is inspired by Git's [skipping negotiator](https://github.com/git/git/commit/42cc7485a2ec49ecc440c921d2eb0cae4da80549). The goal is to determine the set R \ I (operations the responder has that the initiator doesn't) in O(log N) network round trips.

### Initiator (DAGSyncInitiator)

The initiator drives the sync by sending its tips and acknowledging headers.

**State fields**:

| Field | Type | Description |
|-------|------|-------------|
| `have` | `Vec<HeaderId>` | Headers proposed by the responder in the last round's `have` list |

**`run_new()`** — First round:

1. Collect current DAG tips from the local state
2. Send `DAGInitialSync { tips }` to responder
3. Receive response — if `Wait`, block until a follow-up `Response` arrives
4. Store the `have` list for the next round; return received `operations`

**`run_round()`** — Subsequent rounds:

1. Build a `known` bitmap from the previous round's `have` list: for each header the responder proposed, set the bit to `true` if the initiator now has it in its DAG state
2. Collect current DAG tips
3. Send `DAGSync { tips, known }` to responder
4. Receive response; update `have` list and return `operations`

### Responder (DAGSyncResponder)

The responder maintains state across rounds to efficiently determine which operations to send.

**State fields**:

| Field | Type | Description |
|-------|------|-------------|
| `their_known` | `BTreeSet<HeaderId>` | Headers known to be in the initiator's state (including ancestors) |
| `our_unknown` | `BTreeSet<HeaderId>` | Headers the initiator has that the responder doesn't |
| `send_queue` | `BinaryHeap<(Reverse<u64>, HeaderId)>` | Priority queue of headers to deliver, ordered by depth (shallowest first) |
| `sent_haves` | `Vec<HeaderId>` | Headers sent as `have` in the last response |
| `haves_queue` | `BinaryHeap<(bool, u64, HeaderId, u64)>` | Priority queue for selecting `have` headers (is_tip, depth, id, distance) |

**`run_initial()`**:

1. Process the initiator's tips via `handle_their_tips()`
2. Delegate to `run_response_helper()` to prepare and send a response

**`run_round()`**:

1. Process the initiator's new tips via `handle_their_tips()`
2. Process the acknowledgment bitmap via `handle_their_known()`
3. Delegate to `run_response_helper()` to prepare and send a response

**`run_response_helper()`** — core response loop called by both `run_initial()` and `run_round()`:

1. On subsequent iterations (not the first), reprocess the initiator's tips via `handle_their_tips()` to incorporate any new local state learned while waiting
2. Add our tips to the `haves_queue` via `prepare_our_tips()`
3. Build the operations to deliver via `prepare_operations()`
4. Build the `have` headers to propose via `prepare_sent_haves()`
5. If both operations and `sent_haves` are empty:
   - On the first iteration, send `Wait` to the initiator
   - Subscribe for store updates via `DAGStateSubscriber::request_dag_state()` (which sends `UntypedStoreCommand::SubscribeSCG` to the store)
   - Block until updates arrive, then update `our_unknown` and loop back to step 1
6. Otherwise, send `Response { have, operations }` and return

**`handle_their_tips()`**:

- If the initiator has no tips (empty DAG), queue all root nodes for sending
- For each of the initiator's tips:
  - If we have the tip: mark it and all its ancestors as known by them (BFS traversal); queue its children for sending
  - If we don't have the tip: record it in `our_unknown`

**`handle_their_known()`** — processes the acknowledgment bitmap from the initiator:

- For each header in the previous `sent_haves`:
  - If the initiator knows it: mark it and all its ancestors as known (BFS); queue its children for sending
  - If the initiator doesn't know it: if all its parents are known by them (or it's a root node), push it to the `send_queue` for delivery

**`prepare_operations()`** — BFS through the DAG ordered by depth (shallowest first):

1. Pop headers from `send_queue`
2. Skip headers already known by the initiator (`they_know()` checks both `their_known` and `our_unknown`)
3. For unknown headers: include the header and its `RawDAGBody` in the response; mark as known
4. Always add children to the queue for future rounds
5. Stop after `MAX_DELIVER_HEADERS` (32) operations per response

**`prepare_sent_haves()`** — exponential backoff frontier:

1. Start from our DAG tips (added to `haves_queue` with distance 0, `is_tip = true`)
2. Pop headers from the queue; skip if already known by them
3. Include a header in `sent_haves` if its distance from a tip is a power of 2 (1, 2, 4, 8, ...) or if it's at depth 1 (child of root)
4. Add parents to the queue with `distance + 1`
5. Stop after `MAX_HAVE_HEADERS` (32) headers

**`update_our_unknown()`** — called when new DAG state is received:

- For each header in `our_unknown` that now exists in our state, remove it from `our_unknown` and mark it (plus ancestors) as known by them in `their_known`

### DAGStateSubscriber Trait

The `DAGStateSubscriber` trait decouples the DAG sync algorithm from the specific miniprotocol wrapper. `StoreDAGSync` implements this trait to bridge between the responder and the store task.

```
request_dag_state(responder, tips) -> UntypedState
```

Implementation for `StoreDAGSync`:

1. Send `UntypedStoreCommand::SubscribeSCG { peer, tips, response_chan }` to the store
2. Wait for the store to respond with the current DAG state via a oneshot channel
3. Call `responder.update_our_unknown()` with the new state
4. Return the state

## Store Integration

### Spawning

**Server (outgoing, with initiative)** — `store/mod.rs:1379-1398`:

1. Create an unbounded channel for `StoreSCGSyncCommand`s
2. Send `UntypedStoreCommand::RegisterOutgoingPeerSCGSyncing { peer, send_peer }` to register the channel with the store
3. Construct `StoreDAGSync::new_server(peer, recv_peer, send_commands)`
4. Run as a miniprotocol via `run_miniprotocol_async`

**Client (incoming, no initiative)** — `store/mod.rs:1670-1684`:

1. Send `UntypedStoreCommand::RegisterIncomingPeerSCGSyncing { peer }` to register with the store
2. Construct `StoreDAGSync::new_client(peer, send_commands)`
3. Run as a miniprotocol via `run_miniprotocol_async`

### Driving SC Sync — `send_sync_requests()`

The store task drives SC sync in its `send_sync_requests()` method (`store/mod.rs:627-642`):

1. Only active when the store is in the `Syncing` state
2. Collect peers without outstanding SC requests via `get_outstanding_peers(|i| &i.scg_status)`
3. For each ready peer:
   - Clone the current `sc_state.dag_state` (the `UntypedState`)
   - Send `StoreSCGSyncCommand::SCGSyncRequest { dag_state }` to the peer's server miniprotocol
   - Mark the peer as outstanding

### Handling Received Operations — `handle_received_scg_operations()`

When operations arrive from a peer (`store/mod.rs:952-1005`):

1. Mark the peer as ready for the next request (`update_outgoing_peer_scg_to_ready`)
2. For each `(header, raw_operations)` pair:
   - Deserialize the raw operations using `serde_cbor`
   - Insert the header into `sc_state.dag_state` via `insert_header()`
   - If insertion succeeds, register the SC operations with `decrypted_state`
3. Notify SCG subscribers and listeners of the updated state

### Handling Subscribe Requests — `handle_scg_subscribe()`

When a client-side responder requests DAG state (`store/mod.rs:738-765`):

1. If the store is in `Syncing` state:
   - If `tips` is `None` (first request) or our tips differ from the provided tips: respond immediately with the current `sc_state.dag_state` clone
   - Otherwise, register the peer as a subscriber — the store will notify them when the SC state changes
2. If the store is not syncing: register as subscriber (will be notified when syncing begins)

### Peer Status Tracking

Each peer has an `scg_status` field (`PeerProtocolStatus<StoreSCGSyncCommand<SHeaderId, SHeader>>`) in its `PeerInfo` struct (`store/mod.rs:169`). This tracks:

- `incoming_status: PeerStatus<()>` — whether the incoming (client) side is `Known`, `Initializing`, or `Syncing`
- `outgoing_status: PeerStatus<OutgoingPeerStatus<...>>` — whether the outgoing (server) side is ready, including the `is_outstanding` flag that enforces one-request-at-a-time per peer

## Internal Commands

### StoreSCGSyncCommand (Store → Server Miniprotocol)

Sent by the store task to the server-side miniprotocol to trigger a sync round.

| Variant | Fields | Description |
|---------|--------|-------------|
| `SCGSyncRequest` | `dag_state: UntypedState<HeaderId, Header>` | Run a round of DAG sync with the current SC state |

### UntypedStoreCommand (Miniprotocol → Store)

Relevant variants used by the Store SC Sync miniprotocol to communicate with the store task.

| Variant | Fields | Description |
|---------|--------|-------------|
| `RegisterOutgoingPeerSCGSyncing` | `peer: DeviceId`, `send_peer: UnboundedSender<StoreSCGSyncCommand>` | Register an outgoing SC sync peer with its command channel |
| `RegisterIncomingPeerSCGSyncing` | `peer: DeviceId` | Register an incoming SC sync peer |
| `ReceivedSCGOperations` | `peer: DeviceId`, `operations: Vec<(SHeader, RawDAGBody)>` | Deliver received SC operations to the store |
| `SubscribeSCG` | `peer: DeviceId`, `tips: Option<BTreeSet<SHeaderId>>`, `response_chan: oneshot::Sender<UntypedState>` | Subscribe for SC DAG state updates (used by client-side responder) |

## Constants

| Constant | Value | Location | Description |
|----------|-------|----------|-------------|
| `MAX_HAVE_HEADERS` | 32 | `protocol/store_peer/dag_sync.rs:64` | Maximum `have` headers proposed per response |
| `MAX_DELIVER_HEADERS` | 32 | `protocol/store_peer/dag_sync.rs:66` | Maximum operations delivered per response |

## Error Handling

| Condition | Current Behavior | Location |
|-----------|------------------|----------|
| DAG sync already initialized (client receives second `DAGInitialSync`) | `todo!()` panic | `store_sc_dag/v0.rs:204` |
| DAG sync not yet initialized (client receives `DAGSync` before `DAGInitialSync`) | `todo!()` panic | `store_sc_dag/v0.rs:218` |
| `Wait` response followed by second `Wait` | `todo!()` panic | `dag_sync.rs:114` |
| Stream receive failure | `expect("TODO")` panic | `store_sc_dag/v0.rs:198` |
| Stream send failure | `expect("TODO")` panics | `dag_sync.rs:139`, `dag_sync.rs:177` |
| Store command channel send failure | `expect("TODO")` panic | `store_sc_dag/v0.rs:177`, `store_sc_dag/v0.rs:253` |
| Oneshot response channel receive failure | `expect("TODO")` panic | `store_sc_dag/v0.rs:256` |
| Improperly serialized operations from peer | `expect("TODO")` panic during deserialization | `store/mod.rs:979-980` |
| Failed to insert operations from peer | Logged warning, operation skipped | `store/mod.rs:987` |
| Stale `dag_state` in subsequent rounds (server) | Logged warning only | `store_sc_dag/v0.rs:165` |

> **Note**: Error handling is currently incomplete. Most failure paths use `todo!()` or `expect("TODO")` panics. Future implementations should handle these gracefully — e.g., reconnecting on stream failures and ignoring or penalizing peers that send invalid data.

## Security Considerations

- **Operation validation**: Currently `TODO` — operations received from peers are not validated (`store/mod.rs:966`). Malicious peers could inject invalid SC operations.
- **Response size validation**: Currently `TODO` — response sizes from peers are not validated (`dag_sync.rs:119`). Peers could send oversized responses.
- **Request size validation**: Currently `TODO` — the number of tips sent in requests is not bounded (`dag_sync.rs:134`, `dag_sync.rs:172`). A peer with many tips could send large messages.
- **Byzantine resistance**: As noted in `dag_sync.rs` comments, the responder is upper bounded by the depth of its DAG (it can only send operations it has). The initiator is not similarly bounded — a malicious responder could keep proposing headers indefinitely. A cutoff threshold should be implemented. Additionally, the initiator should be cut off if it has acknowledged headers but the responder has not shared any operations.
- **Deserialization**: SC operations are deserialized with `serde_cbor::from_slice()` without size or depth limits. A malicious peer could craft payloads that cause excessive memory allocation.
- **Authentication**: The Store SC Sync protocol itself does not authenticate peers; it relies on connection-level authentication established during the handshake.

## Key Source Files

| File | Purpose |
|------|---------|
| `protocol/store_sc_dag/v0.rs` | `StoreDAGSync` struct, `MsgStoreDAGSync` envelope, `StoreSCGSyncCommand`, `MiniProtocol` impl, `DAGStateSubscriber` impl |
| `protocol/store_peer/dag_sync.rs` | Generic DAG sync algorithm: `DAGSyncInitiator`, `DAGSyncResponder`, `MsgDAGSyncRequest`, `MsgDAGSyncResponse`, `DAGStateSubscriber` trait, constants |
| `store/mod.rs` | Store task: spawning, command handling, `send_sync_requests()`, `handle_received_scg_operations()`, `handle_scg_subscribe()`, peer status tracking |
| `store/dag.rs` | `DAGHeader` trait, `UntypedState`, `State`, `NodeInfo`, `RawDAGBody` |
| `store/bft.rs` | `SCDT` trait, `bft::State` (holds `dag_state: dag::State<SHeader, S>` and `committed_frontier`) |
