use odyssey_crdt::CRDT;
use serde::Serialize;
use tokio::sync::mpsc::{UnboundedSender, UnboundedReceiver};
use tracing::{debug, error, warn};
use std::collections::BTreeSet;
use typeable::{TypeId, Typeable};

use crate::{core::OdysseyType, store::ecg::{ECGBody, ECGHeader}, util};

pub mod ecg;
pub mod v0; // TODO: Move this to network::protocol

pub use v0::{MetadataBody, MetadataHeader, Nonce};


pub struct State<Header: ecg::ECGHeader<T>, T: CRDT, Hash> {
    // Peers that also have this store.
    peers: BTreeSet<()>, // PeerId>,
    state_machine: StateMachine<Header, T, Hash>,
}

// States are:
// - Initializing - Setting up the thread that owns the store (not defined here).
// - Downloading - Don't have the header so we're downloading it.
// - Syncing - Have the header and syncing updates between peers.
pub enum StateMachine<Header: ecg::ECGHeader<T>, T: CRDT, Hash> {
    Downloading {
        store_id: Hash,
    },
    Syncing {
        store_header: MetadataHeader<Hash>,
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

        let state_machine = StateMachine::Syncing {
            store_header,
            ecg_state: ecg::State::new(),
            decrypted_state, // : Some(decrypted_state),
        };
        State {
            peers: BTreeSet::new(),
            state_machine
        }
    }

    /// Create a new store with the given store id that is downloading the store's header.
    pub(crate) fn new_downloading(store_id: Hash) -> Self {
        let state_machine = StateMachine::Downloading {
            store_id
        };

        State {
            peers: BTreeSet::new(),
            state_machine,
        }
    }

    pub fn store_id(&self) -> Hash {
        match &self.state_machine {
            StateMachine::Downloading{store_id} => {
                *store_id
            }
            StateMachine::Syncing { store_header, .. } => {
                store_header.store_id()
            }
        }
    }
}

// JP: Or should Odyssey own this/peers?
/// Manage peers by ranking them, randomize, potentially connecting to some of them, etc.
fn manage_peers<OT: OdysseyType, T: CRDT<Time = OT::Time> + Clone + Send + 'static>(
    store: &mut State<OT::ECGHeader<T>, T, OT::StoreId>,
)
where
    T::Op: Serialize,
{
    warn!("TODO: Connect to peers' store, etc");
}


/// Run the handler that owns this store and manages its state. This handler is typically run in
/// its own tokio thread.
pub(crate) async fn run_handler<OT: OdysseyType, T: CRDT<Time = OT::Time> + Clone + Send + 'static>(
    mut store: State<OT::ECGHeader<T>, T, OT::StoreId>,
    mut recv_commands: UnboundedReceiver<StoreCommand<OT::ECGHeader<T>, T>>,
    mut recv_commands_untyped: UnboundedReceiver<UntypedStoreCommand>,
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
                match cmd {
                    StoreCommand::Apply {
                        operation_header,
                        operation_body,
                    } => {
                        store.state_machine = match store.state_machine {
                            StateMachine::Downloading { store_id } => {
                                // Rank and connect to a few peers.
                                manage_peers::<OT,T>(&mut store);

                                StateMachine::Downloading { store_id }
                            }
                            StateMachine::Syncing { store_header, ecg_state, decrypted_state } => {
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

                                StateMachine::Syncing { store_header, ecg_state, decrypted_state }
                            }
                        };
                    }
                    StoreCommand::SubscribeState { send_state } => {
                        // Send current state.
                        let snapshot = match &store.state_machine {
                            StateMachine::Downloading { .. } => {
                                StateUpdate::Downloading
                            }
                            StateMachine::Syncing { store_header: _, ref ecg_state, ref decrypted_state } => {
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
                    UntypedStoreCommand::RegisterPeers { peers } => {
                        // Add peer to known peers.
                        for peer in peers {
                            store.peers.insert(peer);
                        }

                        // Spawn sync threads for each shared store.
                        // TODO: Only do this if server?
                        // Check if we already are syncing these.
                        manage_peers::<OT,T>(&mut store);
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
    Downloading,
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
#[derive(Clone, Debug)]
pub enum UntypedStoreCommand {
    /// Register the discovered peers.
    RegisterPeers {
        peers: Vec<()>,
    }
}
