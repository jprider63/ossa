
use dynamic::Dynamic;
use futures::StreamExt;
use futures_channel::mpsc::UnboundedSender;
use std::marker::PhantomData;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::thread;
use tokio::net::{TcpListener, TcpStream};
use tokio::runtime::Runtime;
use tokio::task::JoinHandle;
use tokio_util::codec::{self, LengthDelimitedCodec};

use crate::network::protocol::run_handshake_server;
use crate::protocol::Version;
use crate::storage::Storage;
use crate::store;
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
                log::error!(
                    "Failed to initialize tokio runtime: {}",
                    err
                );
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

    pub fn create_store<O: OdysseyType, T, S: Storage>(&self, initial_state: T, storage: S) -> StoreHandle<O, T> {
        // TODO:
        // Create store by generating nonce, etc.
        let store = store::State::<O::ECGHeader, T>::new(initial_state);
        todo!();

        // Initialize storage for this store.

        // Create channels to handle requests and send updates.
        let (send_commands, mut recv_commands) = futures_channel::mpsc::unbounded();

        // Add to DHT
        let future_handle = self.tokio_runtime.spawn(async {
            println!("Creating store");
            while let Some(cmd) = recv_commands.next().await {
                match cmd {
                    StoreCommand::Apply{..} => {
                    }
                }
            }
        });

        StoreHandle {
            future_handle,
            phantom: PhantomData,
        }
    }

    pub fn load_store<T, S: Storage>(&self, store_id: OT::StoreId, storage: S) -> StoreHandle<OT, T> {
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

pub struct OdysseyConfig {
    // IPv4 port to run Odyssey on.
    pub port: u16,
}

pub struct StoreHandle<O: OdysseyType, T> {
    future_handle: JoinHandle<()>,
    phantom: PhantomData<(O, T)>,
}

/// Trait to define newtype wrapers that instantiate type families required by Odyssey.
pub trait OdysseyType {
    type StoreId;
    type ECGHeader: store::ecg::ECGHeader;
}

enum StoreCommand {
    Apply {
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
