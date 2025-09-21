use std::{collections::BTreeSet, fmt::Debug};
use std::future::Future;

use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::sync::oneshot;
use tracing::debug;

use crate::protocol::store_peer::ecg_sync::DAGStateSubscriber;
use crate::store::dag;
use crate::{auth::DeviceId, network::protocol::{receive, MiniProtocol}, protocol::store_peer::{ecg_sync::{ECGSyncInitiator, ECGSyncResponder, MsgDAGSyncRequest}, v0::{MsgStoreSync, MsgStoreSyncRequest}}, store::{dag::v0::HeaderId, UntypedStoreCommand}};


/// Miniprotocol to sync the DAG in the strongly consistent BFT consensus protocol.
pub(crate) struct StoreDAGSync<Hash, HeaderId, Header> {
    peer: DeviceId,
    // Receive commands from store if we have initiative or send commands to store if we're the responder.
    recv_chan: Option<UnboundedReceiver<StoreDAGSyncCommand<HeaderId, Header>>>,
    // Send commands to store if we're the responder and send results back to store if we're the initiator.
    send_chan: UnboundedSender<UntypedStoreCommand<Hash, HeaderId, Header>>, // JP: Make this a stream?
}

impl<Hash, HeaderId, Header> StoreDAGSync<Hash, HeaderId, Header> {
    pub(crate) fn new_server(
        peer: DeviceId,
        recv_chan: UnboundedReceiver<StoreDAGSyncCommand<HeaderId, Header>>,
        send_chan: UnboundedSender<UntypedStoreCommand<Hash, HeaderId, Header>>,
    ) -> Self {
        let recv_chan = Some(recv_chan);
        StoreDAGSync {
            peer,
            recv_chan,
            send_chan,
        }
    }

    pub(crate) fn new_client(
        peer: DeviceId,
        send_chan: UnboundedSender<UntypedStoreCommand<Hash, HeaderId, Header>>,
    ) -> Self {
        StoreDAGSync {
            peer,
            recv_chan: None,
            send_chan,
        }
    }

    pub(crate) fn peer(&self) -> DeviceId {
        self.peer
    }

    pub(crate) fn send_chan(&self) -> &UnboundedSender<UntypedStoreCommand<Hash, HeaderId, Header>> {
        &self.send_chan
    }
}

// TODO: Switch to this.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) enum MsgStoreDAGSync {
}

#[derive(Debug)]
// TODO: Rename StorePeerSCGSyncCommand
pub(crate) enum StoreDAGSyncCommand<HeaderId, Header> {
    DAGSyncRequest {
        // ecg_status: ECGStatus<HeaderId>,
        dag_state: crate::store::dag::UntypedState<HeaderId, Header>,
    },
}
impl<Hash, HeaderId, Header> MiniProtocol for StoreDAGSync<Hash, HeaderId, Header>
where
    Hash: Send + Sync + for<'a> Deserialize<'a> + Serialize,
    HeaderId: Copy + Ord + Debug + Send + Sync + for<'a> Deserialize<'a> + Serialize,
    Header: Clone + Debug + Send + Sync + for<'a> Deserialize<'a> + Serialize,
{
    type Message = MsgStoreSync<Hash, HeaderId, Header>; // MsgStoreDAGSync;

    // Has initiative
    fn run_server<S: crate::util::Stream<Self::Message>>(self, mut stream: S) -> impl Future<Output = ()> + Send {
        async move {
            let mut dag_sync: Option<ECGSyncInitiator<Hash, HeaderId, Header>> = None;

            let mut recv_chan = self
                .recv_chan
                .expect("Unreachable. Server must be given a receive channel.");
            while let Some(cmd) = recv_chan.recv().await {
                match cmd {
                    StoreDAGSyncCommand::DAGSyncRequest { dag_state } => {
                        let operations = match dag_sync {
                            None => {
                                // First round of DAG sync, so create and run first round.

                                // JP: Eventually switch ecg_state to an Arc<RWLock>?
                                let (new_dag_sync, operations) =
                                    ECGSyncInitiator::run_new(&mut stream, &dag_state).await; // TODO: Make the stream abstract over the type.
                                dag_sync = Some(new_dag_sync);
                                operations
                            }
                            Some(ref mut dag_sync) => {
                                // Subsequent rounds of ECG sync.
                                dag_sync.run_round(&mut stream, &dag_state).await
                            }
                        };

                        // JP: Should we check if the operation set is empty?
                        // if !operations.is_empty() {
                        let msg = UntypedStoreCommand::ReceivedSCGOperations {
                            peer: self.peer,
                            operations,
                        };
                        self.send_chan.send(msg).expect("TODO");
                        // } else { todo!() }
                    }
                }
            }

            debug!("StoreDAGSync receiver channel closed");
        }
    }

    fn run_client<S: crate::util::Stream<Self::Message>>(self, mut stream: S) -> impl Future<Output = ()> + Send {
        async move {
            let mut dag_sync: Option<ECGSyncResponder<Hash, HeaderId, Header>> = None;

            // TODO: Check when done.
            loop {
                // Receive request.
                let request = receive(&mut stream).await.expect("TODO");
                match request {
                    MsgStoreSyncRequest::ECGSync { request } => {
                        match request {
                            MsgDAGSyncRequest::DAGInitialSync { tips } => {
                                debug!("Received initial SCG sync request with tips: {tips:?}");

                                if dag_sync.is_some() {
                                    todo!("TODO: Error, SCG sync has already been initialized.");
                                }

                                let mut dag_sync_ = ECGSyncResponder::new();

                                let ecg_state = self.request_dag_state(&mut dag_sync_, None).await;

                                dag_sync_
                                    .run_initial(&self, &mut stream, ecg_state, tips)
                                    .await;
                                dag_sync = Some(dag_sync_);
                            }
                            MsgDAGSyncRequest::DAGSync { tips, known } => {
                                let Some(ref mut dag_sync) = dag_sync else {
                                    todo!("TODO: Error, SCG sync hasn't been initialized.");
                                };

                                let ecg_state = self.request_dag_state(dag_sync, None).await;

                                dag_sync
                                    .run_round(&self, &mut stream, ecg_state, tips, known)
                                    .await;
                            }
                        }
                    }
                    _ => {
                        todo!("Switch away from this type so that this is unreachable");
                    }
                }
            }
        }
    }
}

impl<Hash, HeaderId, Header> DAGStateSubscriber<Hash, HeaderId, Header> for StoreDAGSync<Hash, HeaderId, Header>
where
    HeaderId: Ord + Copy,
{
    async fn request_dag_state(
        &self,
        responder: &mut ECGSyncResponder<Hash, HeaderId, Header>,
        tips: Option<BTreeSet<HeaderId>>,
    ) -> dag::UntypedState<HeaderId, Header> {
        debug!("Requesting SCG state");

        // Send request to store.
        let (response_chan, recv_chan) = oneshot::channel(); // TODO: Use tokio::sync::watch?
        let cmd = UntypedStoreCommand::SubscribeSCG {
            peer: self.peer(),
            tips,
            response_chan,
        };
        self.send_chan().send(cmd).expect("TODO");

        // Wait for DAG updates.
        let state = recv_chan.await.expect("TODO");
        responder.update_our_unknown(&state);

        debug!("Received DAG state");

        state
    }
}
