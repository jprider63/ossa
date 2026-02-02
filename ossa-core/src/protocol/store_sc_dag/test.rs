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

    let i_state = build_state(i_ops);
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
        }

        results
    })
}
