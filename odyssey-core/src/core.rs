use odyssey_crdt::time::CausalState;
// use futures::{SinkExt, StreamExt};
// use futures_channel::mpsc::{UnboundedReceiver, UnboundedSender};
use odyssey_crdt::CRDT;
use serde::{Deserialize, Serialize};
use tokio::sync::{watch, RwLock};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{Debug, Display};
use std::marker::PhantomData;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::sync::Arc;
use std::thread;
use tokio::net::{TcpListener, TcpStream};
use tokio::runtime::Runtime;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::task::JoinHandle;
use tokio_util::codec::{self, LengthDelimitedCodec};
use tracing::{debug, error, info, warn};
use typeable::Typeable;

use crate::auth::{generate_identity, DeviceId, Identity};
use crate::network::protocol::{run_handshake_client, run_handshake_server, HandshakeError};
use crate::protocol::manager::v0::PeerManagerCommand;
use crate::protocol::MiniProtocolArgs;
use crate::storage::Storage;
use crate::store::{self, StateUpdate, StoreCommand, UntypedStoreCommand};
use crate::store::ecg::{self, ECGBody, ECGHeader};
use crate::util::{self, TypedStream};

pub struct Odyssey<OT: OdysseyType> {
    /// Thread running the Odyssey server.
    thread: thread::JoinHandle<()>,
    // command_channel: UnboundedSender<OdysseyCommand>,
    tokio_runtime: Runtime,
    /// Active stores.
    // stores: BTreeMap<OT::StoreId,ActiveStore>,
    active_stores: watch::Sender<StoreStatuses<OT::StoreId>>, // JP: Make this encode more state that other's may want to subscribe to?
    shared_state: SharedState<OT::StoreId>, // JP: Could have another thread own and manage this state
                                  // instead?
    phantom: PhantomData<OT>,
    identity_keys: Identity,
}
pub type StoreStatuses<StoreId> = BTreeMap<StoreId, StoreStatus<StoreId>>; // Rename this MiniProtocolArgs?

// pub enum StoreStatus<O: OdysseyType, T: CRDT<Time = O::Time>>
// where
//     T::Op: Serialize,
pub enum StoreStatus<StoreId> {
    // Store is initializing and async handler is being created.
    Initializing,
    // Store's async handler is running.
    Running {
        store_handle: JoinHandle<()>, // JP: Does this belong here? The state is owned here, but
                                      // the miniprotocols probably don't need to block waiting on
                                      // it...
        // send_command_chan: UnboundedSender<StoreCommand<O::ECGHeader<T>, T>>,
        // https://www.reddit.com/r/rust/comments/1exjiab/the_amazing_pattern_i_discovered_hashmap_with/
        // send_command_chan: UnboundedSender<StoreCommand<store::ecg::v0::Header<dyn Hash, dyn CRDT>, dyn CRDT>>,
        // send_command_chan: UnboundedSender<UntypedStoreCommand>,
        send_command_chan: UnboundedSender<UntypedStoreCommand<StoreId>>,
    },
}

#[derive(Clone,Debug)]
/// Odyssey state that is shared across multiple tasks.
pub(crate) struct SharedState<StoreId> {
    pub(crate) peer_state: Arc<RwLock<BTreeMap<DeviceId, UnboundedSender<PeerManagerCommand<StoreId>>>>>,
}


impl<StoreId> StoreStatus<StoreId> {
    pub(crate) fn is_initializing(&self) -> bool {
        match self {
            StoreStatus::Initializing => true,
            StoreStatus::Running {..} => false,
        }
    }

    pub(crate) fn is_initialized(&self) -> bool {
        !self.is_initializing()
    }

    pub(crate) fn command_channel(&self) -> Option<&UnboundedSender<UntypedStoreCommand<StoreId>>> {
        match self {
            StoreStatus::Initializing => None,
            StoreStatus::Running {send_command_chan, ..} => Some(send_command_chan),
        }
    }
}

impl<OT: OdysseyType> Odyssey<OT> {
    // Start odyssey.
    pub fn start(config: OdysseyConfig) -> Self {
        // TODO: Load identity or take it as an argument.
        let identity_keys = generate_identity();

        // // Create channels to communicate with Odyssey thread.
        // let (send_odyssey_commands, mut recv_odyssey_commands) = futures_channel::mpsc::unbounded();
        let (active_stores, active_stores_receiver) = watch::channel(BTreeMap::new());
        let device_id = DeviceId::new(identity_keys.auth_key().verifying_key());

        let shared_state_ = SharedState {
            peer_state: Arc::new(RwLock::new(BTreeMap::new())),
        };

        // Start async runtime.
        let runtime = match tokio::runtime::Runtime::new() {
            Ok(r) => r,
            Err(err) => {
                error!("Failed to initialize tokio runtime: {}", err);
                todo!()
            }
        };
        let runtime_handle = runtime.handle().clone();
        let shared_state = shared_state_.clone();

        // Spawn server thread.
        let odyssey_thread = thread::spawn(move || {
            runtime_handle.block_on(async move {
                // Start listening for connections.
                let address = SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 1), config.port);
                let listener = match TcpListener::bind(&address).await {
                    Ok(l) => l,
                    Err(err) => {
                        error!("Failed to bind to port ({}): {}", &address, err);
                        return;
                    }
                };

                // // Handle commands from application.
                // tokio::spawn(async move {
                //     while let Some(cmd) = recv_odyssey_commands.next().await {
                //         todo!();
                //     }

                //     unreachable!();
                // });

                info!("Starting server");
                loop {
                    // Accept connection.
                    let (tcpstream, peer) = match listener.accept().await {
                        Ok(r) => r,
                        Err(err) => {
                            error!("Failed to accept connection: {}", err);
                            continue;
                        }
                    };
                    info!("Accepted connection from peer: {}", peer);
                    // Spawn async.
                    let active_stores = active_stores_receiver.clone();
                    // let device_id = DeviceId::new(identity_keys.auth_key().verifying_key());
                    let shared_state = shared_state.clone();


                    let future_handle = tokio::spawn(async move {
                        // let (read_stream, write_stream) = tcpstream.split();
                        let stream = codec::Framed::new(tcpstream, LengthDelimitedCodec::new());

                        // TODO XXX
                        // Handshake.
                        // Diffie Hellman? TLS?
                        // Authenticate peer's public key?
                        let mut stream = TypedStream::new(stream);
                        let handshake_result = run_handshake_server(&mut stream, &device_id).await;
                        let stream = stream.finalize().into_inner();

                        let handshake_result = match handshake_result {
                            Ok(r) => r,
                            Err(HandshakeError::ConnectingToSelf) => {
                                info!("Disconnecting. Attempting to connect to ourself.");
                                return;
                            }
                        };

                        info!("Handshake complete with peer: {}", handshake_result.peer_id());
                        // Store peer in state.
                        if let Some(recv) = initiate_peer(handshake_result.peer_id(), &shared_state).await {
                            // Start miniprotocols.
                            let args = MiniProtocolArgs::new(handshake_result.peer_id(), active_stores, recv);
                            handshake_result.version().run_miniprotocols_server::<OT>(stream, args).await;
                        } else {
                            info!("Disconnecting. Already connected to peer: {}", handshake_result.peer_id());
                        }
                    });

                }
            });
        });



        // TODO: Store identity key

        Odyssey {
            thread: odyssey_thread,
            // command_channel: send_odyssey_commands,
            tokio_runtime: runtime,
            active_stores,
            phantom: PhantomData,
            shared_state: shared_state_,
            identity_keys,
        }
    }

    pub fn create_store<T, S: Storage>(
        &self,
        initial_state: T,
        _storage: S,
    ) -> StoreHandle<OT, T>
    where
        T: CRDT<Time = OT::Time> + Clone + Send + 'static + Typeable + Serialize + for<'d> Deserialize<'d>,
        T::Op: Serialize,
        <OT as OdysseyType>::ECGHeader<T>: Send + Clone + 'static,
        <<OT as OdysseyType>::ECGHeader<T> as ECGHeader<T>>::Body: Send + ECGBody<T>,
        <<OT as OdysseyType>::ECGHeader<T> as ECGHeader<T>>::HeaderId: Send,
    {
        // Create store by generating nonce, etc.
        let store = store::State::<OT::ECGHeader<T>, T, OT::StoreId>::new_syncing(initial_state.clone());
        let store_id = store.store_id();

        // Check if this store id already exists and try again if there's a conflict.
        // Otherwise, mark this store as initializing.
        let mut already_exists = false;
        self.active_stores.send_if_modified(|active_stores| {
            let res = active_stores.try_insert(store_id, StoreStatus::Initializing);
            if res.is_err() {
                already_exists = true;
            }
            false
        });
        if already_exists {
            // This will generate a new nonce if there's a conflict.
            return self.create_store(initial_state, _storage);
        }

        // Launch the store.
        let store_handle = self.launch_store(store_id, store);
        info!("Created store: {}", store_id);
        store_handle
    }

    pub fn connect_to_store<T>(
        &self,
        store_id: OT::StoreId,
        // storage: S,
    ) -> StoreHandle<OT, T>
    where
        <OT as OdysseyType>::ECGHeader<T>: Send + Clone + 'static,
        <<OT as OdysseyType>::ECGHeader<T> as ECGHeader<T>>::Body: Send + ECGBody<T>,
        <<OT as OdysseyType>::ECGHeader<T> as ECGHeader<T>>::HeaderId: Send,
        T: CRDT<Time = OT::Time> + Clone + Send + 'static + for<'d> Deserialize<'d>,
        T::Op: Serialize,
    {
        // Check if store is already active.
        // If it isn't, mark it as initializing and continue.
        let mut is_active = false;
        self.active_stores.send_if_modified(|active_stores| {
            let res = active_stores.try_insert(store_id, StoreStatus::Initializing);
            if res.is_err() {
                is_active = true;
            }
            false
        });
        if is_active {
            // TODO: Get handle of existing store.
            return todo!();
        }

        // TODO:
        // - Load store from disk if we have it locally.
        // Spawn async handler.
        let state = store::State::new_downloading(store_id);
        let store_handler = self.launch_store(store_id, state);
        debug!("Joined store: {}", store_id);
        store_handler


        // - Add it to our active store set with the appropriate status.
        //
        // Update our peers + sync with them? This is automatic?
        //
        // TODO: Set status as initializing in create_store too
    }

    // Connect to network.
    pub fn connect() {
        todo!("Turn on network connection")
    }

    // Disconnect from network.
    pub fn disconnect() {
        todo!("Turn off network connections (work offline)")
    }

    fn device_id(&self) -> DeviceId {
        DeviceId::new(self.identity_keys.auth_key().verifying_key())
    }


    // Connect to a peer over ipv4.
    pub fn connect_to_peer_ipv4(&self, address: SocketAddrV4) {
        let active_stores = self.active_stores.subscribe();
        let device_id = self.device_id();
        let shared_state = self.shared_state.clone();

        // Spawn async.
        let future_handle = self.tokio_runtime.spawn(async move {
            // Attempt to connect to peer, returning message on failure.
            let mut stream = match TcpStream::connect(address).await {
                Ok(tcpstream) => {
                    let stream = codec::Framed::new(tcpstream, LengthDelimitedCodec::new());
                    TypedStream::new(stream)
                }
                Err(err) => {
                    todo!("TODO: Log error");
                    return;
                }
            };

            // Run client handshake.
            let handshake_result = run_handshake_client(&mut stream, &device_id).await;
            let stream = stream.finalize().into_inner();
            debug!("Connected to server!");

            let handshake_result = match handshake_result {
                Ok(r) => r,
                Err(HandshakeError::ConnectingToSelf) => {
                    info!("Disconnecting. Attempting to connect to ourself.");
                    return;
                }
            };

            info!("Handshake complete with peer: {}", handshake_result.peer_id());
            // Store peer in state.
            if let Some(recv) = initiate_peer(handshake_result.peer_id(), &shared_state).await {
                // Start miniprotocols.
                debug!("Start miniprotocols");
                let args = MiniProtocolArgs::new(handshake_result.peer_id(), active_stores, recv);
                handshake_result.version().run_miniprotocols_client::<OT>(stream, args).await;
            } else {
                info!("Disconnecting. Already connected to peer: {}", handshake_result.peer_id());
            }
        });

        // Return channel with peer connection status.
    }

    // TODO: Separate state (that keeps state, syncs with other peers, etc) and optional user API (that sends state updates)?
    fn launch_store<T>(
        &self,
        store_id: OT::StoreId,
        store: store::State<OT::ECGHeader<T>, T, OT::StoreId>,
    ) -> StoreHandle<OT, T>
    where
        <OT as OdysseyType>::ECGHeader<T>: Send + Clone + 'static,
        <<OT as OdysseyType>::ECGHeader<T> as ECGHeader<T>>::Body: ECGBody<T> + Send,
        <<OT as OdysseyType>::ECGHeader<T> as ECGHeader<T>>::HeaderId: Send,
        T::Op: Serialize,
        T: CRDT<Time = OT::Time> + Clone + Send + 'static + for<'d> Deserialize<'d>,
    {
        // Initialize storage for this store.

        // Create channels to handle requests and send updates.
        let (send_commands, recv_commands) =
            tokio::sync::mpsc::unbounded_channel::<store::StoreCommand<OT::ECGHeader<T>, T>>();
        let (send_commands_untyped, recv_commands_untyped) =
            tokio::sync::mpsc::unbounded_channel::<store::UntypedStoreCommand<OT::StoreId>>();


        // Add to DHT

        // Spawn routine that owns this store.

        let shared_state = self.shared_state.clone();
        let send_commands_untyped_ = send_commands_untyped.clone();
        let future_handle = self.tokio_runtime.spawn(async move {
            store::run_handler::<OT, T>(store, recv_commands, send_commands_untyped_, recv_commands_untyped, shared_state).await;
        });

        // Register this store.
        self.active_stores.send_if_modified(|active_stores| {
            let _ = active_stores.insert(store_id, StoreStatus::Running {
                store_handle: future_handle,
                send_command_chan: send_commands_untyped,
            });
            true
        });

        StoreHandle {
            // future_handle,
            send_command_chan: send_commands,
            phantom: PhantomData,
        }
    }

}

/// Initiates a peer by creating a channel to send commands and by inserting it into the shared state. On success, returns the receiver. If the peer already exists, fails with `None`.
async fn initiate_peer<StoreId>(peer_id: DeviceId, shared_state: &SharedState<StoreId>) -> Option<UnboundedReceiver<PeerManagerCommand<StoreId>>> {
    let (send, recv) = tokio::sync::mpsc::unbounded_channel();
    let inserted = {
        let mut w = shared_state.peer_state.write().await;
        w.try_insert(peer_id, send).is_ok()
    };
    if inserted {
        Some(recv)
    }
    else {
        // JP: Record if we're already connected to the peer?
        None
    }

}

#[derive(Clone, Copy)]
pub struct OdysseyConfig {
    // IPv4 port to run Odyssey on.
    pub port: u16,
}

pub struct StoreHandle<O: OdysseyType, T: CRDT<Time = O::Time>>
where
    T::Op: Serialize,
{
    // future_handle: JoinHandle<()>, // JP: Maybe this should be owned by `Odyssey`?
    send_command_chan: UnboundedSender<StoreCommand<O::ECGHeader<T>, T>>,
    phantom: PhantomData<O>,
}

/// Trait to define newtype wrapers that instantiate type families required by Odyssey.
pub trait OdysseyType: 'static {
    type StoreId: util::Hash + Debug + Display + Copy + Ord + Send + Sync + 'static + Serialize + for<'a> Deserialize<'a>; // Hashable instead of AsRef???
    type Hash: util::Hash + Debug + Display + Copy + Ord + Send + Sync + 'static + Serialize + for<'a> Deserialize<'a>; // Hashable instead of AsRef???
    type ECGHeader<T: CRDT<Time = Self::Time, Op: Serialize>>: store::ecg::ECGHeader<T> + Debug + Send;
    type Time;
    type CausalState<T: CRDT<Time = Self::Time, Op: Serialize>>: CausalState<Time = Self::Time>;
    // type OperationId;
    // type Hash: Clone + Copy + Debug + Ord + Send;

    // TODO: This should be refactored and provided automatically.
    fn to_causal_state<T: CRDT<Time = Self::Time, Op: Serialize>>(
        st: &store::ecg::State<Self::ECGHeader<T>, T>,
    ) -> &Self::CausalState<T>;
}


impl<O: OdysseyType, T: CRDT<Time = O::Time>> StoreHandle<O, T>
where
    T::Op: Serialize,
{
    pub fn apply(
        &mut self,
        parents: BTreeSet<<<O as OdysseyType>::ECGHeader<T> as ECGHeader<T>>::HeaderId>,
        op: T::Op,
    ) -> T::Time
    where
        <<O as OdysseyType>::ECGHeader<T> as ECGHeader<T>>::Body: ECGBody<T>,
    {
        self.apply_batch(parents, vec![op]).pop().unwrap()
    }

    // TODO: Don't take parents as an argument. Pull it from the state. XXX
    pub fn apply_batch(
        &mut self,
        parents: BTreeSet<<<O as OdysseyType>::ECGHeader<T> as ECGHeader<T>>::HeaderId>,
        op: Vec<T::Op>,
    ) -> Vec<T::Time>
    where
        <<O as OdysseyType>::ECGHeader<T> as ECGHeader<T>>::Body: ECGBody<T>,
    {
        // TODO: Divide into 256 operation chunks.
        if op.is_empty() {
            return vec![];
        }

        // Create ECG header and body.
        let body =
            <<<O as OdysseyType>::ECGHeader<T> as ECGHeader<T>>::Body as ECGBody<T>>::new_body(op);
        let header = O::ECGHeader::new_header(parents, &body);
        let times = header.get_operation_times(&body);

        self.send_command_chan.send(StoreCommand::Apply {
            operation_header: header,
            operation_body: body,
        }).expect("TODO");

        times
    }

    pub fn subscribe_to_state(&mut self) -> UnboundedReceiver<StateUpdate<O::ECGHeader<T>, T>> {
        let (send_state, recv_state) = tokio::sync::mpsc::unbounded_channel();
        self.send_command_chan
            .send(StoreCommand::SubscribeState { send_state }).expect("TODO");

        recv_state
    }
}

// pub enum OdysseyCommand {
//     CreateStore {
//         // Since Rust doesn't have existentials...
//         initial_state: (), // Box<Dynamic>, // T
//         storage: Box<dyn Storage + Send>,
//     },
// }

// fn handle_odyssey_command() {
// }
