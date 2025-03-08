use async_recursion::async_recursion;
use bytes::{Buf, Bytes, BytesMut};
use futures;
use futures::task::{Context, Poll};
use serde::{Deserialize, Serialize};
use std::cmp::min;
use std::collections::{BTreeMap, BTreeSet};
use std::marker::PhantomData;
use std::pin::Pin;
use tokio::{
    io::{simplex, AsyncReadExt, AsyncWriteExt, SimplexStream, WriteHalf},
    net::TcpStream,
    sync::{
        mpsc::{self, Receiver, Sender},
        watch,
    },
    task::JoinHandle,
};
use tokio_stream::wrappers::ReceiverStream;
use tokio_util::sync::{PollSendError, PollSender};

use crate::core::{OdysseyType, StoreStatuses};
use crate::{
    network::protocol::{MiniProtocol, ProtocolError},
    protocol::v0::MiniProtocols,
    util::{self, TypedStream},
};

const OUTGOING_CAPACITY: usize = 32;
const PROTOCOL_INCOMING_CAPACITY: usize = 4;
const BUFFER_SIZE: usize = 4096;

#[derive(Clone, Copy, Debug)]
pub(crate) enum Party {
    Client,
    Server,
}

impl Party {
    pub(crate) fn is_client(&self) -> bool {
        match self {
            Party::Client => true,
            Party::Server => false,
        }
    }

    pub(crate) fn is_server(&self) -> bool {
        match self {
            Party::Client => false,
            Party::Server => true,
        }
    }
}

pub(crate) struct Multiplexer {
    party: Party,
}

impl Multiplexer {
    pub(crate) fn new(party: Party) -> Multiplexer {
        Multiplexer { party }
    }

    /// Run the multiplexer with these initial mini protocols.
    /// The minitprotocols are assigned identifiers in order, starting at 0.
    pub(crate) async fn run_with_miniprotocols<O: OdysseyType>(
        self,
        mut stream: TcpStream,
        miniprotocols: Vec<MiniProtocols>,
        active_stores: watch::Receiver<StoreStatuses<O::StoreId>>,
    ) {
        println!("run_with_miniprotocols: {:?}", self.party);

        // Create multiplexer state.
        let mut state: MultiplexerState = MultiplexerState::new();

        let (outgoing_channel_send, mut outgoing_channel) = mpsc::channel(OUTGOING_CAPACITY);

        // Initialize and spawn each miniprotocol.
        for (protocol_id, p) in miniprotocols.into_iter().enumerate() {
            let protocol_id = protocol_id.try_into().expect("TODO");
            let outgoing_channel_send = outgoing_channel_send.clone();

            // Create window for the miniprotocol.
            let (sender, receiver) = mpsc::channel(PROTOCOL_INCOMING_CAPACITY);

            // Spawn async for the miniprotocol.
            let active_stores = active_stores.clone();
            let handle = tokio::spawn(p.spawn_async::<O>(self.party.is_client(), protocol_id, outgoing_channel_send, receiver, active_stores));

            let mp = MiniprotocolState { handle, sender };
            state.stream_map.insert(protocol_id, mp);
        }

        // // Create window (buffered channel?) for each miniprotocol.
        // let (mut heartbeat_client_channel, heartbeat_protocol_channel) = util::Channel::new_pair(10);

        // TODO: back pressure

        // TODO: Do some load balancing between miniprotocols?
        // TODO: Pipelining
        // JP: Should we have separate threads for sending and receiving? Makes managing `state` annoying.

        loop {
            let mut buf = BytesMut::with_capacity(BUFFER_SIZE);

            // Wait on data from client or data to send.
            tokio::select! {
                msg_e = outgoing_channel.recv() => {
                    match msg_e {
                        None => {
                            todo!()
                        }
                        // Some(Err(_e)) => {
                        //     todo!()
                        // }
                        Some((stream_id, mut msg)) => {
                            // We rely on the miniprotocol wrapper to send prepend the stream id, length, etc.
                            // Write stream id and message length.
                            // TODO: prepend this to the buffer so we don't make two system calls.
                            stream.write_u32(stream_id).await.expect("TODO");

                            println!("Sending on stream: {}", stream_id);

                            let length = msg.len().try_into().expect("TODO");
                            stream.write_u32(length).await.expect("TODO");
                            println!("Sending length: {}", length);

                            // Write message.
                            stream.write_all_buf(&mut msg).await.expect("TODO");
                        }
                    }
                }
                result = stream.read_buf(&mut buf) => {
                    match result {
                        Err(err) => {
                            todo!("Error: {:?}", err);
                        }
                        Ok(length) => {
                            assert_eq!(length, buf.len());
                            state.read_state = state.read_state.handle_receive(&state.stream_map, buf).await;
                            println!("Test out: {:?}", state.read_state);
                        }
                    }
                }
            }
        }
    }
}

pub(crate) type StreamId = u32;
struct MultiplexerState {
    stream_map: BTreeMap<StreamId, MiniprotocolState>,
    read_state: MultiplexerReadState,
}

impl MultiplexerState {
    fn new() -> MultiplexerState {
        MultiplexerState {
            stream_map: BTreeMap::new(),
            read_state: MultiplexerReadState::new(),
        }
    }
}

const MAX_MESSAGE_LENGTH: u32 = 1000 * 1000 * 1024;
const HEADER_LENGTH: usize = 8;
#[derive(Debug)]
enum MultiplexerReadState {
    ProcessingHeader {
        position: usize,
        header: [u8; HEADER_LENGTH],
    },
    ProcessingBody {
        msg_length: u32,
        sender: Sender<BytesMut>,
        send_buffer: BytesMut,
        // TODO: miniprotocol channel?, Length, ...
    },
}

impl MultiplexerReadState {
    fn new() -> MultiplexerReadState {
        MultiplexerReadState::ProcessingHeader {
            position: 0,
            header: [0; HEADER_LENGTH],
        }
    }

    #[async_recursion]
    async fn handle_receive(
        self,
        stream_map: &BTreeMap<StreamId, MiniprotocolState>,
        mut buf: BytesMut,
    ) -> MultiplexerReadState {
        println!("Test in:  {:?}", self);
        println!("{:?}", buf);
        match self {
            MultiplexerReadState::ProcessingHeader {
                mut position,
                mut header,
            } => {
                // Read header.
                let low_i = position;
                let remaining_c = HEADER_LENGTH - position;
                let received_c = min(buf.len(), remaining_c);

                let high_i = position + received_c;

                let mut header_buf = buf.split_to(received_c);
                header_buf.copy_to_slice(&mut header[low_i..high_i]);

                position += received_c;

                // Check if we've received the entire header.
                if position == HEADER_LENGTH {
                    // Parse stream id.
                    let stream_id = header[0..4].try_into().unwrap();
                    let stream_id = u32::from_be_bytes(stream_id);
                    println!("Received stream id: {stream_id:?}");

                    let p = stream_map.get(&stream_id).expect("TODO");
                    let sender = p.sender.clone();

                    // Parse message length.
                    let msg_length = header[4..8].try_into().unwrap();
                    let msg_length = u32::from_be_bytes(msg_length);
                    println!("Received msg_length: {msg_length:?}");

                    // Check upper bound on message length.
                    if msg_length > MAX_MESSAGE_LENGTH {
                        todo!("TODO: Other party sent too large of a message.");
                    }

                    // Allocate buffer.
                    let send_buffer = BytesMut::with_capacity(msg_length as usize);

                    let next_state = MultiplexerReadState::ProcessingBody {
                        msg_length,
                        sender,
                        send_buffer,
                    };

                    return next_state.handle_receive(stream_map, buf).await;
                } else {
                    MultiplexerReadState::ProcessingHeader { position, header }
                }
            }
            MultiplexerReadState::ProcessingBody {
                msg_length,
                sender,
                mut send_buffer,
            } => {
                // Append received bytes to the buffer.
                let remaining_c = msg_length as usize - send_buffer.len();
                let received_c = min(buf.len(), remaining_c);
                let received_buf = buf.split_to(received_c);
                send_buffer.extend(received_buf);

                // Send message if we've received the entire message.
                if send_buffer.len() == msg_length as usize {
                    sender.send(send_buffer).await;

                    let next_state = MultiplexerReadState::new();

                    // Keep working if there are still bytes left in the buffer.
                    if !buf.is_empty() {
                        println!("Buffer isn't empty!");
                        return next_state.handle_receive(stream_map, buf).await;
                    } else {
                        next_state
                    }
                } else {
                    MultiplexerReadState::ProcessingBody {
                        msg_length,
                        sender,
                        send_buffer,
                    }
                }
            }
        }
    }
}

// Multiplexer's state for a given miniprotocol.
struct MiniprotocolState {
    handle: JoinHandle<()>,
    sender: mpsc::Sender<BytesMut>,
}

pub(crate) async fn spawn_miniprotocol_async<P: MiniProtocol, O: OdysseyType>(
    p: P,
    is_client: bool,
    stream_id: StreamId,
    sender: Sender<(StreamId, Bytes)>,
    receiver: Receiver<BytesMut>,
    active_stores: watch::Receiver<StoreStatuses<O::StoreId>>,
) {
    // Serialize/deserialize byte channel
    let stream = MuxStream::new(stream_id, sender, receiver);
    println!("Here 1: {}", std::any::type_name_of_val(&p));

    if is_client {
        println!("Run client");
        p.run_client::<_, O>(stream, active_stores).await
    } else {
        println!("Run server");
        p.run_server::<_, O>(stream, active_stores).await
    }
}

// struct FramedMiniprotocol<T> {
//     phantom: PhantomData<T>,
// }

// Stream implementation to send bytes between multiplexer and miniprotocols.
struct MuxStream<T> {
    stream_id: StreamId,
    sender: PollSender<(StreamId, Bytes)>,
    receiver: ReceiverStream<BytesMut>,
    phantom: PhantomData<fn(T)>,
}

impl<T> MuxStream<T> {
    fn new(
        stream_id: StreamId,
        sender: Sender<(StreamId, Bytes)>,
        receiver: Receiver<BytesMut>,
    ) -> MuxStream<T> {
        let sender = PollSender::new(sender);
        let receiver = ReceiverStream::new(receiver);
        MuxStream {
            stream_id,
            sender,
            receiver,
            phantom: PhantomData,
        }
    }
}

impl<T> futures::Stream for MuxStream<T>
where
    T: for<'a> Deserialize<'a>,
{
    type Item = Result<T, ProtocolError>;

    fn poll_next(
        mut self: Pin<&mut Self>,
        ctx: &mut Context<'_>,
    ) -> Poll<Option<Result<T, ProtocolError>>> {
        let p = futures::Stream::poll_next(Pin::new(&mut (self.receiver)), ctx);
        p.map(|o| {
            o.map(|bytes| {
                serde_cbor::from_slice(&bytes).map_err(|err| {
                    // log::error!("Failed to parse type {}: {}", type_name::<T>(), err);
                    ProtocolError::DeserializationError(err)
                })
            })
        })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.receiver.size_hint()
    }
}

impl<T> futures::Sink<T> for MuxStream<T>
where
    T: Serialize,
{
    type Error = ProtocolError; // PollSendError<Bytes>;

    fn poll_ready(
        mut self: Pin<&mut Self>,
        ctx: &mut Context<'_>,
    ) -> Poll<Result<(), <Self as futures::Sink<T>>::Error>> {
        let p = Pin::new(&mut self.sender).poll_ready(ctx);
        p.map(|r| {
            r.map_err(|e| {
                log::error!("Send error: {:?}", e);
                ProtocolError::ChannelSendError(e)
            })
        })
    }

    fn start_send(mut self: Pin<&mut Self>, x: T) -> Result<(), <Self as futures::Sink<T>>::Error> {
        match serde_cbor::to_vec(&x) {
            Err(err) => Err(ProtocolError::SerializationError(err)),
            Ok(cbor) => {
                let stream_id = self.stream_id;
                let p = Pin::new(&mut self.sender).start_send((stream_id, cbor.into()));
                p.map_err(|e| {
                    log::error!("Send error: {:?}", e);
                    ProtocolError::ChannelSendError(e)
                })
            }
        }
    }

    fn poll_flush(
        mut self: Pin<&mut Self>,
        ctx: &mut Context<'_>,
    ) -> Poll<Result<(), <Self as futures::Sink<T>>::Error>> {
        let p = Pin::new(&mut self.sender).poll_flush(ctx);
        p.map(|r| {
            r.map_err(|e| {
                log::error!("Send error: {:?}", e);
                ProtocolError::ChannelSendError(e)
            })
        })
    }

    fn poll_close(
        mut self: Pin<&mut Self>,
        ctx: &mut Context<'_>,
    ) -> Poll<Result<(), <Self as futures::Sink<T>>::Error>> {
        let p = Pin::new(&mut self.sender).poll_close(ctx);
        p.map(|r| {
            r.map_err(|e| {
                log::error!("Send error: {:?}", e);
                ProtocolError::ChannelSendError(e)
            })
        })
    }
}

impl<T> util::Stream<T> for MuxStream<T> where T: for<'a> Deserialize<'a> + Serialize {}
