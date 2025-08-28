use async_recursion::async_recursion;
use bytes::{Buf, Bytes, BytesMut};
use futures;
use futures::task::{Context, Poll};
use serde::{Deserialize, Serialize};
use std::cmp::min;
use std::collections::BTreeMap;
use std::marker::PhantomData;
use std::pin::Pin;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::sync::oneshot;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    sync::mpsc::{self, Receiver, Sender},
    task::JoinHandle,
};
use tokio_stream::wrappers::ReceiverStream;
use tokio_util::sync::PollSender;
use tracing::{debug, error, trace, warn};

use crate::core::OssaType;
use crate::store::ecg::ECGHeader;
use crate::{
    network::protocol::{MiniProtocol, ProtocolError},
    protocol::v0::MiniProtocols,
    util,
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
    pub(crate) fn dual(&self) -> Party {
        match self {
            Party::Client => Party::Server,
            Party::Server => Party::Client,
        }
    }
}

pub(crate) struct Multiplexer {
    party: Party,
    command_recv: UnboundedReceiver<MultiplexerCommand>,
}

impl Multiplexer {
    pub(crate) fn new(
        party: Party,
        command_recv: UnboundedReceiver<MultiplexerCommand>,
    ) -> Multiplexer {
        Multiplexer {
            party,
            command_recv,
        }
    }

    /// Run the multiplexer with these initial mini protocols.
    /// The miniprotocols are assigned identifiers in order, starting at 0.
    pub(crate) async fn run_with_miniprotocols<O: OssaType>(
        mut self,
        mut stream: TcpStream,
        miniprotocols: Vec<
            MiniProtocols<O::StoreId, O::Hash, <O::ECGHeader as ECGHeader>::HeaderId, O::ECGHeader>,
        >,
    ) {
        debug!("run_with_miniprotocols: {:?}", self.party);

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
            let handle = tokio::spawn(p.run_async(
                self.party.is_client(),
                protocol_id,
                outgoing_channel_send,
                receiver,
            ));

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

                            trace!("Sending on stream: {}", stream_id);

                            let length = msg.len().try_into().expect("TODO");
                            stream.write_u32(length).await.expect("TODO");
                            trace!("Sending length: {}", length);

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
                            trace!("Test out: {:?}", state.read_state);
                        }
                    }
                }
                cmd_m = self.command_recv.recv() => {
                    let Some(cmd) = cmd_m else {
                        warn!("Multiplexer command channel dropped");
                        todo!();
                    };

                    match cmd {
                        MultiplexerCommand::CreateStream { stream_id, spawn_task, response_chan } => {
                            let outgoing_channel_send = outgoing_channel_send.clone();

                            // Create window for the miniprotocol.
                            let (sender, receiver) = mpsc::channel(PROTOCOL_INCOMING_CAPACITY);


                            // Spawn stream for new miniprotocol.
                            let handle = spawn_task(self.party.is_client(), stream_id, outgoing_channel_send, receiver);

                            let mp = MiniprotocolState { handle, sender };
                            let res = state.stream_map.try_insert(stream_id, mp);

                            // Send response.
                            response_chan.send(res.is_ok()).expect("TODO");
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
        trace!("Test in:  {:?}", self);
        trace!("{:?}", buf);
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
                    trace!("Received stream id: {stream_id:?}");

                    let p = stream_map.get(&stream_id).expect("TODO");
                    let sender = p.sender.clone();

                    // Parse message length.
                    let msg_length = header[4..8].try_into().unwrap();
                    let msg_length = u32::from_be_bytes(msg_length);
                    trace!("Received msg_length: {msg_length:?}");

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
                    sender.send(send_buffer).await.expect("TODO");

                    let next_state = MultiplexerReadState::new();

                    // Keep working if there are still bytes left in the buffer.
                    if !buf.is_empty() {
                        debug!("Buffer isn't empty!");
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

// JP: TODO: This O probably isn't needed.
pub(crate) async fn run_miniprotocol_async<P: MiniProtocol>(
    p: P,
    is_client: bool,
    stream_id: StreamId,
    sender: Sender<(StreamId, Bytes)>,
    receiver: Receiver<BytesMut>,
) {
    // Serialize/deserialize byte channel
    let stream = MuxStream::new(stream_id, sender, receiver);
    debug!(
        "Launching miniprotocol: {} ({}, {stream_id:?})",
        if is_client { "Client" } else { "Server" },
        std::any::type_name_of_val(&p)
    );

    if is_client {
        p.run_client(stream).await
    } else {
        p.run_server(stream).await
    }
}

pub(crate) type SpawnMultiplexerTask =
    dyn FnOnce(bool, StreamId, Sender<(u32, Bytes)>, Receiver<BytesMut>) -> JoinHandle<()> + Send;
pub(crate) enum MultiplexerCommand {
    CreateStream {
        stream_id: StreamId,
        spawn_task: Box<SpawnMultiplexerTask>, // impl Future>,
        // miniprotocol: Box<dyn MiniProtocol<Message = dyn Any>>,
        // handle: JoinHandle<()>,
        // sender: mpsc::Sender<BytesMut>,
        response_chan: oneshot::Sender<bool>,
    },
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
                    // error!("Failed to parse type {}: {}", type_name::<T>(), err);
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
                error!("Send error: {:?}", e);
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
                    error!("Send error: {:?}", e);
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
                error!("Send error: {:?}", e);
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
                error!("Send error: {:?}", e);
                ProtocolError::ChannelSendError(e)
            })
        })
    }
}

impl<T> util::Stream<T> for MuxStream<T> where T: for<'a> Deserialize<'a> + Serialize {}
