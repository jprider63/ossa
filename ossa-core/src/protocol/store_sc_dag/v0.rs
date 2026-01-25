use std::future::Future;
use std::{collections::BTreeSet, fmt::Debug};

use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::sync::oneshot;
use tracing::{debug, warn};

use crate::protocol::store_peer::dag_sync::{DAGStateSubscriber, MsgDAGSyncResponse};
use crate::store::dag;
use crate::util::Stream;
use crate::{
    auth::DeviceId,
    network::protocol::{receive, MiniProtocol},
    protocol::store_peer::{
        dag_sync::{DAGSyncInitiator, DAGSyncResponder, MsgDAGSyncRequest},
    },
    store::UntypedStoreCommand,
};

/// Miniprotocol to sync the DAG in the strongly consistent BFT consensus protocol.
pub(crate) struct StoreDAGSync<Hash, SHeaderId, SHeader, THeaderId, THeader> {
    peer: DeviceId,
    // Receive commands from store if we have initiative or send commands to store if we're the responder.
    recv_chan: Option<UnboundedReceiver<StoreSCGSyncCommand<SHeaderId, SHeader>>>,
    // Send commands to store if we're the responder and send results back to store if we're the initiator.
    send_chan: UnboundedSender<UntypedStoreCommand<Hash, SHeaderId, SHeader, THeaderId, THeader>>, // JP: Make this a stream?
}

impl<Hash, SHeaderId, SHeader, THeaderId, THeader>
    StoreDAGSync<Hash, SHeaderId, SHeader, THeaderId, THeader>
{
    pub(crate) fn new_server(
        peer: DeviceId,
        recv_chan: UnboundedReceiver<StoreSCGSyncCommand<SHeaderId, SHeader>>,
        send_chan: UnboundedSender<
            UntypedStoreCommand<Hash, SHeaderId, SHeader, THeaderId, THeader>,
        >,
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
        send_chan: UnboundedSender<
            UntypedStoreCommand<Hash, SHeaderId, SHeader, THeaderId, THeader>,
        >,
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

    pub(crate) fn send_chan(
        &self,
    ) -> &UnboundedSender<UntypedStoreCommand<Hash, SHeaderId, SHeader, THeaderId, THeader>> {
        &self.send_chan
    }
}

// TODO: Switch to this.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) enum MsgStoreDAGSync<HeaderId, Header> {
    Request(MsgDAGSyncRequest<HeaderId>),
    SCGResponse(MsgDAGSyncResponse<HeaderId, Header>),
}

#[derive(Debug)]
// TODO: Rename StorePeerSCGSyncCommand
pub(crate) enum StoreSCGSyncCommand<HeaderId, Header> {
    SCGSyncRequest {
        // ecg_status: ECGStatus<HeaderId>,
        dag_state: crate::store::dag::UntypedState<HeaderId, Header>,
    },
}

impl<HeaderId, Header> Into<MsgStoreDAGSync<HeaderId, Header>> for MsgDAGSyncRequest<HeaderId> {
    fn into(self) -> MsgStoreDAGSync<HeaderId, Header> {
        MsgStoreDAGSync::Request(self)
    }
}

impl<HeaderId, Header> Into<MsgStoreDAGSync<HeaderId, Header>>
    for MsgDAGSyncResponse<HeaderId, Header>
{
    fn into(self) -> MsgStoreDAGSync<HeaderId, Header> {
        MsgStoreDAGSync::SCGResponse(self)
    }
}

impl<HeaderId, Header> TryInto<MsgDAGSyncRequest<HeaderId>> for MsgStoreDAGSync<HeaderId, Header> {
    type Error = ();
    fn try_into(self) -> Result<MsgDAGSyncRequest<HeaderId>, ()> {
        match self {
            MsgStoreDAGSync::Request(r) => Ok(r),
            MsgStoreDAGSync::SCGResponse(_) => Err(()),
        }
    }
}

impl<HeaderId, Header> TryInto<MsgDAGSyncResponse<HeaderId, Header>>
    for MsgStoreDAGSync<HeaderId, Header>
{
    type Error = ();
    fn try_into(self) -> Result<MsgDAGSyncResponse<HeaderId, Header>, ()> {
        match self {
            MsgStoreDAGSync::Request(_) => Err(()),
            MsgStoreDAGSync::SCGResponse(r) => Ok(r),
        }
    }
}

impl<Hash, SHeaderId, SHeader, THeaderId, THeader> MiniProtocol
    for StoreDAGSync<Hash, SHeaderId, SHeader, THeaderId, THeader>
where
    Hash: Send + Sync + for<'a> Deserialize<'a> + Serialize,
    SHeaderId: Copy + Ord + Debug + Send + Sync + for<'a> Deserialize<'a> + Serialize,
    SHeader: Clone + Debug + Send + Sync + for<'a> Deserialize<'a> + Serialize,
    THeaderId: Copy + Ord + Debug + Send + Sync + for<'a> Deserialize<'a> + Serialize,
    THeader: Clone + Debug + Send + Sync + for<'a> Deserialize<'a> + Serialize,
{
    type Message = MsgStoreDAGSync<SHeaderId, SHeader>;

    // Has initiative
    fn run_server<S: Stream<Self::Message>>(
        self,
        mut stream: S,
    ) -> impl Future<Output = ()> + Send {
        async move {
            debug!("StoreDAGSync server running!");
            let mut dag_sync: Option<DAGSyncInitiator<Hash, SHeaderId, SHeader>> = None;

            let mut recv_chan = self
                .recv_chan
                .expect("Unreachable. Server must be given a receive channel.");
            while let Some(cmd) = recv_chan.recv().await {
                match cmd {
                    StoreSCGSyncCommand::SCGSyncRequest { dag_state } => {
                        let operations = match dag_sync {
                            None => {
                                // First round of DAG sync, so create and run first round.

                                // JP: Eventually switch dag_state to an Arc<RWLock>?
                                let (new_dag_sync, operations) =
                                    DAGSyncInitiator::<Hash, SHeaderId, SHeader>::run_new(
                                        &mut stream,
                                        &dag_state,
                                    )
                                    .await; // TODO: Make the stream abstract over the type.
                                dag_sync = Some(new_dag_sync);
                                operations
                            }
                            Some(ref mut dag_sync) => {
                                warn!("TODO: External updates to `dag_state` might make this out of sync?");
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

    fn run_client<S: Stream<Self::Message>>(
        self,
        mut stream: S,
    ) -> impl Future<Output = ()> + Send {
        async move {
            debug!("StoreDAGSync client running!");
            let mut dag_sync: Option<DAGSyncResponder<Hash, SHeaderId, SHeader>> = None;

            // TODO: Check when done.
            loop {
                // Receive request.
                let request = receive(&mut stream).await.expect("TODO");
                match request {
                    MsgDAGSyncRequest::DAGInitialSync { tips } => {
                        debug!("Received initial SCG sync request with tips: {tips:?}");

                        if dag_sync.is_some() {
                            todo!("TODO: Error, SCG sync has already been initialized.");
                        }

                        let mut dag_sync_ = DAGSyncResponder::new();

                        let scg_state = self.request_dag_state(&mut dag_sync_, None).await;

                        dag_sync_
                            .run_initial(&self, &mut stream, scg_state, tips)
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
        }
    }
}

impl<Hash, SHeaderId, SHeader, THeaderId, THeader> DAGStateSubscriber<Hash, SHeaderId, SHeader>
    for StoreDAGSync<Hash, SHeaderId, SHeader, THeaderId, THeader>
where
    SHeaderId: Ord + Copy,
{
    async fn request_dag_state(
        &self,
        responder: &mut DAGSyncResponder<Hash, SHeaderId, SHeader>,
        tips: Option<BTreeSet<SHeaderId>>,
    ) -> dag::UntypedState<SHeaderId, SHeader> {
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
