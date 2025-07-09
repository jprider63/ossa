use bitvec::{prelude::Msb0, BitArr};
use rand::{thread_rng, Rng};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Debug;
use std::future::Future;
use std::marker::PhantomData;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::sync::{oneshot, watch};
use tokio::time::{sleep, Duration};
use tracing::{debug, error, info, warn};

use crate::auth::DeviceId;
use crate::core::{OdysseyType, StoreStatus, StoreStatuses};
use crate::network::multiplexer::{MultiplexerCommand, SpawnMultiplexerTask, StreamId};
use crate::store::UntypedStoreCommand;
use crate::{
    network::{
        multiplexer::Party,
        protocol::{receive, send, MiniProtocol},
    },
    util::{Channel, Hash, Sha256Hash, Stream},
};

// MiniProtocol instance for stream/connection management.
pub(crate) struct Manager<StoreId, Hash, HeaderId, Header> {
    party_with_initiative: Party,
    peer_id: DeviceId, // DeviceId of peer we're connected to.
    active_stores: watch::Receiver<StoreStatuses<StoreId, Hash, HeaderId, Header>>,
    // `Some` implies we have initiative.
    manager_channel: Option<UnboundedReceiver<PeerManagerCommand<StoreId>>>,
    latest_stream_id: StreamId,
    multiplexer_channel: UnboundedSender<MultiplexerCommand>,
}

impl<StoreId, Hash, HeaderId, Header> Manager<StoreId, Hash, HeaderId, Header> {
    pub(crate) fn new(
        initiative: Party,
        peer_id: DeviceId,
        active_stores: watch::Receiver<StoreStatuses<StoreId, Hash, HeaderId, Header>>,
        manager_channel: Option<UnboundedReceiver<PeerManagerCommand<StoreId>>>,
        latest_stream_id: StreamId,
        multiplexer_channel: UnboundedSender<MultiplexerCommand>,
    ) -> Manager<StoreId, Hash, HeaderId, Header> {
        Manager {
            party_with_initiative: initiative,
            peer_id,
            active_stores,
            manager_channel,
            latest_stream_id,
            multiplexer_channel,
        }
    }

    fn server_has_initiative(&self) -> bool {
        match self.party_with_initiative {
            Party::Client => false,
            Party::Server => true,
        }
    }
}

impl<
        StoreId: Send + Sync + Copy + AsRef<[u8]> + Ord + Debug + Serialize + for<'a> Deserialize<'a>,
        Hash: Send,
        HeaderId: Send,
        Header: Send,
    > MiniProtocol for Manager<StoreId, Hash, HeaderId, Header>
{
    type Message = MsgManager<StoreId>;

    async fn run_server<S: Stream<Self::Message>>(self, stream: S) {
        if self.server_has_initiative() {
            self.run_with_initiative(stream).await
        } else {
            self.run_without_initiative(stream).await
        }
        debug!("Manager server exiting");
    }

    async fn run_client<S: Stream<Self::Message>>(self, stream: S) {
        if self.server_has_initiative() {
            self.run_without_initiative(stream).await
        } else {
            // Sleep for 5 seconds to not duplicate effort from the server.
            sleep(Duration::new(5, 0)).await;
            self.run_with_initiative(stream).await
        }
        debug!("Manager client exiting");
    }
}

impl<StoreId: Send + Sync + Copy + AsRef<[u8]> + Ord + Debug, Hash, HeaderId, Header>
    Manager<StoreId, Hash, HeaderId, Header>
{
    /// Manager run in mode that sends requests to peer.
    async fn run_with_initiative<S: Stream<MsgManager<StoreId>>>(mut self, mut stream: S) {
        debug!("Mux manager started with initiative!");

        // Advertise stores.
        let shared_stores = run_advertise_stores_server::<_, _, Hash, HeaderId, Header>(
            &mut stream,
            &mut self.active_stores,
        )
        .await;
        handle_shared_stores(self.peer_id, shared_stores);

        // Note: This replaces the manager_channel with `None`. This will fail if this manager ends up being called multiple times.
        let mut cmd_chan = self
            .manager_channel
            .take()
            .expect("Manager with initiative must have command channel.");
        loop {
            debug!("Mux manager looping with initiative!");
            tokio::select! {
                changed_e = self.active_stores.changed() => {
                    changed_e.expect("TODO");

                    let shared_stores = run_advertise_stores_server::<_, _, Hash, HeaderId, Header>(&mut stream, &mut self.active_stores).await;
                    debug!("Client sent store ids: {:?}", shared_stores);
                    handle_shared_stores(self.peer_id, shared_stores);
                }
                // cmd_m = self.manager_channel.as_mut().unwrap().recv() => {
                cmd_m = cmd_chan.recv() => {
                    match cmd_m {
                        None => {
                            debug!("Manager command channel closed.");
                            // TODO: Do something here?
                        }
                        Some(PeerManagerCommand::RequestStoreSync { store_id, spawn_task }) => {
                            let stream_id = self.next_stream_id();
                            let _response = self.run_request_new_stream_server(&mut stream, stream_id, store_id, spawn_task).await;
                            debug!("Requested to sync store with peer.");
                        }
                    }
                }
            }
        }
    }

    /// Manager run in mode that responds to requests from peer.
    async fn run_without_initiative<S: Stream<MsgManager<StoreId>>>(mut self, mut stream: S) {
        debug!("Mux manager started without initiative!");

        loop {
            debug!("Mux manager looping without initiative!");
            // Receive requests from initiator.
            let response: MsgManagerRequest<StoreId> = receive(&mut stream).await.expect("TODO");
            match response {
                MsgManagerRequest::AdvertiseStores { nonce, store_ids } => {
                    debug!("Received MsgManagerRequest::AdvertiseStores: {nonce:?}, {store_ids:?}");
                    let shared_stores =
                        run_advertise_stores_client::<_, _, Hash, HeaderId, Header>(
                            &mut stream,
                            nonce,
                            store_ids,
                            &mut self.active_stores,
                        )
                        .await;
                    // TODO: Store and handle peers too?
                    debug!("Server sent store ids: {:?}", shared_stores);
                    handle_shared_stores(self.peer_id, shared_stores);
                }
                MsgManagerRequest::CreateStoreStream {
                    stream_id,
                    store_id,
                } => {
                    debug!(
                        "Received MsgManagerRequest::CreateStoreStream: {stream_id}, {store_id:?}"
                    );
                    self.run_request_new_stream_client(&mut stream, stream_id, store_id)
                        .await;
                }
            }
        }
    }

    /// Returns whether the proposed stream id is valid for the peer.
    fn is_valid_stream_id(&self, our_initiative: bool, stream_id: &StreamId) -> bool {
        let our_party = if our_initiative {
            // We have initiative.
            self.party_with_initiative
        } else {
            // We don't have initiative.
            self.party_with_initiative.dual()
        };
        let their_party = our_party.dual();
        if their_party.is_client() {
            stream_id % 2 == 1
        } else {
            stream_id % 2 == 0
        }
    }

    fn next_stream_id(&mut self) -> StreamId {
        self.latest_stream_id += 2;

        self.latest_stream_id
    }

    /// Run protocol to start a new (StorePeer) stream as the client (responder).
    async fn run_request_new_stream_client<S: Stream<MsgManager<StoreId>>>(
        &self,
        stream: &mut S,
        stream_id: StreamId,
        store_id: StoreId,
    ) {
        let accept = {
            // Check if stream is valid (it can be allocated by peer and is available).
            let is_valid_id = self.is_valid_stream_id(false, &stream_id);
            if !is_valid_id {
                debug!("Peer sent invalid stream id.");
                Err(MsgManagerError::InvalidStreamId)
            } else {
                // Ask store task if they want to sync with this peer.
                let store_chan_m = {
                    self.active_stores
                        .borrow()
                        .get(&store_id)
                        .and_then(|s| s.command_channel().cloned())
                };
                if let Some(store_chan) = store_chan_m {
                    let (response_chan, rx) = oneshot::channel();
                    store_chan
                        .send(UntypedStoreCommand::SyncWithPeer {
                            peer: self.peer_id,
                            response_chan,
                        })
                        .expect("TODO");

                    let spawn_task = rx.await.expect("TODO");
                    if let Some(spawn_task) = spawn_task {
                        // Tell multiplexer to create miniprotocol.
                        let (response_chan, rx) = oneshot::channel();
                        let cmd = MultiplexerCommand::CreateStream {
                            stream_id,
                            spawn_task,
                            response_chan,
                        };
                        self.multiplexer_channel.send(cmd).expect("TODO");

                        // Wait for stream to be created and return success.
                        let is_running = rx.await.expect("TODO");
                        Ok(is_running)
                    } else {
                        debug!("Store rejected syncing.");
                        Ok(false)
                    }
                } else {
                    // Store isn't running.
                    debug!("Store isn't running.");
                    Ok(false)
                }
            }
        };

        // Send back response to peer.
        let response = MsgManagerCreateStoreResponse { accept };
        send(stream, response).await.expect("TODO");
    }

    async fn run_request_new_stream_server<S: Stream<MsgManager<StoreId>>>(
        &self,
        stream: &mut S,
        stream_id: StreamId,
        store_id: StoreId,
        spawn_task: Box<SpawnMultiplexerTask>,
    ) {
        // Send request message.
        let req = MsgManagerRequest::CreateStoreStream {
            stream_id,
            store_id,
        };
        send(stream, req).await.expect("TODO");

        // Wait for response.
        let response: MsgManagerCreateStoreResponse = receive(stream).await.expect("TODO");

        match response.accept {
            Err(MsgManagerError::InvalidStreamId) => {
                warn!("Invalid stream id");
                todo!("Restore status to Known");
            }
            Ok(false) => {
                info!("Peer denied sync request for store"); // : {}", store_id);
                todo!("Restore status to Known");
            }
            Ok(true) => {
                // Tell multiplexer to create miniprotocol.
                let (response_chan, rx) = oneshot::channel();
                let cmd = MultiplexerCommand::CreateStream {
                    stream_id,
                    spawn_task,
                    response_chan,
                };
                self.multiplexer_channel.send(cmd).expect("TODO");

                // Wait for stream to be created and return success.
                let is_running = rx.await.expect("TODO");

                if !is_running {
                    error!("Failed to create miniprotocol stream to sync store.");
                    panic!(
                        "TODO: Send shutdown for this miniprotocol and restore status to Known."
                    );
                }
            }
        }
    }
}

// Server                         Client
//        ----- My StoreIds ---->
//        <---- My StoreIds -----
//
// Store (Peer, Vec<StoreId>) in peer thread? Probably in watch::Sender<Map<PeerId, Vec<StoreId>>>
// Or: watch::Sender<Map<StoreId, Vec<PeerId>>?
// Or: Map<StoreId, watch::Sender<Set<PeerId>>? ***
// Or: Spawn sync threads for each shared store.

fn handle_shared_stores<StoreId, Hash, HeaderId, Header>(
    peer_id: DeviceId,
    shared_stores: Vec<(
        StoreId,
        UnboundedSender<UntypedStoreCommand<Hash, HeaderId, Header>>,
    )>,
) {
    // Register the peer for this store.
    let peers = vec![peer_id];

    for (_store_id, store_sender) in shared_stores {
        let cmd = UntypedStoreCommand::RegisterPeers {
            peers: peers.clone(),
        };
        let res = store_sender.send(cmd); // .expect("TODO");
        if res.is_err() {
            error!("Failed to register peer: {:?}", res);
        }
    }
}

fn hash_store_id_with_nonce<StoreId: AsRef<[u8]>>(
    nonce: [u8; 4],
    store_id: &StoreId,
) -> Sha256Hash {
    let mut h = <Sha256Hash as Hash>::new();
    <Sha256Hash as Hash>::update(&mut h, nonce);
    <Sha256Hash as Hash>::update(&mut h, store_id);
    <Sha256Hash as Hash>::finalize(h)
}

async fn run_advertise_stores_server<
    S: Stream<MsgManager<StoreId>>,
    StoreId,
    Hash,
    HeaderId,
    Header,
>(
    stream: &mut S,
    store_ids: &mut watch::Receiver<StoreStatuses<StoreId, Hash, HeaderId, Header>>,
) -> Vec<(
    StoreId,
    UnboundedSender<UntypedStoreCommand<Hash, HeaderId, Header>>,
)>
where
    StoreId: Copy + AsRef<[u8]>,
{
    // TODO: Prioritize and choose stores.
    // Truncate stores length to MAX_ADVERTISE_STORES.
    let store_ids: Vec<_> = store_ids
        .borrow_and_update()
        .iter()
        .filter_map(|e| e.1.command_channel().map(|c| (e.0, c)))
        .take(MAX_ADVERTISE_STORES)
        .map(|(&s, c)| (s, c.clone()))
        .collect();

    // Send store advertising request.
    let nonce = thread_rng().gen();

    let hashed_store_ids = store_ids
        .iter()
        .map(|(store_id, _)| hash_store_id_with_nonce(nonce, store_id))
        .collect();
    let req = MsgManagerRequest::AdvertiseStores {
        nonce,
        store_ids: hashed_store_ids,
    };
    send(stream, req).await.expect("TODO");

    // Wait for response.
    let response: MsgManagerAdvertiseStoresResponse = receive(stream).await.expect("TODO");

    store_ids
        .into_iter()
        .zip(response.have_stores)
        .filter_map(|((store_id, chan), is_shared)| {
            if is_shared {
                Some((store_id, chan))
            } else {
                None
            }
        })
        .collect()
}

async fn run_advertise_stores_client<
    S: Stream<MsgManager<StoreId>>,
    StoreId: Copy + Ord + AsRef<[u8]>,
    Hash,
    HeaderId,
    Header,
>(
    stream: &mut S,
    nonce: [u8; 4],
    their_store_ids: Vec<Sha256Hash>,
    our_store_ids: &mut watch::Receiver<StoreStatuses<StoreId, Hash, HeaderId, Header>>,
) -> Vec<(
    StoreId,
    UnboundedSender<UntypedStoreCommand<Hash, HeaderId, Header>>,
)> {
    let mut our_store_ids: BTreeMap<Sha256Hash, (StoreId, UnboundedSender<_>)> = our_store_ids
        .borrow_and_update()
        .iter()
        .filter_map(|e| e.1.command_channel().map(|c| (e.0, c)))
        .map(|(store_id, c)| {
            let h = hash_store_id_with_nonce(nonce, store_id);
            (h, (*store_id, c.clone()))
        })
        .collect();

    let mut have_stores = StoreBitmap::ZERO;
    let mut mutual_store_ids = Vec::new();
    their_store_ids
        .into_iter()
        .enumerate()
        .for_each(|(i, their_store_id)| {
            // If they send the same store multiple times, subsequent responses will be false.
            if let Some(v) = our_store_ids.remove(&their_store_id) {
                have_stores.set(i, true);
                mutual_store_ids.push((v.0, v.1));
            }
        });

    let response = MsgManagerAdvertiseStoresResponse { have_stores };
    send(stream, response).await.expect("TODO");

    mutual_store_ids
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) enum MsgManager<StoreId> {
    Request(MsgManagerRequest<StoreId>),
    AdvertiseStoresResponse(MsgManagerAdvertiseStoresResponse),
    CreateStoreResponse(MsgManagerCreateStoreResponse),
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) enum MsgManagerRequest<StoreId> {
    // Advertise stores using a poor man's PSI.
    AdvertiseStores {
        nonce: [u8; 4],
        store_ids: Vec<Sha256Hash>,
        // Hash of store ids, using the nonce as a salt.
        // Maximum length is MAX_ADVERTISE_STORES.
        // TODO: Verify length while parsing.
    },
    /// Request to create a store stream.
    CreateStoreStream {
        stream_id: StreamId,
        store_id: StoreId,
    },
}

pub const MAX_ADVERTISE_STORES: usize = 256;
pub type StoreBitmap = BitArr!(for MAX_ADVERTISE_STORES, in u8, Msb0);
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct MsgManagerAdvertiseStoresResponse {
    have_stores: StoreBitmap,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct MsgManagerCreateStoreResponse {
    /// Whether or not the peer accepts or rejects the new stream.
    accept: Result<bool, MsgManagerError>, // JP: The MsgManagerError should probably be pulled out front?
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) enum MsgManagerError {
    InvalidStreamId,
}

impl<StoreId> Into<MsgManager<StoreId>> for MsgManagerRequest<StoreId> {
    fn into(self) -> MsgManager<StoreId> {
        MsgManager::Request(self)
    }
}

impl<StoreId> Into<MsgManager<StoreId>> for MsgManagerAdvertiseStoresResponse {
    fn into(self) -> MsgManager<StoreId> {
        MsgManager::AdvertiseStoresResponse(self)
    }
}

impl<StoreId> Into<MsgManager<StoreId>> for MsgManagerCreateStoreResponse {
    fn into(self) -> MsgManager<StoreId> {
        MsgManager::CreateStoreResponse(self)
    }
}

impl<StoreId> TryInto<MsgManagerRequest<StoreId>> for MsgManager<StoreId> {
    type Error = ();
    fn try_into(self) -> Result<MsgManagerRequest<StoreId>, ()> {
        match self {
            MsgManager::Request(r) => Ok(r),
            MsgManager::AdvertiseStoresResponse(_) => Err(()),
            MsgManager::CreateStoreResponse(_) => Err(()),
        }
    }
}

impl<StoreId> TryInto<MsgManagerAdvertiseStoresResponse> for MsgManager<StoreId> {
    type Error = ();
    fn try_into(self) -> Result<MsgManagerAdvertiseStoresResponse, ()> {
        match self {
            MsgManager::Request(_) => Err(()),
            MsgManager::CreateStoreResponse(_) => Err(()),
            MsgManager::AdvertiseStoresResponse(r) => Ok(r),
        }
    }
}

impl<StoreId> TryInto<MsgManagerCreateStoreResponse> for MsgManager<StoreId> {
    type Error = ();
    fn try_into(self) -> Result<MsgManagerCreateStoreResponse, ()> {
        match self {
            MsgManager::Request(_) => Err(()),
            MsgManager::AdvertiseStoresResponse(_) => Err(()),
            MsgManager::CreateStoreResponse(r) => Ok(r),
        }
    }
}

// #[derive(Debug)]
pub(crate) enum PeerManagerCommand<StoreId> {
    /// Request that the peer sync the given store. Creates a new multiplexer stream.
    RequestStoreSync {
        store_id: StoreId,
        spawn_task: Box<SpawnMultiplexerTask>,
    },
}
