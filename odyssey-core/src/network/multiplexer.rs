
use bytes::{Bytes, BytesMut};
use futures;
use futures::task::{Context, Poll};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::marker::PhantomData;
use std::pin::Pin;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, SimplexStream, WriteHalf, simplex},
    net::TcpStream,
    sync::mpsc::{self, Receiver, Sender},
    task::JoinHandle,
};
use tokio_stream::wrappers::ReceiverStream;
use tokio_util::sync::{PollSender, PollSendError};

use crate::{
    network::protocol::{
        MiniProtocol,
        ProtocolError,
    },
    util::{self, TypedStream},
};

const OUTGOING_CAPACITY: usize = 32;
const PROTOCOL_INCOMING_CAPACITY: usize = 4;
const BUFFER_SIZE: usize = 4096;

#[derive(Clone, Copy)]
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
    party: Party
}

impl Multiplexer {
    pub(crate) fn new(party: Party) -> Multiplexer {
        Multiplexer {
            party,
        }
    }

    /// Run the multiplexer with these initial mini protocols.
    /// The minitprotocols are assigned identifiers in order, starting at 0.
    pub(crate) async fn run_with_miniprotocols(self, mut stream: TcpStream, miniprotocols: Vec<impl MiniProtocol + 'static>) {
        // Create multiplexer state.
        let mut state: MultiplexerState = BTreeMap::new();

        let (outgoing_channel_send, mut outgoing_channel) = mpsc::channel::<Bytes>(OUTGOING_CAPACITY);

        // Initialize and spawn each miniprotocol.
        for (protocol_id, p) in miniprotocols.into_iter().enumerate() {
            let protocol_id = protocol_id.try_into().expect("TODO");
            let outgoing_channel_send = outgoing_channel_send.clone();

            // Create window for the miniprotocol.
            let (sender, receiver) = mpsc::channel(PROTOCOL_INCOMING_CAPACITY);

            // Spawn async for the miniprotocol.
            let handle = tokio::spawn(async move {

                // TODO: Convert the channel to a T: Stream<MsgHeartbeat>
                // Serialize/deserialize byte channel
                // let stream: crate::util::Channel<_> = todo!();
                // let stream: crate::util::Channel<BytesMut> = todo!();
                // let stream: crate::util::Channel<Result<BytesMut, std::io::Error>> = todo!();
                let stream = MuxStream::new(outgoing_channel_send, receiver);
                // todo!()
                // let stream = TypedStream::new(stream);
                if self.party.is_client() {
                    p.run_client(stream);
                } else {
                    p.run_server(stream);
                }
            });

            let mp = MiniprotocolState {
                handle,
                sender,
            };
            state.insert(protocol_id, mp);
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
                        Some(mut msg) => {
                            // We rely on the miniprotocol wrapper to send prepend the stream id, length, etc.
                            stream.write_all_buf(&mut msg).await;
                        }
                    }
                }
                result = stream.read_buf(&mut buf) => {
                    match result {
                        Err(_e) => {
                            todo!();
                        }
                        Ok(length) => {
                            // Check length.
                            if length < 8 {
                                todo!("Insufficient bytes");
                            }
                            // TODO: Check upper bound on length.
                            assert_eq!(length, buf.len());

                            // Read stream id.
                            let stream_id = (*buf.split_to(4)).try_into().expect("TODO");
                            let stream_id = u32::from_be_bytes(stream_id);

                            // Check message length.
                            let msg_length = (*buf.split_to(4)).try_into().expect("TODO");
                            let msg_length = u32::from_be_bytes(msg_length);
                            // TODO: Check upper bound on msg_length.

                            let p = state.get_mut(&stream_id).expect("TODO");

                            // TODO: Allocate buffer.


                            // TODO: This currently blocks if the channel is full
                            // p.sender.write_all_buf(&mut buf).await.expect("TODO");
                            p.sender.send(buf).await.expect("TODO");

                            if msg_length > BUFFER_SIZE as u32 - 8 {
                                todo!("Handle larger messages")
                            }
                        }
                    }
                    

                }
            }
        }
    }
}

type StreamId = u32;
type MultiplexerState = BTreeMap<StreamId, MiniprotocolState>;

// Multiplexer's state for a given miniprotocol.
struct MiniprotocolState {
    handle: JoinHandle<()>,
    sender: mpsc::Sender<BytesMut>,
}

// struct FramedMiniprotocol<T> {
//     phantom: PhantomData<T>,
// }

// Stream implementation to send bytes between multiplexer and miniprotocols.
struct MuxStream<T> {
    sender: PollSender<Bytes>,
    receiver: ReceiverStream<BytesMut>,
    phantom: PhantomData<fn(T)>,
}

impl<T> MuxStream<T> {
    fn new(sender: Sender<Bytes>, receiver: Receiver<BytesMut>) -> MuxStream<T> {
        let sender = PollSender::new(sender);
        let receiver = ReceiverStream::new(receiver);
        MuxStream {
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
            o.map(|bytes|
                serde_cbor::from_slice(&bytes).map_err(|err| {
                    // log::error!("Failed to parse type {}: {}", type_name::<T>(), err);
                    ProtocolError::DeserializationError(err)
                })
            )
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
                let p = Pin::new(&mut self.sender).start_send(cbor.into());
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

impl<T> util::Stream<T> for MuxStream<T>
where
    T: for<'a> Deserialize<'a> + Serialize,
{}
