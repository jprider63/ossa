use std::fmt::Debug;
use std::future::Future;

use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tracing::debug;

use crate::{auth::DeviceId, network::protocol::MiniProtocol, protocol::store_peer::{ecg_sync::ECGSyncInitiator, v0::MsgStoreSync}, store::{dag::v0::HeaderId, UntypedStoreCommand}};


/// Miniprotocol to sync the DAG in the strongly consistent BFT consensus protocol.
pub(crate) struct StoreDAGSync<Hash, HeaderId, Header> {
    peer: DeviceId,
    // Receive commands from store if we have initiative or send commands to store if we're the responder.
    recv_chan: Option<UnboundedReceiver<StoreDAGSyncCommand<HeaderId, Header>>>,
    // Send commands to store if we're the responder and send results back to store if we're the initiator.
    send_chan: UnboundedSender<UntypedStoreCommand<Hash, HeaderId, Header>>, // JP: Make this a stream?
}

// TODO: Switch to this.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) enum MsgStoreDAGSync {
}

pub(crate) enum StoreDAGSyncCommand<HeaderId, Header> {
    DAGSyncRequest {
        // ecg_status: ECGStatus<HeaderId>,
        dag_state: crate::store::dag::UntypedState<HeaderId, Header>,
    },
}
impl<Hash, HeaderId, Header> MiniProtocol for StoreDAGSync<Hash, HeaderId, Header>
where
    Hash: Send + Sync + for<'a> Deserialize<'a> + Serialize,
    HeaderId: Clone + Ord + Debug + Send + Sync + for<'a> Deserialize<'a> + Serialize,
    Header: Debug + Send + Sync + for<'a> Deserialize<'a> + Serialize,
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
                        let msg = UntypedStoreCommand::ReceivedBFTDAGOperations {
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

    fn run_client<S: crate::util::Stream<Self::Message>>(self, stream: S) -> impl Future<Output = ()> + Send {
        async move {
            todo!()
        }
    }
}
