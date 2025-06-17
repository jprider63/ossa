use odyssey_crdt::CRDT;
use serde::Serialize;
use tokio::{sync::{mpsc::{UnboundedReceiver, UnboundedSender}, oneshot}, task::JoinHandle};
use tracing::{debug, error, warn};
use std::collections::{BTreeMap, BTreeSet};
use typeable::{TypeId, Typeable};

use crate::{auth::DeviceId, core::{OdysseyType, SharedState}, network::{multiplexer::{run_miniprotocol_async, SpawnMultiplexerTask}, protocol::MiniProtocol}, protocol::{manager::v0::PeerManagerCommand, store_peer::v0::{StoreSync, StoreSyncCommand}}, store::ecg::{ECGBody, ECGHeader}, util};

pub mod ecg;
pub mod v0; // TODO: Move this to network::protocol

pub use v0::{MetadataBody, MetadataHeader, Nonce};

pub struct State<Header: ecg::ECGHeader<T>, T: CRDT, Hash> {
    // Peers that also have this store (that we are potentially connected to?).
    peers: BTreeMap<DeviceId, PeerInfo>, // BTreeSet<DeviceId>,
    state_machine: StateMachine<Header, T, Hash>,
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
    }
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
    incoming_status: PeerStatus,
    /// Status of outgoing sync status to peer.
    outgoing_status: PeerStatus,
}

/// Status of peers who we are potentially syncing this store with.
#[derive(Debug)]
pub(crate) enum PeerStatus {
    /// Peer is known and likely connected to, but is not syncing this store.
    Known, // JP: Inactive?
    /// Setting up the thread that syncs the store with the peer. It's possible that the peer will reject the sync request.
    Initializing,
    // {
    //     task: JoinHandle<()>,
    // },
    /// The thread that syncs the store with the peer is syncing.
    Syncing, // JP: Running instead?

//     /// Peers that we are connected to, but are not syncing this store. It's possible that these connections have dropped.
//     Known(), // TODO: Last known IP address, port, statistics (latency, bandwidth, ...). JP: Should some of this be stored globally?
//     /// Peers that we are connected to, but are not syncing this store. It's possible that these connections have dropped.
//     Connected(), 
//     /// Peers that we are connected to and are syncing this store. It's possible that these connections have dropped.
//     Active(), 
}

impl PeerStatus {
    fn is_known(&self) -> bool {
        if let PeerStatus::Known = self {
            true
        } else {
            false
        }
    }
}

// impl PeerStatus {
//     pub(crate) fn is_connected(&self) -> bool {
//         match self {
//             PeerStatus::Connected() => true,
//             _ => false,
//         }
//     }
// }

impl<Header: ecg::ECGHeader<T>, T: CRDT, Hash: util::Hash> State<Header, T, Hash> {
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
            state_machine
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
    fn update_peer_to_initializing(&mut self, peer: &DeviceId, direction_lambda: fn(&mut PeerInfo) -> &mut PeerStatus) {
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
}

// JP: Or should Odyssey own this/peers?
/// Manage peers by ranking them, randomize, potentially connecting to some of them, etc.
async fn manage_peers<OT: OdysseyType, T: CRDT<Time = OT::Time> + Clone + Send + 'static>(
    store: &mut State<OT::ECGHeader<T>, T, OT::StoreId>,
    shared_state: &SharedState<OT::StoreId>,
    // send_commands: UnboundedSender<StoreCommand<OT::ECGHeader<T>, T>>,
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
        let spawn_task = Box::new(|_party, stream_id, sender, receiver| {
            // Create miniprotocol
            // Spawn task that syncs store with peer.
            // JP: Should run with initiative?
            tokio::spawn(async move {
                // // Send cmd to store to set peer's status as syncing.
                // Tell store we're running and send it our channel.
                let (send_peer, recv_peer) =
                    tokio::sync::mpsc::unbounded_channel::<StoreSyncCommand>();

                // let register_cmd = StoreCommand::<OT::ECGHeader<T>, T>::RegisterOutgoingPeerSyncing {
                //     send_peer,
                // };
                // send_commands.send(register_cmd).expect("TODO");
                todo!("Tell store we're running and send it our channel");

                // Start miniprotocol as server.
                let mp = StoreSync::<OT::StoreId>::new_server(recv_peer);
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
    mut recv_commands_untyped: UnboundedReceiver<UntypedStoreCommand>,
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
                    StoreCommand::RegisterOutgoingPeerSyncing{ send_peer } => {
                        let peer_id = todo!();
                        // Update peer's state to syncing and register 
                        todo!("TODO");

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
                        manage_peers::<OT,T>(&mut store, &shared_state).await;
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
                                    let spawn_task: Box<SpawnMultiplexerTask> = Box::new(|party, stream_id, sender, receiver| {
                                        // Create miniprotocol
                                        // Spawn task that syncs store with peer.
                                        // JP: Should run without initiative so that other peer can setup their handler?
                                        tokio::spawn(async move {
                                            warn!("TODO: Sync with peer (without initiative).")
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
    RegisterOutgoingPeerSyncing {
        send_peer: UnboundedSender<StoreSyncCommand>,
    }
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

/// Untyped variant of `StoreCommand` since existentials don't work.
// #[derive(Debug)]
pub(crate) enum UntypedStoreCommand {
    /// Register the discovered peers.
    RegisterPeers {
        peers: Vec<DeviceId>,
    },
    /// Request store to sync with peer. Store can refuse.
    SyncWithPeer {
        peer: DeviceId,
        response_chan: oneshot::Sender<Option<Box<SpawnMultiplexerTask>>>,
    },
}
