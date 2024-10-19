use odyssey_crdt::time::CausalState;
// use futures::{SinkExt, StreamExt};
// use futures_channel::mpsc::{UnboundedReceiver, UnboundedSender};
use odyssey_crdt::CRDT;
use serde::Serialize;
use std::collections::BTreeSet;
use std::fmt::Debug;
use std::marker::PhantomData;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::thread;
use tokio::net::{TcpListener, TcpStream};
use tokio::runtime::Runtime;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::task::JoinHandle;
use tokio_util::codec::{self, LengthDelimitedCodec};

use crate::network::protocol::run_handshake_server;
use crate::protocol::Version;
use crate::storage::Storage;
use crate::store;
use crate::store::ecg::v0::{Body, Header, OperationId};
use crate::store::ecg::{self, ECGBody, ECGHeader};
use crate::util::TypedStream;

pub struct Odyssey<OT> {
    thread: thread::JoinHandle<()>,
    // command_channel: UnboundedSender<OdysseyCommand>,
    tokio_runtime: Runtime,
    phantom: PhantomData<OT>,
}

impl<OT: OdysseyType> Odyssey<OT> {
    // Start odyssey.
    pub fn start(config: OdysseyConfig) -> Self {
        // // Create channels to communicate with Odyssey thread.
        // let (send_odyssey_commands, mut recv_odyssey_commands) = futures_channel::mpsc::unbounded();

        // Start async runtime.
        let runtime = match tokio::runtime::Runtime::new() {
            Ok(r) => r,
            Err(err) => {
                log::error!("Failed to initialize tokio runtime: {}", err);
                todo!()
            }
        };
        let runtime_handle = runtime.handle().clone();

        // Spawn thread.
        let odyssey_thread = thread::spawn(move || {
            runtime_handle.block_on(async {
                // Start listening for connections.
                let address = SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 1), config.port);
                let listener = match TcpListener::bind(&address).await {
                    Ok(l) => l,
                    Err(err) => {
                        log::error!("Failed to bind to port ({}): {}", &address, err);
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

                println!("Starting server");
                loop {
                    // Accept connection.
                    let (tcpstream, peer) = match listener.accept().await {
                        Ok(r) => r,
                        Err(err) => {
                            log::error!("Failed to accept connection: {}", err);
                            continue;
                        }
                    };
                    log::info!("Accepted connection from peer: {}", peer);
                    // Spawn async.
                    tokio::spawn(async {
                        // let (read_stream, write_stream) = tcpstream.split();
                        let stream = codec::Framed::new(tcpstream, LengthDelimitedCodec::new());

                        // TODO XXX
                        // Handshake.
                        // Diffie Hellman?
                        // Authenticate peer's public key?
                        let stream = TypedStream::new(stream);
                        let Version::V0 = run_handshake_server(&stream);
                    });
                }
            });
        });

        Odyssey {
            thread: odyssey_thread,
            // command_channel: send_odyssey_commands,
            tokio_runtime: runtime,
            phantom: PhantomData,
        }
    }

    pub fn create_store<T: CRDT<Time = OT::Time> + Clone + Send + 'static, S: Storage>(
        &self,
        initial_state: T,
        storage: S,
    ) -> StoreHandle<OT, T>
    where
        // T::Op: Send,
        <OT as OdysseyType>::ECGHeader<T>: Send + Clone + 'static,
        <<OT as OdysseyType>::ECGHeader<T> as ECGHeader<T>>::Body: Send,
        <<OT as OdysseyType>::ECGHeader<T> as ECGHeader<T>>::Body: ECGBody<T>,
        <<OT as OdysseyType>::ECGHeader<T> as ECGHeader<T>>::HeaderId: Send,
        // T: CRDT<Op = <<<OT as OdysseyType>::ECGHeader<T> as ECGHeader>::Body as ECGBody>::Operation>,
        // T: CRDT<Time = <<OT as OdysseyType>::ECGHeader<T> as ECGHeader>::OperationId>,
        // T: CRDT<Time = OperationId<Header<OT::Hash, T>>>,
        // OperationId<Header::HeaderId>
        // <OT as OdysseyType>::Hash: 'static,
        T::Op: Serialize,
        // OT::Time: CausalOrder<State = ecg::State<OT::ECGHeader<T>, T>>,
    {
        // TODO:
        // Check if this store already exists and return that.

        // Create store by generating nonce, etc.
        let mut store = store::State::<OT::ECGHeader<T>, T>::new(initial_state.clone());

        // Initialize storage for this store.

        // Create channels to handle requests and send updates.
        let (send_commands, mut recv_commands) =
            tokio::sync::mpsc::unbounded_channel::<StoreCommand<OT::ECGHeader<T>, T>>();

        // Add to DHT

        // Spawn routine that owns this store.
        let future_handle = self.tokio_runtime.spawn(async move {
            let mut state = initial_state;
            let mut listeners: Vec<UnboundedSender<StateUpdate<OT::ECGHeader<T>, T>>> = vec![];

            println!("Creating store");
            // TODO: Create ECGState, ...
            while let Some(cmd) = recv_commands.recv().await {
                match cmd {
                    StoreCommand::Apply {
                        operation_header,
                        operation_body,
                    } => {
                        // Update ECG state.
                        let success = store.ecg_state.insert_header(operation_header.clone());
                        if !success {
                            todo!("Invalid header"); // : {:?}", operation_header);
                        }

                        // Update state.
                        // TODO: Get new time?.. Or take it as an argument
                        // Operation ID/time is function of tips, current operation, ...? How do we
                        // do batching? (HeaderId(h) | Self, Index(u8)) ? This requires having all
                        // the batched operations?
                        let causal_state = OT::to_causal_state(&store.ecg_state);
                        for (time, operation) in
                            operation_header.zip_operations_with_time(operation_body)
                        {
                            state = state.apply(causal_state, time, operation);
                        }

                        // Send state to subscribers.
                        for mut l in &listeners {
                            let snapshot = StateUpdate::Snapshot {
                                snapshot: state.clone(),
                                ecg_state: store.ecg_state.clone(),
                            };
                            l.send(snapshot);
                        }
                    }
                    StoreCommand::SubscribeState { mut send_state } => {
                        // Send current state.
                        let snapshot = StateUpdate::Snapshot {
                            snapshot: state.clone(),
                            ecg_state: store.ecg_state.clone(),
                        };
                        send_state.send(snapshot);

                        // Register this subscriber.
                        listeners.push(send_state);
                    }
                }
            }
        });

        // Register this store.

        StoreHandle {
            future_handle,
            send_command_chan: send_commands,
            phantom: PhantomData,
        }
    }

    pub fn load_store<T: CRDT<Time = OT::Time>, S: Storage>(
        &self,
        store_id: OT::StoreId,
        storage: S,
    ) -> StoreHandle<OT, T>
    where
        T::Op: Serialize,
    {
        todo!()
    }

    // Connect to network.
    pub fn connect() {
        todo!("Turn on network connection")
    }

    // Disconnect from network.
    pub fn disconnect() {
        todo!("Turn off network connection (work offline)")
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
    future_handle: JoinHandle<()>, // JP: Maybe this should be owned by `Odyssey`?
    send_command_chan: UnboundedSender<StoreCommand<O::ECGHeader<T>, T>>,
    phantom: PhantomData<O>,
}

/// Trait to define newtype wrapers that instantiate type families required by Odyssey.
pub trait OdysseyType {
    type StoreId; // <T>
    type ECGHeader<T: CRDT<Time = Self::Time, Op: Serialize>>: store::ecg::ECGHeader<T>;
    type Time;
    type CausalState<T: CRDT<Time = Self::Time, Op: Serialize>>: CausalState<Time = Self::Time>;
    // type OperationId;
    // type Hash: Clone + Copy + Debug + Ord + Send;

    // TODO: This should be refactored and provided automatically.
    fn to_causal_state<'a, T: CRDT<Time = Self::Time, Op: Serialize>>(
        st: &'a store::ecg::State<Self::ECGHeader<T>, T>,
    ) -> &'a Self::CausalState<T>;
}

enum StoreCommand<Header: ECGHeader<T>, T: CRDT> {
    Apply {
        operation_header: Header,     // <Hash, T>,
        operation_body: Header::Body, // <Hash, T>,
    },
    SubscribeState {
        send_state: UnboundedSender<StateUpdate<Header, T>>,
    },
}

pub enum StateUpdate<Header: ECGHeader<T>, T: CRDT> {
    Snapshot {
        snapshot: T,
        ecg_state: ecg::State<Header, T>,
        // TODO: ECG DAG
    },
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
        if op.len() == 0 {
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
        });

        times
    }

    pub fn subscribe_to_state(&mut self) -> UnboundedReceiver<StateUpdate<O::ECGHeader<T>, T>> {
        let (send_state, mut recv_state) = tokio::sync::mpsc::unbounded_channel();
        self.send_command_chan
            .send(StoreCommand::SubscribeState { send_state });

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
