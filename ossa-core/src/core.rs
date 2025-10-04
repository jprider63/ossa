use ossa_crdt::time::CausalState;
use ossa_crdt::CRDT;
use ossa_typeable::Typeable;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{Debug, Display};
use std::marker::PhantomData;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::num::NonZero;
use std::sync::Arc;
use std::thread;
use tokio::net::{TcpListener, TcpStream};
use tokio::runtime::Runtime;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::sync::{watch, RwLock};
use tokio::task::JoinHandle;
use tokio_util::codec::{self, LengthDelimitedCodec};
use tracing::{debug, error, info, warn};

use crate::auth::{DeviceId, identity::DevicePrivateKeys};
use crate::network::protocol::{run_handshake_client, run_handshake_server, HandshakeError};
use crate::protocol::manager::v0::PeerManagerCommand;
use crate::protocol::MiniProtocolArgs;
use crate::storage::Storage;
use crate::store::dag::{ECGBody, ECGHeader};
use crate::store::{self, StateUpdate, StoreCommand, UntypedStoreCommand};
use crate::time::ConcretizeTime;
use crate::util::{self, TypedStream};

pub struct Ossa<OT: OssaType> {
    /// Thread running the Ossa server.
    thread: thread::JoinHandle<()>,
    // command_channel: UnboundedSender<OssaCommand>,
    tokio_runtime: Runtime,
    /// Active stores.
    // stores: BTreeMap<OT::StoreId,ActiveStore>,
    active_stores: watch::Sender<
        StoreStatuses<OT::StoreId, OT::Hash, <OT::ECGHeader as ECGHeader>::HeaderId, OT::ECGHeader>,
    >, // JP: Make this encode more state that other's may want to subscribe to?
    shared_state: SharedState<OT::StoreId>, // JP: Could have another thread own and manage this state
    // instead?
    phantom: PhantomData<OT>,
    device_keys: DevicePrivateKeys,
}
pub type StoreStatuses<StoreId, Hash, HeaderId, Header> =
    BTreeMap<StoreId, StoreStatus<Hash, HeaderId, Header>>; // Rename this MiniProtocolArgs?

// pub enum StoreStatus<O: OssaType, T: CRDT<Time = O::Time>>
// where
//     T::Op: Serialize,
pub enum StoreStatus<Hash, HeaderId, Header> {
    // Store is initializing and async handler is being created.
    Initializing,
    // Store's async handler is running.
    Running {
        store_handle: JoinHandle<()>, // JP: Does this belong here? The state is owned here, but
        // the miniprotocols probably don't need to block waiting on
        // it...
        // send_command_chan: UnboundedSender<StoreCommand<O::ECGHeader, T>>,
        // https://www.reddit.com/r/rust/comments/1exjiab/the_amazing_pattern_i_discovered_hashmap_with/
        // send_command_chan: UnboundedSender<StoreCommand<store::ecg::v0::Header<dyn Hash, dyn CRDT>, dyn CRDT>>,
        // send_command_chan: UnboundedSender<UntypedStoreCommand>,
        send_command_chan: UnboundedSender<UntypedStoreCommand<Hash, HeaderId, Header>>,
    },
}

#[derive(Clone, Debug)]
/// Ossa state that is shared across multiple tasks.
pub(crate) struct SharedState<StoreId> {
    pub(crate) peer_state:
        Arc<RwLock<BTreeMap<DeviceId, UnboundedSender<PeerManagerCommand<StoreId>>>>>,
}

impl<Hash, HeaderId, Header> StoreStatus<Hash, HeaderId, Header> {
    pub(crate) fn is_initializing(&self) -> bool {
        match self {
            StoreStatus::Initializing => true,
            StoreStatus::Running { .. } => false,
        }
    }

    pub(crate) fn is_initialized(&self) -> bool {
        !self.is_initializing()
    }

    pub(crate) fn command_channel(
        &self,
    ) -> Option<&UnboundedSender<UntypedStoreCommand<Hash, HeaderId, Header>>> {
        match self {
            StoreStatus::Initializing => None,
            StoreStatus::Running {
                send_command_chan, ..
            } => Some(send_command_chan),
        }
    }
}

impl<OT: OssaType> Ossa<OT> {
    async fn bind_server_ipv4(mut port: u16) -> Option<(TcpListener, SocketAddrV4)> {
        for _ in 0..10 {
            let address = SocketAddrV4::new(Ipv4Addr::new(0, 0, 0, 0), port);
            match TcpListener::bind(&address).await {
                Ok(l) => {
                    info!("Started server: {address}");
                    return Some((l, address));
                }
                Err(err) => {
                    warn!("Failed to bind to port ({}): {}", &address, err);
                    port += 1;
                }
            }
        }

        None
    }

    // Start ossa.
    pub fn start(config: OssaConfig) -> Self {
        // TODO: Load identity or take it as an argument.
        let identity_keys = DevicePrivateKeys::generate_device_keys();

        // // Create channels to communicate with Ossa thread.
        // let (send_ossa_commands, mut recv_ossa_commands) = futures_channel::mpsc::unbounded();
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
        let ossa_thread = thread::spawn(move || {
            runtime_handle.block_on(async move {
                // Start listening for connections.
                let Some((listener, addr)) = Ossa::<OT>::bind_server_ipv4(config.port).await else {
                    error!("Failed to start server.");
                    return;
                };

                // If UPNP is enabled, attempt to open port.
                if config.nat_traversal {
                    tokio::spawn(async move {
                        manage_nat(addr).await;
                    });
                }

                // // Handle commands from application.
                // tokio::spawn(async move {
                //     while let Some(cmd) = recv_ossa_commands.next().await {
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

                        info!(
                            "Handshake complete with peer: {}",
                            handshake_result.peer_id()
                        );
                        // Store peer in state.
                        if let Some(recv) =
                            initiate_peer(handshake_result.peer_id(), &shared_state).await
                        {
                            // Start miniprotocols.
                            let args = MiniProtocolArgs::new(
                                handshake_result.peer_id(),
                                active_stores,
                                recv,
                            );
                            handshake_result
                                .version()
                                .run_miniprotocols_server::<OT>(stream, args)
                                .await;
                        } else {
                            info!(
                                "Disconnecting. Already connected to peer: {}",
                                handshake_result.peer_id()
                            );
                        }
                    });
                }
            });
        });

        // TODO: Store identity key

        Ossa {
            thread: ossa_thread,
            // command_channel: send_ossa_commands,
            tokio_runtime: runtime,
            active_stores,
            phantom: PhantomData,
            shared_state: shared_state_,
            device_keys: identity_keys,
        }
    }

    pub fn create_store<S, T, ST: Storage>(&self, initial_sc_state: S, initial_ec_state: T, _storage: ST) -> StoreHandle<OT, S, T>
    where
        S: Serialize
            + Typeable
            + Clone
            + Send
            + for<'d> Deserialize<'d>
            + 'static,
        T: CRDT<Time = OT::Time>
            + Clone
            + Debug
            + Send
            + 'static
            + Typeable
            + Serialize
            + for<'d> Deserialize<'d>,
        // T::Op<CausalTime<OT::Time>>: Serialize,
        // T::Op: ConcretizeTime<T::Time>, // <OT::ECGHeader as ECGHeader>::HeaderId>,
        T::Op: ConcretizeTime<<OT::ECGHeader as ECGHeader>::HeaderId>,
        OT::ECGBody<T>: Send
            + Serialize
            + for<'d> Deserialize<'d>
            + Debug
            + ECGBody<
                T::Op,
                <T::Op as ConcretizeTime<<OT::ECGHeader as ECGHeader>::HeaderId>>::Serialized,
                Header = OT::ECGHeader,
            >,
        OT::ECGHeader: Send + Sync + Clone + 'static + Serialize + for<'d> Deserialize<'d>,
        // OT::ECGBody<T>:
        //     Send + ECGBody<T, Header = OT::ECGHeader> + Serialize + for<'d> Deserialize<'d> + Debug,
        <OT::ECGHeader as ECGHeader>::HeaderId: Send + Serialize + for<'d> Deserialize<'d>,
    {
        // Create store by generating nonce, etc.
        let store = store::State::<OT::StoreId, OT::ECGHeader, S, T, OT::Hash>::new_syncing(
            initial_sc_state.clone(),
            initial_ec_state.clone(),
        );
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
            // TODO: Pull out initial_state from store to eliminate clone.
            // This will generate a new nonce if there's a conflict.
            return self.create_store(initial_sc_state, initial_ec_state, _storage);
        }

        // Launch the store.
        let store_handle = self.launch_store(store_id, store);
        info!("Created store: {}", store_id);
        store_handle
    }

    pub fn connect_to_store<S, T>(
        &self,
        store_id: OT::StoreId,
        // storage: S,
    ) -> StoreHandle<OT, S, T>
    where
        OT::ECGHeader: Send + Sync + Clone + 'static,
        S: for<'d> Deserialize<'d> + Send + 'static,
        T::Op: ConcretizeTime<<OT::ECGHeader as ECGHeader>::HeaderId>,
        OT::ECGBody<T>: Send
            + Serialize
            + for<'d> Deserialize<'d>
            + Debug
            + ECGBody<
                T::Op,
                <T::Op as ConcretizeTime<<OT::ECGHeader as ECGHeader>::HeaderId>>::Serialized,
                Header = OT::ECGHeader,
            >,
        // T::Op: ConcretizeTime<T::Time>,
        // OT::ECGBody<T>:
        //     Send + ECGBody<T, Header = OT::ECGHeader> + Serialize + for<'d> Deserialize<'d> + Debug,
        <<OT as OssaType>::ECGHeader as ECGHeader>::HeaderId: Send,
        T: CRDT<Time = OT::Time> + Clone + Debug + Send + 'static + for<'d> Deserialize<'d>,
        // T::Op<CausalTime<OT::Time>>: Serialize,
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
        DeviceId::new(self.device_keys.auth_key().verifying_key())
    }

    // Connect to a peer over ipv4.
    pub fn connect_to_peer_ipv4(&self, address: SocketAddrV4) {
        let active_stores = self.active_stores.subscribe();
        let device_id = self.device_id();
        let shared_state = self.shared_state.clone();

        // Spawn async.
        let future_handle = self.tokio_runtime.spawn(async move {
            // Attempt to connect to peer, returning message on failure.
            info!("Connecting to peer: {address}");
            let mut stream = match TcpStream::connect(address).await {
                Ok(tcpstream) => {
                    let stream = codec::Framed::new(tcpstream, LengthDelimitedCodec::new());
                    TypedStream::new(stream)
                }
                Err(err) => {
                    warn!("Failed to connect to peer: {err}");
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

            info!(
                "Handshake complete with peer: {}",
                handshake_result.peer_id()
            );
            // Store peer in state.
            if let Some(recv) = initiate_peer(handshake_result.peer_id(), &shared_state).await {
                // Start miniprotocols.
                debug!("Start miniprotocols");
                let args = MiniProtocolArgs::new(handshake_result.peer_id(), active_stores, recv);
                handshake_result
                    .version()
                    .run_miniprotocols_client::<OT>(stream, args)
                    .await;
            } else {
                info!(
                    "Disconnecting. Already connected to peer: {}",
                    handshake_result.peer_id()
                );
            }
        });

        // Return channel with peer connection status.
    }

    // TODO: Separate state (that keeps state, syncs with other peers, etc) and optional user API (that sends state updates)?
    fn launch_store<S, T>(
        &self,
        store_id: OT::StoreId,
        store: store::State<OT::StoreId, OT::ECGHeader, S, T, OT::Hash>,
    ) -> StoreHandle<OT, S, T>
    where
        OT::ECGHeader: Send + Sync + Clone + 'static + for<'d> Deserialize<'d> + Serialize,
        T::Op: ConcretizeTime<<OT::ECGHeader as ECGHeader>::HeaderId>,
        OT::ECGBody<T>: Send
            + Serialize
            + for<'d> Deserialize<'d>
            + Debug
            + ECGBody<
                T::Op,
                <T::Op as ConcretizeTime<<OT::ECGHeader as ECGHeader>::HeaderId>>::Serialized,
                Header = OT::ECGHeader,
            >,
        // OT::ECGBody<T>:
        //     Send + ECGBody<T, Header = OT::ECGHeader> + Serialize + for<'d> Deserialize<'d> + Debug,
        <<OT as OssaType>::ECGHeader as ECGHeader>::HeaderId:
            Send + for<'d> Deserialize<'d> + Serialize,
        // T::Op<CausalTime<OT::Time>>: Serialize,
        S: for<'d> Deserialize<'d> + Send + 'static,
        T: CRDT<Time = OT::Time> + Debug + Clone + Send + 'static + for<'d> Deserialize<'d>,
    {
        // Initialize storage for this store.

        // Create channels to handle requests and send updates.
        let (send_commands, recv_commands) = tokio::sync::mpsc::unbounded_channel::<
            store::StoreCommand<OT::ECGHeader, OT::ECGBody<T>, T>,
        >();
        let (send_commands_untyped, recv_commands_untyped) = tokio::sync::mpsc::unbounded_channel::<
            store::UntypedStoreCommand<
                OT::Hash,
                <OT::ECGHeader as ECGHeader>::HeaderId,
                OT::ECGHeader,
            >,
        >();

        // Add to DHT

        // Spawn routine that owns this store.

        let shared_state = self.shared_state.clone();
        let send_commands_untyped_ = send_commands_untyped.clone();
        let future_handle = self.tokio_runtime.spawn(async move {
            store::run_handler::<OT, S, T>(
                store,
                recv_commands,
                send_commands_untyped_,
                recv_commands_untyped,
                shared_state,
            )
            .await;
        });

        // Register this store.
        self.active_stores.send_if_modified(|active_stores| {
            let _ = active_stores.insert(
                store_id,
                StoreStatus::Running {
                    store_handle: future_handle,
                    send_command_chan: send_commands_untyped,
                },
            );
            true
        });

        StoreHandle {
            // future_handle,
            send_command_chan: send_commands,
            phantom: PhantomData,
        }
    }
}

/// Thread to handle NAT traversals using UPnP IGD.
async fn manage_nat(local_addr: SocketAddrV4) {
    let config = portmapper::Config {
        // enable_upnp: false,
        protocol: portmapper::Protocol::Tcp,
        ..Default::default()
    };
    let portmapper = portmapper::Client::new(config);
    let mut watcher = portmapper.watch_external_address();

    let res = portmapper.probe().await.unwrap().unwrap();
    debug!("Available NAT traversal protocols: {res:?}");

    let port = NonZero::new(local_addr.port()).expect("0 is not a valid port");
    info!("Attempting to create external address for port: {:?}", port);
    portmapper.update_local_port(port);

    loop {
        info!("External IP address: {:?}", *watcher.borrow_and_update());
        if watcher.changed().await.is_err() {
            break;
        }
    }

    info!("External NAT IP address terminated");
}

/// Initiates a peer by creating a channel to send commands and by inserting it into the shared state. On success, returns the receiver. If the peer already exists, fails with `None`.
async fn initiate_peer<StoreId>(
    peer_id: DeviceId,
    shared_state: &SharedState<StoreId>,
) -> Option<UnboundedReceiver<PeerManagerCommand<StoreId>>> {
    let (send, recv) = tokio::sync::mpsc::unbounded_channel();
    let inserted = {
        let mut w = shared_state.peer_state.write().await;
        w.try_insert(peer_id, send).is_ok()
    };
    if inserted {
        Some(recv)
    } else {
        // JP: Record if we're already connected to the peer?
        None
    }
}

#[derive(Clone, Copy)]
pub struct OssaConfig {
    /// IPv4 port to run Ossa on.
    pub port: u16,
    /// Whether or not to attempt NAT traversal using UPnP IGD.
    pub nat_traversal: bool,
}

pub struct StoreHandle<
    O: OssaType,
    S,
    T: CRDT<Time = O::Time, Op: ConcretizeTime<<O::ECGHeader as ECGHeader>::HeaderId>>,
>
// where
//     // T::Op: Serialize,
//     T::Op<CausalTime<OT::Time>>: Serialize,
{
    // future_handle: JoinHandle<()>, // JP: Maybe this should be owned by `Ossa`?
    send_command_chan: UnboundedSender<StoreCommand<O::ECGHeader, O::ECGBody<T>, T>>,
    phantom: PhantomData<fn(O, S)>,
}

/// Trait to define newtype wrapers that instantiate type families required by Ossa.
pub trait OssaType: 'static {
    type StoreId: Debug
        + Display
        + Eq
        + Copy
        + Ord
        + Send
        + Sync
        + 'static
        + Serialize
        + for<'a> Deserialize<'a>
        + AsRef<[u8]>; // Hashable instead of AsRef???
    type Hash: util::Hash
        + Debug
        + Display
        + Copy
        + Ord
        + Send
        + Sync
        + 'static
        + Serialize
        + for<'a> Deserialize<'a>
        + Into<Self::StoreId>; // Hashable instead of AsRef???
                               // type ECGHeader<T: CRDT<Time = Self::Time, Op: Serialize>>: store::ecg::ECGHeader + Debug + Send;
    type ECGHeader: store::dag::ECGHeader<HeaderId: Send + Sync + Serialize + for<'a> Deserialize<'a>>
        + Debug
        + Send
        + Serialize
        + for<'a> Deserialize<'a>;
    type ECGBody<T: CRDT<Op: ConcretizeTime<<Self::ECGHeader as ECGHeader>::HeaderId>>>; // : Serialize + for<'a> Deserialize<'a>; // : CRDT<Time = Self::Time, Op: Serialize>;
    type SCGBody<S>: 
          Serialize
        + for<'a> Deserialize<'a>;
    type Time;
    // type CausalState<T: CRDT<Time = Self::Time, Op<CausalTime<Self::Time>>: Serialize>>: CausalState<Time = Self::Time>;
    type CausalState<T: CRDT<Time = Self::Time>>: CausalState<Time = Self::Time>;
    // type OperationId;
    // type Hash: Clone + Copy + Debug + Ord + Send;

    // TODO: This should be refactored and provided automatically.
    fn to_causal_state<T: CRDT<Time = Self::Time>>(
        st: &store::dag::State<Self::ECGHeader, T>,
    ) -> &Self::CausalState<T>;
}

impl<
        O: OssaType,
        S,
        T: CRDT<Time = O::Time, Op: ConcretizeTime<<O::ECGHeader as ECGHeader>::HeaderId>>,
    > StoreHandle<O, S, T>
// where
//     T::Op<CausalTime<T::Time>>: Serialize,
{
    pub fn apply(
        &mut self,
        parents: BTreeSet<<O::ECGHeader as ECGHeader>::HeaderId>,
        op: <T::Op as ConcretizeTime<<O::ECGHeader as ECGHeader>::HeaderId>>::Serialized,
    ) -> <O::ECGHeader as ECGHeader>::HeaderId
    where
        T::Op: ConcretizeTime<<O::ECGHeader as ECGHeader>::HeaderId>,
        O::ECGBody<T>: ECGBody<
            T::Op,
            <T::Op as ConcretizeTime<<O::ECGHeader as ECGHeader>::HeaderId>>::Serialized,
            Header = O::ECGHeader,
        >,
    {
        self.apply_batch(parents, vec![op])
    }

    // TODO: Don't take parents as an argument. Pull it from the state. XXX
    pub fn apply_batch(
        &mut self,
        parents: BTreeSet<<<O as OssaType>::ECGHeader as ECGHeader>::HeaderId>,
        op: Vec<<T::Op as ConcretizeTime<<O::ECGHeader as ECGHeader>::HeaderId>>::Serialized>, // T::Op<CausalTime<T::Time>>>,
                                                                                               // op: Vec<T::Op>,
    ) -> <O::ECGHeader as ECGHeader>::HeaderId
    where
        T::Op: ConcretizeTime<<O::ECGHeader as ECGHeader>::HeaderId>,
        <O as OssaType>::ECGBody<T>: ECGBody<
            T::Op,
            <T::Op as ConcretizeTime<<O::ECGHeader as ECGHeader>::HeaderId>>::Serialized,
            Header = O::ECGHeader,
        >,
    {
        // TODO: Divide into 256 operation chunks.
        // if op.is_empty() {
        //     return vec![];
        // }

        // Create ECG header and body.
        let body = <<O as OssaType>::ECGBody<T> as ECGBody<
            T::Op,
            <T::Op as ConcretizeTime<<O::ECGHeader as ECGHeader>::HeaderId>>::Serialized,
        >>::new_body(op);
        let header = body.new_header(parents);
        let header_id = header.get_header_id();
        // let times = body.get_operation_times(&header);

        self.send_command_chan
            .send(StoreCommand::Apply {
                operation_header: header,
                operation_body: body,
            })
            .expect("TODO");

        // times
        header_id
    }

    pub fn subscribe_to_state(&mut self) -> UnboundedReceiver<StateUpdate<O::ECGHeader, T>> {
        let (send_state, recv_state) = tokio::sync::mpsc::unbounded_channel();
        self.send_command_chan
            .send(StoreCommand::SubscribeState { send_state })
            .expect("TODO");

        recv_state
    }
}

// pub enum OssaCommand {
//     CreateStore {
//         // Since Rust doesn't have existentials...
//         initial_state: (), // Box<Dynamic>, // T
//         storage: Box<dyn Storage + Send>,
//     },
// }

// fn handle_ossa_command() {
// }
