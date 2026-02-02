use std::collections::BTreeSet;
use std::marker::PhantomData;

use ossa_crdt::register::LWW;

use crate::network::protocol::receive;
use crate::protocol::store_peer::dag_sync::{
    DAGStateSubscriber, DAGSyncInitiator, DAGSyncResponder, MsgDAGSyncRequest,
};
use crate::protocol::store_sc_dag::v0::MsgStoreDAGSync;
use crate::store::dag::{self, DAGHeader};
use crate::store::dag::v0::TestHeader;
use crate::util::UnboundChannel;

type TestCRDT = LWW<(), ()>;
type Header = TestHeader<TestCRDT>;
type Msg = MsgStoreDAGSync<u32, Header>;

/// Inserts nodes into a DAG state from a list of `(header_id, parent_ids)` tuples.
fn add_ops(st: &mut dag::State<Header, TestCRDT>, ops: &[(u32, &[u32])]) {
    for (header_id, parent_ids) in ops {
        let header = TestHeader {
            header_id: *header_id,
            parent_ids: parent_ids.to_vec(),
            phantom: PhantomData,
        };
        assert!(
            st.insert_header(header, vec![]),
            "Failed to insert header {}",
            header_id
        );
    }
}

/// Builds a typed DAG state from a list of `(header_id, parent_ids)` tuples.
fn build_state(ops: &[(u32, &[u32])]) -> dag::State<Header, TestCRDT> {
    let mut st = dag::State::<Header, TestCRDT>::new();
    add_ops(&mut st, ops);
    st
}

/// Extracts header IDs from a list of `(header, raw_body)` operation tuples.
fn extract_header_ids(ops: &[(Header, Vec<u8>)]) -> Vec<u32> {
    ops.iter().map(|(h, _)| h.get_header_id()).collect()
}

/// A `DAGStateSubscriber` that panics if called.
///
/// Use for tests where the responder should never reach the Wait path
/// (i.e., the responder always has operations or haves to send).
struct PanicSubscriber;

impl DAGStateSubscriber<(), u32, Header> for PanicSubscriber {
    async fn request_dag_state(
        &self,
        _responder: &mut DAGSyncResponder<(), u32, Header>,
        _tips: Option<BTreeSet<u32>>,
    ) -> dag::UntypedState<u32, Header> {
        panic!("Unexpected: test should not reach the Wait/subscribe path");
    }
}

/// A `DAGStateSubscriber` that immediately returns a pre-built state.
///
/// Use for tests where the responder hits the Wait path and needs an updated
/// state to continue (e.g., when both DAGs are identical or the responder is
/// behind the initiator).
struct MockSubscriber {
    updated_state: dag::UntypedState<u32, Header>,
}

impl MockSubscriber {
    fn new(ops: &[(u32, &[u32])]) -> Self {
        let st = build_state(ops);
        MockSubscriber {
            updated_state: st.state.clone(),
        }
    }
}

impl DAGStateSubscriber<(), u32, Header> for MockSubscriber {
    async fn request_dag_state(
        &self,
        _responder: &mut DAGSyncResponder<(), u32, Header>,
        _tips: Option<BTreeSet<u32>>,
    ) -> dag::UntypedState<u32, Header> {
        self.updated_state.clone()
    }
}

/// Runs a multi-round DAG sync between an initiator and responder, returning the
/// operations received by the initiator in each round (as header IDs).
///
/// This is the core test helper that all DAG sync tests use. It:
/// 1. Builds DAG states from the provided operation lists
/// 2. Creates an `UnboundChannel` pair for communication
/// 3. Runs `num_rounds` of the DAG sync protocol using `tokio::join!`
/// 4. Returns a `Vec<Vec<u32>>` where each inner vec is the header IDs of
///    operations received by the initiator in that round
///
/// # Arguments
///
/// - `i_ops` — Operations to build the initiator's DAG state (the peer requesting sync).
/// - `r_ops` — Operations to build the responder's DAG state (the peer providing data).
/// - `num_rounds` — Number of protocol rounds to execute (>= 1).
/// - `subscriber` — `DAGStateSubscriber` for the responder. Use `PanicSubscriber` for
///    tests that should never hit Wait, or `MockSubscriber` for tests that expect it.
fn run_dag_sync(
    i_ops: &[(u32, &[u32])],
    r_ops: &[(u32, &[u32])],
    num_rounds: usize,
    subscriber: impl DAGStateSubscriber<(), u32, Header>,
) -> Vec<Vec<u32>> {
    assert!(num_rounds >= 1, "Must run at least 1 round");

    let mut i_state = build_state(i_ops);
    let r_state = build_state(r_ops);

    let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
    rt.block_on(async {
        let (mut chan_i, mut chan_r) = UnboundChannel::<Msg>::new_pair();
        let mut responder = DAGSyncResponder::<(), u32, Header>::new();
        let mut results = Vec::with_capacity(num_rounds);

        // Round 1: Initial sync.
        // The initiator sends DAGInitialSync with its tips; the responder processes
        // the tips and replies with operations and/or haves.
        let ((mut initiator, ops1), ()) = tokio::join!(
            DAGSyncInitiator::<(), u32, Header>::run_new(&mut chan_i, &i_state.state),
            async {
                let req: MsgDAGSyncRequest<u32> =
                    receive(&mut chan_r).await.expect("Failed to receive round 1 request");
                match req {
                    MsgDAGSyncRequest::DAGInitialSync { tips } => {
                        responder
                            .run_initial(
                                &subscriber,
                                &mut chan_r,
                                r_state.state.clone(),
                                tips,
                            )
                            .await;
                    }
                    _ => panic!("Expected DAGInitialSync in round 1"),
                }
            }
        );
        results.push(extract_header_ids(&ops1));

        // Update initiator state with received headers.
        for (header, body) in ops1 {
            i_state.insert_header(header, body);
        }

        // Subsequent rounds: The initiator sends DAGSync with its tips and a bitmap
        // indicating which of the responder's previous haves it knows. The responder
        // uses the bitmap to narrow down the fork point and sends operations/haves.
        for round in 1..num_rounds {
            let (ops, ()) = tokio::join!(
                initiator.run_round(&mut chan_i, &i_state.state),
                async {
                    let req: MsgDAGSyncRequest<u32> = receive(&mut chan_r)
                        .await
                        .unwrap_or_else(|e| {
                            panic!("Failed to receive round {} request: {:?}", round + 1, e)
                        });
                    match req {
                        MsgDAGSyncRequest::DAGSync { tips, known } => {
                            responder
                                .run_round(
                                    &subscriber,
                                    &mut chan_r,
                                    r_state.state.clone(),
                                    tips,
                                    known,
                                )
                                .await;
                        }
                        _ => panic!("Expected DAGSync in round {}", round + 1),
                    }
                }
            );
            results.push(extract_header_ids(&ops));

            // Update initiator state with received headers.
            for (header, body) in ops {
                i_state.insert_header(header, body);
            }
        }

        // Assert that the initiator's final state includes all responder headers.
        for (header_id, _) in r_ops {
            assert!(
                i_state.contains(header_id),
                "Initiator is missing header {} from responder's state\n{results:?}",
                header_id
            );
        }

        results
    })
}

/// Example 1: Linear Catch-Up (1 round)
///
/// The initiator is behind on the same chain. The responder has B (the
/// initiator's tip) so it immediately identifies the fork point and sends
/// B's descendants C and D.
///
/// ```text
/// Initiator:  R(0) ← A(1) ← B(2)                  tips: {2}
/// Responder:  R(0) ← A(1) ← B(2) ← C(3) ← D(4)   tips: {4}
/// ```
#[test]
fn example_1_linear_catchup() {
    let results = run_dag_sync(
        &[(0, &[]), (1, &[0]), (2, &[1])],
        &[(0, &[]), (1, &[0]), (2, &[1]), (3, &[2]), (4, &[3])],
        1,
        PanicSubscriber,
    );
    assert_eq!(results[0], vec![3, 4]);
}

/// Example 2: Initiator Is Empty (1 round)
///
/// The initiator has no data. Empty tips trigger the root-node path — the
/// responder queues all roots and BFS delivers everything depth-first.
///
/// ```text
/// Initiator:  (empty)                      tips: {}
/// Responder:  R(0) ← A(1) ← B(2)          tips: {2}
/// ```
#[test]
fn example_2_initiator_empty() {
    let results = run_dag_sync(
        &[],
        &[(0, &[]), (1, &[0]), (2, &[1])],
        1,
        PanicSubscriber,
    );
    assert_eq!(results[0], vec![0, 1, 2]);
}

/// Example 3: Simple Fork (2 rounds)
///
/// Both peers diverged from a common root R. The responder doesn't have X;
/// the initiator doesn't have A, B, C.
///
/// Round 1: Responder sends haves [C, B, A, R] (exponential backoff from
/// tip C). Round 2: Initiator knows only R → responder sends A, B, C.
///
/// ```text
/// Initiator:  R(0) ── X(10)                        tips: {10}
/// Responder:  R(0) ── A(1) ── B(2) ── C(3)         tips: {3}
/// ```
#[test]
fn example_3_simple_fork() {
    let results = run_dag_sync(
        &[(0, &[]), (10, &[0])],
        &[(0, &[]), (1, &[0]), (2, &[1]), (3, &[2])],
        2,
        PanicSubscriber,
    );
    assert_eq!(results[0], vec![]);        // Round 1: haves only
    assert_eq!(results[1], vec![1, 2, 3]); // Round 2: A, B, C
}

/// Example 4: Deep Chain with Exponential Backoff (2 rounds)
///
/// A long chain where the fork point is deep. The responder's exponential
/// backoff selects haves at distances 0, 1, 2, 4, 8 from the tip plus the
/// root (depth=1), covering 15 nodes with only 6 probes.
///
/// Round 1: haves = [15, 14, 13, 11, 7, R]. Round 2: Initiator knows 7 and
/// R → responder identifies the fork region.
///
/// ```text
/// Shared:     R(0) ← 1 ← 2 ← 3 ← 4 ← 5 ← 6 ← 7
/// Initiator:  7 ← X(99)                              tips: {99}
/// Responder:  7 ← 8 ← 9 ← 10 ← 11 ← 12 ← 13 ← 14 ← 15   tips: {15}
/// ```
///
/// Round 2: Initiator knows 7 and R → responder identifies the fork region
/// and sends only the divergent nodes 8-15.
#[test]
fn example_4_deep_chain_exponential_backoff() {
    let results = run_dag_sync(
        &[
            (0, &[]), (1, &[0]), (2, &[1]), (3, &[2]),
            (4, &[3]), (5, &[4]), (6, &[5]), (7, &[6]),
            (99, &[7]), // X
        ],
        &[
            (0, &[]), (1, &[0]), (2, &[1]), (3, &[2]),
            (4, &[3]), (5, &[4]), (6, &[5]), (7, &[6]),
            (8, &[7]), (9, &[8]), (10, &[9]), (11, &[10]),
            (12, &[11]), (13, &[12]), (14, &[13]), (15, &[14]),
        ],
        2,
        PanicSubscriber,
    );
    assert_eq!(results[0], vec![]);
    assert_eq!(results[1], vec![8, 9, 10, 11, 12, 13, 14, 15]);
}

/// Example 5: Already Synchronized — Wait (1 round)
///
/// Both peers have identical DAGs. The responder has nothing to send, so it
/// sends `Wait` and blocks on `DAGStateSubscriber::request_dag_state()`.
/// The mock subscriber returns a state with an additional node C(3). The
/// responder loops, discovers C, and sends it.
///
/// ```text
/// Initiator:       R(0) ← A(1) ← B(2)              tips: {2}
/// Responder:       R(0) ← A(1) ← B(2)              tips: {2}
/// Subscriber:      R(0) ← A(1) ← B(2) ← C(3)      tips: {3}
/// ```
#[test]
fn example_5_already_synchronized_wait() {
    let common: &[(u32, &[u32])] = &[(0, &[]), (1, &[0]), (2, &[1])];
    let subscriber_state: &[(u32, &[u32])] = &[(0, &[]), (1, &[0]), (2, &[1]), (3, &[2])];

    let results = run_dag_sync(
        common,
        common,
        1,
        MockSubscriber::new(subscriber_state),
    );
    assert_eq!(results[0], vec![3]);
}

/// Example 6: Responder Is Behind the Initiator (2 rounds, Wait in round 2)
///
/// The responder has strictly less data than the initiator (R ⊂ I).
///
/// Round 1: Responder doesn't have C(3), sends haves [A(1), R(0)].
/// Round 2: Initiator knows both → bitmap [1,1]. Responder marks all its
/// nodes as known by initiator, has nothing left → sends Wait. The mock
/// subscriber provides an expanded state including D(4). The responder
/// discovers C(3) (previously in `our_unknown`) is now resolved, queues
/// C's child D, and sends D(4).
///
/// ```text
/// Initiator:       R(0) ← A(1) ← B(2) ← C(3)      tips: {3}
/// Responder:       R(0) ← A(1)                      tips: {1}
/// Subscriber:      R(0) ← A(1) ← B(2) ← C(3) ← D(4)  tips: {4}
/// ```
#[test]
fn example_6_responder_behind() {
    let subscriber_state: &[(u32, &[u32])] = &[
        (0, &[]), (1, &[0]), (2, &[1]), (3, &[2]), (4, &[3]),
    ];

    let results = run_dag_sync(
        &[(0, &[]), (1, &[0]), (2, &[1]), (3, &[2])],
        &[(0, &[]), (1, &[0])],
        2,
        MockSubscriber::new(subscriber_state),
    );
    assert_eq!(results[0], vec![]);  // Round 1: haves only
    assert_eq!(results[1], vec![4]); // Round 2: Wait → subscriber → D(4)
}

/// Example 7: Multiple Roots with Missing Parent (1 round)
///
/// The DAG has two roots (R1, R2) from concurrent writers. Node A has both
/// as parents. The initiator only has R1.
///
/// When the responder pops A from the send queue, it detects that parent R2
/// is unknown to the initiator. It queues R2 (and re-queues A) so that R2
/// is sent first, ensuring the initiator receives all parents before A.
///
/// ```text
/// Initiator:  R1(0)                                  tips: {0}
/// Responder:  R1(0) ─┬── A(2)                        tips: {2}
///             R2(1) ──┘
/// ```
#[test]
fn example_7_multiple_roots_missing_parent() {
    let results = run_dag_sync(
        &[(0, &[])],
        &[(0, &[]), (1, &[]), (2, &[0, 1])],
        2,
        PanicSubscriber,
    );
    // R2(1) is sent before A(2), so the initiator has all parents.
    assert_eq!(results[0], vec![]);
    assert_eq!(results[1], vec![1, 2]);
}

#[test]
fn example_8_multiple_roots_missing_parent() {
    let results = run_dag_sync(
        &[(0, &[]), (1, &[])],
        &[(0, &[]), (1, &[]), (2, &[0, 1])],
        1,
        PanicSubscriber,
    );
    assert_eq!(results[0], vec![2]);
}

/// Example 9: Diamond from Single Root (1 round)
///
/// A single root branches into two children that merge at a common descendant.
/// Both siblings are direct children of the known tip, so they are both
/// enqueued immediately. The merge node C is only enqueued once both siblings
/// have been sent.
///
/// ```text
/// Initiator:  R(0)                                   tips: {0}
/// Responder:  R(0) ─┬── A(1) ─┬── C(3)              tips: {3}
///                    └── B(2) ─┘
/// ```
#[test]
fn example_9_diamond_single_root() {
    let results = run_dag_sync(
        &[(0, &[])],
        &[(0, &[]), (1, &[0]), (2, &[0]), (3, &[1, 2])],
        1,
        PanicSubscriber,
    );
    // Both siblings are children of the known tip, so all three arrive in one round.
    // Heap pops B(2) before A(1) at equal depth (higher HeaderId first), then C(3).
    assert_eq!(results[0], vec![2, 1, 3]);
}

/// Example 10: Two Chains from Different Roots Merging (2 rounds)
///
/// Two independent chains (rooted at R1 and R2) merge at node C. The
/// initiator only knows R1, so R2's chain must be discovered via haves.
///
/// ```text
/// Initiator:  R1(0)                                  tips: {0}
/// Responder:  R1(0) ── A(2) ─┬── C(4)               tips: {4}
///             R2(1) ── B(3) ─┘
/// ```
#[test]
fn example_10_two_chains_merge() {
    let results = run_dag_sync(
        &[(0, &[])],
        &[(0, &[]), (1, &[]), (2, &[0]), (3, &[1]), (4, &[2, 3])],
        2,
        PanicSubscriber,
    );
    // Round 1: A(2) is sent (child of known R1). C(4) blocked on unknown B(3).
    assert_eq!(results[0], vec![2]);
    // Round 2: R2(1) discovered via haves, then B(3) and C(4) follow.
    assert_eq!(results[1], vec![1, 3, 4]);
}

/// Example 11: Three Roots Merging (2 rounds)
///
/// Three independent roots all merge at a single node D. The initiator only
/// knows R1, so R2 and R3 must be discovered via haves before D can be sent.
///
/// ```text
/// Initiator:  R1(0)                                  tips: {0}
/// Responder:  R1(0) ──┐
///             R2(1) ──┼── D(3)                       tips: {3}
///             R3(2) ──┘
/// ```
#[test]
fn example_11_three_roots_merge() {
    let results = run_dag_sync(
        &[(0, &[])],
        &[(0, &[]), (1, &[]), (2, &[]), (3, &[0, 1, 2])],
        2,
        PanicSubscriber,
    );
    // Round 1: D(3) blocked on unknown R2 and R3, nothing sent.
    assert_eq!(results[0], vec![]);
    // Round 2: R2 and R3 discovered via haves, then D follows.
    assert_eq!(results[1], vec![2, 1, 3]);
}

/// Example 12: Diamond Where Initiator Knows One Branch (2 rounds)
///
/// Same diamond shape as Example 9, but the initiator already knows one
/// branch (R and A). The other branch B must be discovered via haves
/// before the merge node C can be sent.
///
/// ```text
/// Initiator:  R(0) ── A(1)                           tips: {1}
/// Responder:  R(0) ─┬── A(1) ─┬── C(3)              tips: {3}
///                    └── B(2) ─┘
/// ```
#[test]
fn example_12_diamond_one_branch_known() {
    let results = run_dag_sync(
        &[(0, &[]), (1, &[0])],
        &[(0, &[]), (1, &[0]), (2, &[0]), (3, &[1, 2])],
        2,
        PanicSubscriber,
    );
    // Round 1: C(3) blocked on unknown B(2), nothing sent.
    assert_eq!(results[0], vec![]);
    // Round 2: B(2) discovered via haves, then C(3) follows.
    assert_eq!(results[1], vec![2, 3]);
}

/// Example 13: Initiator Knows Different Root (2 rounds)
///
/// Two roots merge at C, but the initiator only knows R2 — the opposite root
/// from the one that would normally be discovered first. R1 must be discovered
/// via haves before C can be sent.
///
/// ```text
/// Initiator:  R2(1)                                  tips: {1}
/// Responder:  R1(0) ──── C(2)                        tips: {2}
/// ```
#[test]
fn example_13_initiator_knows_different_root() {
    let results = run_dag_sync(
        &[(1, &[])],
        &[(0, &[]), (2, &[0])],
        2,
        PanicSubscriber,
    );
    // Round 1: C(2) blocked on unknown R1(0), nothing sent.
    assert_eq!(results[0], vec![]);
    // Round 2: R1(0) discovered via haves (root), then C(2) follows.
    assert_eq!(results[1], vec![0, 2]);
}

#[test]
fn example_14_initiator_knows_different_root() {
    let results = run_dag_sync(
        &[(1, &[])],
        &[(0, &[])],
        2,
        PanicSubscriber,
    );
    // Round 1: C(2) blocked on unknown R1(0), nothing sent.
    assert_eq!(results[0], vec![]);
    // Round 2: R1(0) discovered via haves (root), then C(2) follows.
    assert_eq!(results[1], vec![0]);
}

/// Example 15: Initiator Has Chain from Different Root (2 rounds)
///
/// Two chains from separate roots merge at D. The initiator has R2's chain
/// (R2 → B) but nothing from R1's chain. R1 and A must be discovered via
/// haves before D can be sent.
///
/// ```text
/// Initiator:  R2(1) ── B(3)                          tips: {3}
/// Responder:  R1(0) ── A(2) ─┬── D(4)               tips: {4}
///             R2(1) ── B(3) ─┘
/// ```
#[test]
fn example_15_initiator_has_chain_from_different_root() {
    let results = run_dag_sync(
        &[(1, &[]), (3, &[1])],
        &[(0, &[]), (1, &[]), (2, &[0]), (3, &[1]), (4, &[2, 3])],
        2,
        PanicSubscriber,
    );
    // Round 1: D(4) blocked on unknown A(2), nothing sent.
    assert_eq!(results[0], vec![]);
    // Round 2: R1(0) discovered via haves (root), then A(2) and D(4) follow.
    assert_eq!(results[1], vec![0, 2, 4]);
}
