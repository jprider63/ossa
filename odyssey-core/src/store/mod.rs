use itertools::Itertools;
use odyssey_crdt::CRDT;
use rand::{seq::SliceRandom as _, thread_rng};
use serde::Serialize;
use tokio::{sync::{mpsc::{UnboundedReceiver, UnboundedSender}, oneshot::{self, Sender}}, task::JoinHandle};
use tracing::{debug, error, warn};
use std::{collections::{BTreeMap, BTreeSet}, ops::Range};
use std::fmt::Debug;
use typeable::{TypeId, Typeable};

use crate::{auth::DeviceId, core::{OdysseyType, SharedState}, network::{multiplexer::{run_miniprotocol_async, SpawnMultiplexerTask}, protocol::MiniProtocol}, protocol::{manager::v0::PeerManagerCommand, store_peer::v0::{MsgStoreSyncRequest, StoreSync, StoreSyncCommand}}, store::ecg::{ECGBody, ECGHeader}, util::{self, compress_consecutive_into_ranges}};

pub mod ecg;
pub mod v0; // TODO: Move this to network::protocol

pub use v0::{MetadataBody, MetadataHeader, Nonce};

pub struct State<Header: ecg::ECGHeader<T>, T: CRDT, Hash> {
    // Peers that also have this store (that we are potentially connected to?).
    peers: BTreeMap<DeviceId, PeerInfo>, // BTreeSet<DeviceId>,
    state_machine: StateMachine<Header, T, Hash>,
    metadata_subscribers: BTreeMap<DeviceId, oneshot::Sender<Option<v0::MetadataHeader<Hash>>>>,
    merkle_subscribers: BTreeMap<DeviceId, (Vec<Range<u64>>, oneshot::Sender<Option<Vec<Hash>>>)>,
}

// States are:
// - Initializing - Setting up the thread that owns the store (not defined here).
// - DownloadingMetadata - Don't have the header so we're downloading it.
// - Syncing - Have the header and syncing updates between peers.
pub enum StateMachine<Header: ecg::ECGHeader<T>, T: CRDT, Hash> {
    DownloadingMetadata {
        store_id: Hash,
    },
    DownloadingMerkle {
        metadata: MetadataHeader<Hash>,
        piece_hashes: Vec<Option<Hash>>,
    },
    DownloadingInitialState {
        metadata: MetadataHeader<Hash>,
        piece_hashes: Vec<Hash>,
        initial_state: Vec<Option<u8>>,
    },
    Syncing {
        metadata: MetadataHeader<Hash>,
        piece_hashes: Vec<Hash>,
        initial_state: Vec<u8>, // Or just T?
        ecg_state: ecg::State<Header, T>,
        decrypted_state: DecryptedState<Header, T>, // Temporary
        // decrypted_state: Option<DecryptedState<Header, T>>, // JP: Is this actually used?
                                                               // Does it make sense?
    },
}

pub struct DecryptedState<Header: ecg::ECGHeader<T>, T: CRDT> {
    /// Latest ECG application state we've seen.
    latest_state: T,

    /// Headers corresponding to the latest ECG application state.
    latest_headers: BTreeSet<Header::HeaderId>,
}

/// Information about a peer.
#[derive(Debug)]
struct PeerInfo {
    /// Status of incoming sync status from peer.
    incoming_status: PeerStatus<()>,
    /// Status of outgoing sync status to peer.
    outgoing_status: PeerStatus<OutgoingPeerStatus>,
}

impl PeerInfo {
    /// Checks if the peer is ready for a sync request.
    /// This means the peer is syncing and does not have an outstanding request.
    fn is_ready_for_sync(&self) -> bool {
        if let PeerStatus::Syncing(s) = &self.outgoing_status {
            !s.is_outstanding
        } else {
            false
        }
    }
}

#[derive(Debug)]
/// Outgoing information about a syncing peer.
struct OutgoingPeerStatus {
    /// Sender channel for requests to the outgoing peer store.
    sender_peer: UnboundedSender<StoreSyncCommand>,
    /// Whether we have an outgoing peer request that is outstanding.
    is_outstanding: bool,
}

/// Status of peers who we are potentially syncing this store with.
#[derive(Debug)]
pub(crate) enum PeerStatus<T> {
    /// Peer is known and likely connected to, but is not syncing this store.
    Known, // JP: Inactive?
    /// Setting up the thread that syncs the store with the peer. It's possible that the peer will reject the sync request.
    Initializing,
    // {
    //     task: JoinHandle<()>,
    // },
    /// The thread that syncs the store with the peer is syncing.
    Syncing(T), // JP: Running instead?

//     /// Peers that we are connected to, but are not syncing this store. It's possible that these connections have dropped.
//     Known(), // TODO: Last known IP address, port, statistics (latency, bandwidth, ...). JP: Should some of this be stored globally?
//     /// Peers that we are connected to, but are not syncing this store. It's possible that these connections have dropped.
//     Connected(), 
//     /// Peers that we are connected to and are syncing this store. It's possible that these connections have dropped.
//     Active(), 
}

impl<T> PeerStatus<T> {
    fn is_known(&self) -> bool {
        if let PeerStatus::Known = self {
            true
        } else {
            false
        }
    }

    fn is_initializing(&self) -> bool {
        if let PeerStatus::Initializing = self {
            true
        } else {
            false
        }
    }
}

impl<Header: ecg::ECGHeader<T>, T: CRDT, Hash: util::Hash + Debug> State<Header, T, Hash> {
    /// Initialize a new store with the given state. This initializes the header, including
    /// generating a random nonce.
    pub fn new_syncing(initial_state: T) -> State<Header, T, Hash>
    where
        T: Serialize + Typeable,
    {
        let init_body = MetadataBody::new(&initial_state);
        let store_header = MetadataHeader::generate::<T>(&init_body);
        let decrypted_state = DecryptedState {
            latest_state: initial_state,
            latest_headers: BTreeSet::new(),
        };

        let (piece_hashes, initial_state) = init_body.build();

        let state_machine = StateMachine::Syncing {
            metadata: store_header,
            piece_hashes,
            initial_state,
            ecg_state: ecg::State::new(),
            decrypted_state, // : Some(decrypted_state),
        };
        State {
            peers: BTreeMap::new(),
            state_machine,
            metadata_subscribers: BTreeMap::new(),
            merkle_subscribers: BTreeMap::new(),
        }
    }

    /// Create a new store with the given store id that is downloading the store's header.
    pub(crate) fn new_downloading(store_id: Hash) -> Self {
        let state_machine = StateMachine::DownloadingMetadata {
            store_id
        };

        State {
            peers: BTreeMap::new(),
            state_machine,
            metadata_subscribers: BTreeMap::new(),
            merkle_subscribers: BTreeMap::new(),
        }
    }

    pub fn store_id(&self) -> Hash {
        match &self.state_machine {
            StateMachine::DownloadingMetadata{store_id} => {
                *store_id
            }
            StateMachine::DownloadingMerkle { metadata, .. } => {
                metadata.store_id()
            }
            StateMachine::DownloadingInitialState { metadata, .. } => {
                metadata.store_id()
            }
            StateMachine::Syncing { metadata, .. } => {
                metadata.store_id()
            }
        }
    }

    /// Insert a peer as known if its status isn't already tracked by the store.
    fn insert_known_peer(&mut self, peer: DeviceId) {
        self.peers
            .entry(peer)
            // .and_modify(|s| {
            //     match s {
            //         PeerStatus::Known => *s = PeerStatus::Known,
            //         PeerStatus::Initializing => (),
            //         PeerStatus::Syncing => (),
            //     }
            // })
            // .or_insert(PeerStatus::Known);
            .or_insert(PeerInfo { incoming_status: PeerStatus::Known, outgoing_status: PeerStatus::Known});
    }

    /// Helper to update a known peer to initializing.
    fn update_peer_to_initializing<A>(&mut self, peer: &DeviceId, direction_lambda: fn(&mut PeerInfo) -> &mut PeerStatus<A>)
    where
        A: Debug
    {
        let Some(info) = self.peers.get_mut(peer) else {
            error!("Invariant violated. Attempted to initialize an unknown peer: {}", peer);
            panic!();
        };
        let status = direction_lambda(info);
        if status.is_known() {
            *status = PeerStatus::Initializing;
        } else {
            error!("Invariant violated. Attempted to initialize an already initialized peer: {} - {:?}", peer, status);
            panic!();
        }
    }

    /// Update a known peer's outgoing status to initializing.
    fn update_peer_to_initializing_outgoing(&mut self, peer: &DeviceId) {
        self.update_peer_to_initializing(peer, |info| &mut info.outgoing_status);
    }

    /// Update a known peer's incoming status to initializing.
    fn update_peer_to_initializing_incoming(&mut self, peer: &DeviceId) {
        self.update_peer_to_initializing(peer, |info| &mut info.incoming_status);
    }

    /// Helper to update a known peer to syncing.
    fn update_peer_to_syncing<A>(&mut self, peer: &DeviceId, direction_lambda: fn(&mut PeerInfo) -> &mut PeerStatus<A>, sender_m: A)
    where
        A: Debug
    {
        let Some(info) = self.peers.get_mut(peer) else {
            error!("Invariant violated. Attempted to initialize an unknown peer: {}", peer);
            panic!();
        };
        let status = direction_lambda(info);
        match status {
            PeerStatus::Initializing => {
                *status = PeerStatus::Syncing(sender_m);
            }
            PeerStatus::Known => {
                error!("Invariant violated. Attempted to initialize an unknown peer: {} - {:?}", peer, status);
                panic!();
            }
            PeerStatus::Syncing(_) => {
                error!("Invariant violated. Attempted to sync an already syncing peer: {} - {:?}", peer, status);
                panic!();
            }
        }
    }

    fn update_peer_to_syncing_incoming(&mut self, peer: &DeviceId) {
        self.update_peer_to_syncing(peer, |info| &mut info.incoming_status, ());
    }

    fn update_peer_to_syncing_outgoing(&mut self, peer: &DeviceId, sender: OutgoingPeerStatus) {
        self.update_peer_to_syncing(peer, |info| &mut info.outgoing_status, sender);
    }

    fn update_outgoing_peer_to_ready(&mut self, peer: &DeviceId) {
        let Some(info) = self.peers.get_mut(peer) else {
            error!("Invariant violated. Attempted to update an unknown peer: {}", peer);
            panic!();
        };
        match info.outgoing_status {
            PeerStatus::Initializing => {
                error!("Invariant violated. Attempted to update a peer that is initializing: {} - {:?}", peer, info.outgoing_status);
                panic!();
            }
            PeerStatus::Known => {
                error!("Invariant violated. Attempted to update a peer that is not syncing: {} - {:?}", peer, info.outgoing_status);
                panic!();
            }
            PeerStatus::Syncing(ref mut status) => {
                status.is_outstanding = false;
            }
        }
    }

    /// Send sync requests to peers.
    fn send_sync_requests(&mut self) {
        fn send_command(i: &mut PeerInfo, message: StoreSyncCommand) {
            let PeerStatus::Syncing(ref mut s) = i.outgoing_status else {
                unreachable!("Already checked that the peer is ready.");
            };

            // Mark as outstanding.
            s.is_outstanding = true;
            s.sender_peer.send(message).expect("TODO");               
        }

        // Get peers (of this store) without outstanding requests.
        let mut peers: Vec<_> = self.peers.iter_mut().filter(|(_,i)| i.is_ready_for_sync()).collect();
        // TODO: Rank and (weighted) randomize the peers.
        let mut rng = thread_rng();
        peers.shuffle(&mut rng);

        // Send requests to peers based on what we need.
        match &self.state_machine {
            StateMachine::DownloadingMetadata { .. } => {
                // Tell the first peer store task to request the metadata.
                peers.iter_mut().take(1).for_each(|(_, i)| {
                    let message = StoreSyncCommand::MetadataHeaderRequest;
                    send_command(i, message);
                });
            }
            StateMachine::DownloadingMerkle { piece_hashes, .. } => {
                let count_upper_bound = 100;
                // TODO: Keep track of (and filter out) which ones are currently requested.
                let needed_hashes = piece_hashes.iter().enumerate().filter_map(|h| if h.1.is_none() { Some(h.0 as u64) } else { None }).chunks(count_upper_bound);
                let mut needed_hashes: Vec<_> = needed_hashes.into_iter().collect();

                // Randomize which peer to request the hashes from.
                needed_hashes.shuffle(&mut rng);

                peers.iter_mut().zip(needed_hashes).for_each(|(p, hashes)| {
                    let message = StoreSyncCommand::MerkleRequest(compress_consecutive_into_ranges(hashes).collect());
                    send_command(p.1, message);
                });
            }
            StateMachine::DownloadingInitialState { metadata, piece_hashes, initial_state } => {
                warn!("TODO: Request initial state from peer");
            }
            StateMachine::Syncing { metadata, piece_hashes, initial_state, ecg_state, decrypted_state } => {
                // TODO: Request updates from peer
                warn!("TODO: Request updates from peer");
            }
        }
    }

    // Handle a sync request for this store from a peer.
    fn handle_metadata_peer_request(&mut self, peer: DeviceId, response_chan: Sender<HandlePeerResponse<MetadataHeader<Hash>>>) {
        if let Some(metadata) = self.metadata() {
            // We have the metadata so share it with the peer.
            response_chan.send(Ok(*metadata)).expect("TODO");
        } else {
            // We don't have the metadata so tell them to wait.
            let (send_chan, recv_chan) = oneshot::channel();
            response_chan.send(Err(recv_chan)).expect("TODO");

            // Register the wait channel.
            self.metadata_subscribers.insert(peer, send_chan); // JP: Safe to drop old one?
        }
    }

    fn handle_merkle_peer_request(&mut self, peer: DeviceId, piece_ids: Vec<Range<u64>>, response_chan: Sender<HandlePeerResponse<Vec<Hash>>>) {

        let hashes = self.piece_hashes().map(|piece_hashes| {
            handle_merkle_peer_request_helper(piece_hashes, &piece_ids)
        });

        if let Some(piece_hashes) = hashes {
            // We have the hashes so share it with the peer.
            response_chan.send(Ok(piece_hashes)).expect("TODO");
        } else {
            // We don't have the hashes so tell them to wait.
            let (send_chan, recv_chan) = oneshot::channel();
            response_chan.send(Err(recv_chan)).expect("TODO");

            // Register the wait channel.
            self.merkle_subscribers.insert(peer, (piece_ids, send_chan)); // JP: Safe to drop old one?
        }
    }

    fn handle_received_metadata(&mut self, peer: DeviceId, metadata: MetadataHeader<Hash>) {
        debug!("Recieved metadata from peer ({peer}): {metadata:?}");

        // Mark peer as ready.
        self.update_outgoing_peer_to_ready(&peer);

        // Validate metadata.
        let is_valid = metadata.validate_store_id(self.store_id());
        warn!("TODO: Validate signature.");

        if !is_valid {
            // TODO: Penalize/blacklist/disconnect peer?
            warn!("TODO: Peer provided invalid metadata.");
            return;
        }

        // Update state.
        self.state_machine = StateMachine::DownloadingMerkle {
            piece_hashes: vec![None; metadata.piece_count() as usize],
            metadata,
        };

        // Send metadata to any peers that are waiting.
        let subs = std::mem::take(&mut self.metadata_subscribers);
        for (sub_peer, sub) in subs {
            let peer_knows = sub_peer == peer;
            let msg = if peer_knows {
                None
            } else {
                Some(metadata)
            };
            sub.send(msg).expect("TODO");
        }

        // Sync with peer(s). Do this for all commands??
        self.send_sync_requests();
    }

    fn metadata(&self) -> Option<&MetadataHeader<Hash>> {
        match &self.state_machine {
            StateMachine::DownloadingMetadata { .. } => {
                None
            }
            StateMachine::DownloadingMerkle { metadata, .. } => {
                Some(metadata)
            }
            StateMachine::DownloadingInitialState { metadata, .. } => {
                Some(metadata)
            }
            StateMachine::Syncing { metadata, .. } => {
                Some(metadata)
            }
        }
    }

    fn piece_hashes(&self) -> Option<&Vec<Hash>> {
        match &self.state_machine {
            StateMachine::DownloadingMetadata { .. } => {
                None
            }
            StateMachine::DownloadingMerkle { .. } => {
                // We shouldn't share pieces until we've verified them.
                None
            }
            StateMachine::DownloadingInitialState { piece_hashes, .. } => {
                Some(piece_hashes)
            }
            StateMachine::Syncing { piece_hashes, .. } => {
                Some(piece_hashes)
            }
        }
    }

    fn handle_received_merkle_pieces(&mut self, peer: DeviceId, piece_ids: Vec<Range<u64>>, their_piece_hashes: Vec<Hash>) {
        warn!("TODO: Keep track if you received different hashes from different peers.");

        // Mark peer as ready.
        self.update_outgoing_peer_to_ready(&peer);

        let piece_ids: Vec<_> = piece_ids.into_iter().flatten().collect();
        if piece_ids.len() != their_piece_hashes.len() {
            warn!("TODO: Peer provided an invalid response");
            return;
        }

        // Update state.
        let piece_hashes = if let StateMachine::DownloadingMerkle { ref mut piece_hashes, .. } = &mut self.state_machine {
            piece_hashes
        } else {
            // We're not downloading anymore so we're done.
            return;
        };
        piece_ids.into_iter().zip(their_piece_hashes).for_each(|(i, hash)| {
            piece_hashes[i as usize] = Some(hash);
        });

        // Check if we're done.
        let piece_hashes_m: Option<Vec<_>> = piece_hashes.iter().cloned().collect();
        let Some(piece_hashes) = piece_hashes_m else {
            return;
        };

        // Validate piece hashes.
        let is_valid = self.metadata().unwrap().merkle_root == util::merkle_root(&piece_hashes);
        if !is_valid {
            // TODO: Penalize/blacklist/disconnect peer?
            warn!("TODO: Peer provided invalid piece hashes.");
            return;
        }

        // Update state.
        self.state_machine = match self.state_machine {
            StateMachine::DownloadingMerkle { metadata, ..} => {
                let initial_state = vec![None; metadata.piece_count() as usize];
                StateMachine::DownloadingInitialState { metadata, piece_hashes, initial_state }
            }
            _ => unreachable!("We already checked that we're downloading merkle"),
        };

        // Send piece hashes to any peers that are waiting.
        let subs = std::mem::take(&mut self.merkle_subscribers);
        let piece_hashes = self.piece_hashes().expect("Unreachable: We just set the piece hashes");
        for (sub_peer, (piece_ids, sub)) in subs {
            let peer_knows = sub_peer == peer;
            let msg = if peer_knows {
                None
            } else {
                Some(handle_merkle_peer_request_helper(piece_hashes, &piece_ids))
            };
            sub.send(msg).expect("TODO");
        }

        // Sync with peer(s). Do this for all commands??
        self.send_sync_requests();
    }
}

// JP: Or should Odyssey own this/peers?
/// Manage peers by ranking them, randomize, potentially connecting to some of them, etc.
async fn manage_peers<OT: OdysseyType, T: CRDT<Time = OT::Time> + Clone + Send + 'static>(
    store: &mut State<OT::ECGHeader<T>, T, OT::StoreId>,
    shared_state: &SharedState<OT::StoreId>,
    send_commands: &UnboundedSender<UntypedStoreCommand<OT::StoreId>>,
)
where
    T::Op: Serialize,
    //OT::ECGHeader<T>::HeaderId : Send,
    //T: Send,
{
    // For now, sync with all (connected?) peers.
    // Don't connect to peers we're already syncing with.
    let peers: Vec<_> = store.peers.iter().filter(|p| p.1.outgoing_status.is_known()).collect();
    let peers: Vec<_> = {
        // Acquire lock on shared state.
        let peer_states = shared_state.peer_state.read().await;
        peers.into_iter().filter_map(|(peer_id, _)| {
            let chan = peer_states.get(peer_id)?;
            Some((*peer_id, chan.clone()))
        }).collect()
    };
    for (peer_id, command_chan) in peers {
        // Mark task as initializing.
        store.update_peer_to_initializing_outgoing(&peer_id);

        // Create closure that spawns task to sync store with peer.
        let send_commands = send_commands.clone();
        let spawn_task = Box::new(move |_party, stream_id, sender, receiver| {
            // Create miniprotocol
            // Spawn task that syncs store with peer.
            // JP: Should run with initiative?
            tokio::spawn(async move {
                // Tell store we're running and send it our channel.
                let (send_peer, recv_peer) =
                    tokio::sync::mpsc::unbounded_channel::<StoreSyncCommand>();

                let register_cmd = UntypedStoreCommand::RegisterOutgoingPeerSyncing {
                    peer: peer_id,
                    send_peer,
                };
                send_commands.send(register_cmd).expect("TODO");

                // Start miniprotocol as server.
                let mp = StoreSync::<OT::StoreId>::new_server(peer_id, recv_peer, send_commands);
                run_miniprotocol_async::<_, OT>(mp, false, stream_id, sender, receiver).await;

                debug!("Store sync with peer (with initiative) exited.")

                // JP: This requires StorePeer protocols in both directions (if both sides want updates from the other party).
                // This has the downside that we may run ECG sync in both directions.. Does it make sense to store StorePeer state in a shared Arc<RWLock>?
                // Separate thread for SCSync?
                // TODO: Setup in both directions, remove initialized check.
                //
                // In the store task, sync state:
                // Based on status,
                //   if we're still downloading the metadata, request the peer to send it 
                //   if we are downloading the initial state, send a download request for a piece(s) we need. How do we do timeouts here though? Perhaps in the StorePeer task?
                //   if we're syncing:
                //      run ECG sync to find the meet
                //      Request all operations after the meet
            })
        });

        // Send request to peer's manager for stream.
        let store_id = store.store_id();
        let cmd = PeerManagerCommand::RequestStoreSync {
            store_id,
            spawn_task,
        };
        command_chan.send(cmd).expect("TODO");
    }
}


/// Run the handler that owns this store and manages its state. This handler is typically run in
/// its own tokio thread.
pub(crate) async fn run_handler<OT: OdysseyType, T: CRDT<Time = OT::Time> + Clone + Send + 'static>(
    mut store: State<OT::ECGHeader<T>, T, OT::StoreId>,
    mut recv_commands: UnboundedReceiver<StoreCommand<OT::ECGHeader<T>, T>>,
    send_commands_untyped: UnboundedSender<UntypedStoreCommand<OT::StoreId>>,
    mut recv_commands_untyped: UnboundedReceiver<UntypedStoreCommand<OT::StoreId>>,
    shared_state: SharedState<OT::StoreId>,
)
where
    <OT as OdysseyType>::ECGHeader<T>: Send + Clone + 'static,
    <<OT as OdysseyType>::ECGHeader<T> as ECGHeader<T>>::Body: ECGBody<T> + Send,
    <<OT as OdysseyType>::ECGHeader<T> as ECGHeader<T>>::HeaderId: Send,
    T::Op: Serialize,
{
    let mut listeners: Vec<UnboundedSender<StateUpdate<OT::ECGHeader<T>, T>>> = vec![];

    // TODO: Check when done
    loop {
        tokio::select! {
            cmd_m = recv_commands.recv() => {
                let Some(cmd) = cmd_m else {
                    error!("Failed to receive StoreCommand");
                    return;
                };

                // // Rank and connect to a few peers.
                // manage_peers::<OT,T>(&mut store, &shared_state).await;

                match cmd {
                    StoreCommand::Apply {
                        operation_header,
                        operation_body,
                    } => {
                        store.state_machine = match store.state_machine {
                            // StateMachine::DownloadingMetadata { store_id } => {
                            //     // JP: Should we ever apply an operation if we're still downloading the store??
                            //     warn!("Is this unreachable?");

                            //     // Rank and connect to a few peers.
                            //     manage_peers::<OT,T>(&mut store, &shared_state).await;

                            //     StateMachine::DownloadingMetadata { store_id }
                            // }
                            // StateMachine::DownloadingMerkle { metadata, piece_hashes } => {
                            //     // JP: Should we ever apply an operation if we're still downloading the store??
                            //     warn!("Is this unreachable?");

                            //     // Rank and connect to a few peers.
                            //     manage_peers::<OT,T>(&mut store, &shared_state).await;

                            //     StateMachine::DownloadingMerkle { metadata, piece_hashes }
                            // }
                            // StateMachine::DownloadingInitialState { metadata, piece_hashes, initial_state } => {
                            //     // JP: Should we ever apply an operation if we're still downloading the store??
                            //     warn!("Is this unreachable?");

                            //     // Rank and connect to a few peers.
                            //     manage_peers::<OT,T>(&mut store, &shared_state).await;

                            //     StateMachine::DownloadingInitialState { metadata, piece_hashes, initial_state }
                            // }
                            StateMachine::Syncing { metadata, piece_hashes, initial_state , ecg_state, decrypted_state } => {
                                let mut ecg_state = ecg_state;
                                let mut decrypted_state = decrypted_state;

                                // Update ECG state.
                                let success = ecg_state.insert_header(operation_header.clone());
                                if !success {
                                    todo!("Invalid header"); // : {:?}", operation_header);
                                }

                                // Update state.
                                // TODO: Get new time?.. Or take it as an argument
                                // Operation ID/time is function of tips, current operation, ...? How do we
                                // do batching? (HeaderId(h) | Self, Index(u8)) ? This requires having all
                                // the batched operations?
                                let causal_state = OT::to_causal_state(&ecg_state);
                                for (time, operation) in
                                    operation_header.zip_operations_with_time(operation_body)
                                {
                                    decrypted_state.latest_state = decrypted_state.latest_state.apply(causal_state, time, operation);
                                }

                                // Send state to subscribers.
                                for l in &listeners {
                                    let snapshot = StateUpdate::Snapshot {
                                        snapshot: decrypted_state.latest_state.clone(),
                                        ecg_state: ecg_state.clone(),
                                    };
                                    l.send(snapshot).expect("TODO");
                                }

                                StateMachine::Syncing { metadata, piece_hashes, initial_state, ecg_state, decrypted_state }
                            }
                            _ => {
                                warn!("JP: Does this ever happen?");
                                store.state_machine
                            }
                        };
                    }
                    StoreCommand::SubscribeState { send_state } => {
                        // Send current state.
                        let snapshot = match &store.state_machine {
                            StateMachine::DownloadingMetadata { .. } => {
                                StateUpdate::Downloading { percent: 0 }
                            }
                            StateMachine::DownloadingMerkle { .. } => {
                                StateUpdate::Downloading { percent: 0 }
                            }
                            StateMachine::DownloadingInitialState { metadata, initial_state, .. } => {
                                let percent = if metadata.initial_state_size == 0 {
                                    0
                                } else {
                                    let downloaded = initial_state.iter().filter(|p| p.is_some()).count() as u64;
                                    downloaded * (metadata.piece_size as u64) / metadata.initial_state_size
                                };
                                StateUpdate::Downloading { percent }
                            }
                            StateMachine::Syncing { ref ecg_state, ref decrypted_state, .. } => {
                                StateUpdate::Snapshot {
                                    snapshot: decrypted_state.latest_state.clone(),
                                    ecg_state: ecg_state.clone(),
                                }
                            }
                        };
                        send_state.send(snapshot).expect("TODO");

                        // Register this subscriber.
                        listeners.push(send_state);
                    }
                }
            }
            cmd_m = recv_commands_untyped.recv() => {
                let Some(cmd) = cmd_m else {
                    error!("Failed to receive UntypedStoreCommand");
                    return;
                };
                match cmd {
                    // Called when:
                    // - Peer manager threads have this thread as a mutual store.
                    UntypedStoreCommand::RegisterPeers { peers } => {
                        debug!("Received UntypedStoreCommand::RegisterPeers: {:?}", peers);

                        // Add peer to known peers.
                        for peer in peers {
                            store.insert_known_peer(peer);
                        }

                        debug!("Peer statuses: {:?}", store.peers);

                        // Spawn sync threads for each shared store.
                        // TODO: Only do this if server?
                        // Check if we already are syncing these.
                        manage_peers::<OT,T>(&mut store, &shared_state, &send_commands_untyped).await;
                    }
                    // Sets up task to respond to a request to sync this store from a peer (without initiative).
                    // Called when:
                    // - The peer requests we sync this store with them
                    UntypedStoreCommand::SyncWithPeer { peer, response_chan } => {
                        debug!("Received UntypedStoreCommand::SyncWithPeer: {:?}", peer);

                        // Insert peer as known if we don't know them (since they're requesting the store).
                        store.insert_known_peer(peer);

                        let response = {
                            // Check if already syncing with this peer. (JP: What if they're both already "Initializing"? Potential race condition where they don't sync)
                            if let Some(status) = store.peers.get(&peer) {
                                if status.incoming_status.is_known() {
                                    // Mark task as initializing.
                                    store.update_peer_to_initializing_incoming(&peer);

                                    // Create closure that spawns task to sync store with peer.
                                    let send_commands_untyped = send_commands_untyped.clone();
                                    let spawn_task: Box<SpawnMultiplexerTask> = Box::new(move |party, stream_id, sender, receiver| {
                                        // Create miniprotocol
                                        // Spawn task that syncs store with peer.
                                        // JP: Should run without initiative so that other peer can setup their handler?
                                        tokio::spawn(async move {
                                            debug!("Sync with peer (without initiative).");

                                            // Tell store we're running.
                                            let register_cmd = UntypedStoreCommand::RegisterIncomingPeerSyncing {
                                                peer,
                                            };
                                            send_commands_untyped.send(register_cmd).expect("TODO");

                                            // Start miniprotocol as client.
                                            let mp = StoreSync::<OT::StoreId>::new_client(peer, send_commands_untyped);
                                            run_miniprotocol_async::<_, OT>(mp, true, stream_id, sender, receiver).await;
                                            debug!("Store sync with peer (without initiative) exited.")
                                        })
                                    });
                                    Some(spawn_task)
                                } else {
                                    debug!("Store is already running");
                                    None
                                }
                            } else {
                                unreachable!("Don't know this peer.");
                                // JP: This should be impossible now.
                                None
                            }
                        };

                        response_chan.send(response).or(Err(())).expect("TODO");
                    }
                    UntypedStoreCommand::RegisterOutgoingPeerSyncing{ peer, send_peer } => {
                        // JP: Maybe send_peer actually isn't needed??? We could construct oneshots???
                        // Update peer's state to syncing and register channel.
                        let outgoing_status = OutgoingPeerStatus {
                            sender_peer: send_peer,
                            is_outstanding: false,
                        };
                        store.update_peer_to_syncing_outgoing(&peer, outgoing_status);

                        // Sync with peer(s). Do this for all commands??
                        store.send_sync_requests();
                    }
                    UntypedStoreCommand::RegisterIncomingPeerSyncing{ peer } => {
                        // JP: Maybe this actually isn't needed??? We could construct oneshots for every request..

                        // Update peer's state to syncing and register channel.
                        store.update_peer_to_syncing_incoming(&peer);
                    }
                    UntypedStoreCommand::HandleMetadataPeerRequest(HandlePeerRequest { peer, request, response_chan }) => {
                        store.handle_metadata_peer_request(peer, response_chan);
                    }
                    UntypedStoreCommand::HandleMerklePeerRequest(HandlePeerRequest { peer, request, response_chan }) => {
                        store.handle_merkle_peer_request(peer, request, response_chan);
                    }
                    UntypedStoreCommand::ReceivedMetadata { peer, metadata } => {
                        store.handle_received_metadata(peer, metadata);
                    }
                    UntypedStoreCommand::ReceivedMerklePieces { peer, ranges, pieces } => {
                        store.handle_received_merkle_pieces(peer, ranges, pieces);
                    }
                }
            }
        }
    }
    debug!("Store thread exiting.");
}

pub(crate) enum StoreCommand<Header: ECGHeader<T>, T> {
    Apply {
        operation_header: Header,     // <Hash, T>,
        operation_body: Header::Body, // <Hash, T>,
    },
    // TODO: Support unsubscribe.
    SubscribeState {
        send_state: UnboundedSender<StateUpdate<Header, T>>,
    },
}

pub enum StateUpdate<Header: ECGHeader<T>, T> {
    Downloading {
        percent: u64,
    },
    Snapshot {
        snapshot: T,
        ecg_state: ecg::State<Header, T>,
        // TODO: ECG DAG
    },
}

// trait UntypedCRDT: CRDT<Op = dyn Any, Time = dyn Any> {} // Any + Sized + 'static +  
// trait UntypedECGHeader: ECGHeader<dyn Any> {} // Any + Sized + 'static + 
// 
// struct UntypedStoreCommand(StoreCommand<dyn UntypedECGHeader, dyn UntypedCRDT>);
// struct UntypedStoreCommand(StoreCommand<dyn UntypedECGHeader, dyn Any>);

type HandlePeerResponse<Response> = Result<
        Response,
        oneshot::Receiver<Option<Response>>,
    >;

/// Untyped variant of `StoreCommand` since existentials don't work.
// #[derive(Debug)]
pub(crate) enum UntypedStoreCommand<Hash> {
    /// Register the discovered peers.
    RegisterPeers {
        peers: Vec<DeviceId>,
    },
    /// Request store to sync with peer. Store can refuse.
    SyncWithPeer {
        peer: DeviceId,
        response_chan: oneshot::Sender<Option<Box<SpawnMultiplexerTask>>>,
    },
    RegisterOutgoingPeerSyncing {
        peer: DeviceId,
        send_peer: UnboundedSender<StoreSyncCommand>,
    },
    HandleMetadataPeerRequest(HandlePeerRequest<(), v0::MetadataHeader<Hash>>),
    HandleMerklePeerRequest(HandlePeerRequest<Vec<Range<u64>>, Vec<Hash>>),
    RegisterIncomingPeerSyncing {
        peer: DeviceId,
    },
    ReceivedMetadata {
        peer: DeviceId,
        metadata: MetadataHeader<Hash>,
    },
    ReceivedMerklePieces {
        peer: DeviceId,
        ranges: Vec<Range<u64>>,
        pieces: Vec<Hash>,
    }
}

pub(crate) struct HandlePeerRequest<Request, Response> {
    pub(crate) peer: DeviceId,
    pub(crate) request: Request, // MsgStoreSyncRequest,
    /// Return either the result or a channel to wait on for the response.
    pub(crate) response_chan: oneshot::Sender<HandlePeerResponse<Response>>,
}

fn handle_merkle_peer_request_helper<H: Copy>(piece_hashes: &Vec<H>, piece_ids: &Vec<Range<u64>>) -> Vec<H> {
    warn!("TODO: check ranges are in bounds or return error");
    let hashes: Vec<_> = piece_ids.iter().cloned().flatten().map(|i| *piece_hashes.get(i as usize).expect("TODO: Properly handle invalid requests")).collect();
    hashes
}
