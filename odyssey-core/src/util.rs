use bytes::{Bytes, BytesMut};
use futures;
use futures::task::{Context, Poll};
use rand::{rngs::OsRng, TryRngCore};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::any::type_name;
use std::fmt::{self, Debug, Display};
use std::marker::PhantomData;
use std::pin::Pin;
use tokio::sync::mpsc::{Receiver, Sender};
use typeable::Typeable;

use crate::network::protocol::ProtocolError;
use crate::store::Nonce;

/// Generate a random nonce.
pub(crate) fn generate_nonce() -> Nonce {
    let mut nonce = [0; 32];
    OsRng.try_fill_bytes(&mut nonce).expect("Nonce generation failed");
    nonce
}

#[test]
fn test() {
    let n1 = generate_nonce();
    let n2 = generate_nonce();
    // println!("Nonce: {:?}", n1);
    // println!("Nonce: {:?}", n2);
    assert!(n1 != n2);
}

// TODO: Remove this trait
pub trait Hash: PartialEq + AsRef<[u8]> {
    type HashState;

    fn new() -> Self::HashState;
    fn update(state: &mut Self::HashState, data: impl AsRef<[u8]>);
    fn finalize(state: Self::HashState) -> Self;
}

#[derive(Clone, Copy, Deserialize, Eq, Ord, PartialOrd, Serialize, Typeable)]
pub struct Sha256Hash(pub [u8; 32]);

impl Debug for Sha256Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(f, "0x")?;
        for b in self.0 {
            write!(f, "{:02X}", b)?;
        }
        Ok(())
    }
}

impl Display for Sha256Hash {
    // bs58
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> Result<(), std::fmt::Error> {
        let base58 = bs58::encode(self.0).into_string();
        write!(f, "{base58}")?;
        Ok(())
    }
}

impl AsRef<[u8]> for Sha256Hash {
    fn as_ref(&self) -> &[u8] {
        self.0.as_ref()
    }
}

impl PartialEq for Sha256Hash {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl Hash for Sha256Hash {
    type HashState = Sha256;

    fn new() -> Self::HashState {
        Sha256::new()
    }

    fn update(state: &mut Self::HashState, data: impl AsRef<[u8]>) {
        state.update(data)
    }

    fn finalize(state: Self::HashState) -> Self {
        Sha256Hash(state.finalize().into())
    }
}

#[derive(Debug)]
pub enum Sha256HashParseError {
    Base58(bs58::decode::Error),
    Hex(hex::FromHexError),
}
impl std::str::FromStr for Sha256Hash {
    type Err = Sha256HashParseError;
    fn from_str(s: &str) -> Result<Self, Sha256HashParseError> {
        // "0xA01BE6E4A62BA7D0988FD6F1FE5DC964FD818628A96B472AA3472E6EFFB9A74F"
        let mut store_id = [0; 32];
        if s.len() == 66 {
            // Parse as Hex (with 0x).
            hex::decode_to_slice(&s[2..], &mut store_id).map_err(Sha256HashParseError::Hex)?;
        } else if s.len() == 64 {
            // Parse as Hex.
            hex::decode_to_slice(s, &mut store_id).map_err(Sha256HashParseError::Hex)?;
        } else {
            bs58::decode(s).onto(&mut store_id).map_err(Sha256HashParseError::Base58)?;
        }
        Ok(Sha256Hash(store_id))
    }
}

// TODO: Generalize the error and stream.
pub trait Stream<T>:
      futures::Stream<Item=Result<T,ProtocolError>> // Result<BytesMut,std::io::Error>>
    + futures::Sink<T, Error=ProtocolError>
    + Unpin
    + Send // JP: This is needed for async_recursion. Not sure if this makes sense in practice.
    + Sync // JP: This is needed for async_recursion. Not sure if this makes sense in practice.
{}

// TODO: Move this somewhere else, or remove this since we have MuxStream now?
pub struct TypedStream<S, T> {
    stream: S,
    phantom: PhantomData<fn(T)>,
}

impl<S, T> TypedStream<S, T> {
    pub fn new(stream: S) -> TypedStream<S, T> {
        TypedStream {
            stream,
            phantom: PhantomData,
        }
    }

    pub fn finalize(self) -> S {
        self.stream
    }
}

impl<S, T> futures::Stream for TypedStream<S, T>
where
    S: futures::Stream<Item = Result<BytesMut, std::io::Error>> + Unpin,
    T: for<'a> Deserialize<'a>,
{
    type Item = Result<T, ProtocolError>;
    fn poll_next(
        mut self: Pin<&mut Self>,
        ctx: &mut Context<'_>,
    ) -> Poll<std::option::Option<<Self as futures::Stream>::Item>> {
        let p = futures::Stream::poll_next(Pin::new(&mut self.stream), ctx);
        p.map(|o| {
            o.map(|t| match t {
                Ok(bytes) => serde_cbor::from_slice(&bytes).map_err(|err| {
                    // log::error!("Failed to parse type {}: {}", type_name::<T>(), err);
                    ProtocolError::DeserializationError(err)
                }),
                Err(err) => Err(ProtocolError::StreamReceiveError(err)),
            })
        })
    }
}

impl<S, T> futures::Sink<T> for TypedStream<S, T>
where
    S: futures::Sink<Bytes, Error = std::io::Error> + Unpin,
    T: Serialize,
{
    type Error = ProtocolError;

    fn poll_ready(
        mut self: Pin<&mut Self>,
        ctx: &mut Context<'_>,
    ) -> Poll<Result<(), <Self as futures::Sink<T>>::Error>> {
        let p = Pin::new(&mut self.stream).poll_ready(ctx);
        p.map(|r| {
            r.map_err(|e| {
                // log::error!("Send error: {:?}", e);
                ProtocolError::StreamSendError(e)
            })
        })
    }

    fn start_send(mut self: Pin<&mut Self>, x: T) -> Result<(), <Self as futures::Sink<T>>::Error> {
        // TODO: to_writer instead?
        // serde_cbor::to_writer(&stream, &response);
        match serde_cbor::to_vec(&x) {
            Err(err) => Err(ProtocolError::SerializationError(err)),
            Ok(cbor) => {
                let p = Pin::new(&mut self.stream).start_send(cbor.into());
                p.map_err(|e| ProtocolError::StreamSendError(e))
            }
        }
    }

    fn poll_flush(
        mut self: Pin<&mut Self>,
        ctx: &mut Context<'_>,
    ) -> Poll<Result<(), <Self as futures::Sink<T>>::Error>> {
        let p = Pin::new(&mut self.stream).poll_flush(ctx);
        p.map_err(|e| ProtocolError::StreamSendError(e))
    }

    fn poll_close(
        mut self: Pin<&mut Self>,
        ctx: &mut Context<'_>,
    ) -> Poll<Result<(), <Self as futures::Sink<T>>::Error>> {
        let p = Pin::new(&mut self.stream).poll_close(ctx);
        p.map_err(|e| ProtocolError::StreamSendError(e))
    }
}

impl<S, T> Stream<T> for TypedStream<S, T>
where
    S: futures::Stream<Item = Result<BytesMut, std::io::Error>> + Send,
    S: futures::Sink<Bytes, Error = std::io::Error>,
    S: Unpin,
    S: Sync,
    T: for<'a> Deserialize<'a> + Serialize,
{
}

pub struct Channel<T> {
    send: Sender<T>,
    recv: Receiver<T>,
}

impl<T> Channel<T> {
    pub fn new_pair(capacity: usize) -> (Channel<T>, Channel<T>) {
        let (send1, recv1) = tokio::sync::mpsc::channel(capacity);
        let (send2, recv2) = tokio::sync::mpsc::channel(capacity);
        let c1 = Channel {
            send: send1,
            recv: recv2,
        };
        let c2 = Channel {
            send: send2,
            recv: recv1,
        };
        (c1, c2)
    }
}

impl<T> Stream<T> for Channel<T>
where
    Channel<T>: Sync,
    Channel<T>: Send,
{
}

impl<T> futures::Stream for Channel<T> {
    type Item = Result<T, ProtocolError>;

    fn poll_next(
        mut self: Pin<&mut Self>,
        ctx: &mut Context<'_>,
    ) -> Poll<Option<Result<T, ProtocolError>>> {
        todo!()
        // let p = futures::Stream::poll_next(Pin::new(&mut self.recv), ctx);
        // p.map(|o| o.map(|t| Ok(t)))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        todo!()
        // self.recv.size_hint()
    }
}

impl<T> futures::Sink<T> for Channel<T> {
    type Error = ProtocolError;

    fn poll_ready(
        mut self: Pin<&mut Self>,
        ctx: &mut Context<'_>,
    ) -> Poll<Result<(), <Self as futures::Sink<T>>::Error>> {
        todo!()
        // let p = Pin::new(&mut self.send).poll_ready(ctx);
        // p.map(|r| {
        //     r.map_err(|e| {
        //         log::error!("Send error: {:?}", e);
        //         ProtocolError::StreamSendError(std::io::Error::other("poll_ready error"))
        //     })
        // })
    }

    fn start_send(mut self: Pin<&mut Self>, x: T) -> Result<(), <Self as futures::Sink<T>>::Error> {
        todo!()
        // let p = Pin::new(&mut self.send).start_send(x);
        // p.map_err(|e| {
        //     log::error!("Send error: {:?}", e);
        //     ProtocolError::StreamSendError(std::io::Error::other("start_send error"))
        // })
    }

    fn poll_flush(
        mut self: Pin<&mut Self>,
        ctx: &mut Context<'_>,
    ) -> Poll<Result<(), <Self as futures::Sink<T>>::Error>> {
        todo!()
        // let p = Pin::new(&mut self.send).poll_flush(ctx);
        // p.map(|r| {
        //     r.map_err(|e| {
        //         log::error!("Send error: {:?}", e);
        //         ProtocolError::StreamSendError(std::io::Error::other("poll_flush error"))
        //     })
        // })
    }

    fn poll_close(
        mut self: Pin<&mut Self>,
        ctx: &mut Context<'_>,
    ) -> Poll<Result<(), <Self as futures::Sink<T>>::Error>> {
        todo!()
        // let p = Pin::new(&mut self.send).poll_close(ctx);
        // p.map(|r| {
        //     r.map_err(|e| {
        //         log::error!("Send error: {:?}", e);
        //         ProtocolError::StreamSendError(std::io::Error::other("poll_close error"))
        //     })
        // })
    }
}

#[cfg(test)]
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

#[cfg(test)]
pub struct UnboundChannel<T> {
    send: UnboundedSender<T>,
    recv: UnboundedReceiver<T>,
}

#[cfg(test)]
impl<T> UnboundChannel<T> {
    pub fn new_pair() -> (UnboundChannel<T>, UnboundChannel<T>) {
        let (send1, recv1) = tokio::sync::mpsc::unbounded_channel();
        let (send2, recv2) = tokio::sync::mpsc::unbounded_channel();
        let c1 = UnboundChannel {
            send: send1,
            recv: recv2,
        };
        let c2 = UnboundChannel {
            send: send2,
            recv: recv1,
        };
        (c1, c2)
    }
}

#[cfg(test)]
impl<T> Stream<T> for UnboundChannel<T>
where
    UnboundChannel<T>: Sync,
    UnboundChannel<T>: Send,
{
}

#[cfg(test)]
impl<T> futures::Stream for UnboundChannel<T> {
    type Item = Result<T, ProtocolError>;

    fn poll_next(
        mut self: Pin<&mut Self>,
        ctx: &mut Context<'_>,
    ) -> Poll<Option<Result<T, ProtocolError>>> {
        todo!()
        // let p = futures::Stream::poll_next(Pin::new(&mut self.recv), ctx);
        // p.map(|o| o.map(|t| Ok(t)))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        todo!()
        // self.recv.size_hint()
    }
}

#[cfg(test)]
impl<T> futures::Sink<T> for UnboundChannel<T> {
    type Error = ProtocolError;

    fn poll_ready(
        mut self: Pin<&mut Self>,
        ctx: &mut Context<'_>,
    ) -> Poll<Result<(), <Self as futures::Sink<T>>::Error>> {
        todo!()
        // let p = Pin::new(&mut self.send).poll_ready(ctx);
        // p.map(|r| {
        //     r.map_err(|e| {
        //         log::error!("Send error: {:?}", e);
        //         ProtocolError::StreamSendError(std::io::Error::other("poll_ready error"))
        //     })
        // })
    }

    fn start_send(mut self: Pin<&mut Self>, x: T) -> Result<(), <Self as futures::Sink<T>>::Error> {
        todo!()
        // let p = Pin::new(&mut self.send).start_send(x);
        // p.map_err(|e| {
        //     log::error!("Send error: {:?}", e);
        //     ProtocolError::StreamSendError(std::io::Error::other("start_send error"))
        // })
    }

    fn poll_flush(
        mut self: Pin<&mut Self>,
        ctx: &mut Context<'_>,
    ) -> Poll<Result<(), <Self as futures::Sink<T>>::Error>> {
        todo!()
        // let p = Pin::new(&mut self.send).poll_flush(ctx);
        // p.map(|r| {
        //     r.map_err(|e| {
        //         log::error!("Send error: {:?}", e);
        //         ProtocolError::StreamSendError(std::io::Error::other("poll_flush error"))
        //     })
        // })
    }

    fn poll_close(
        mut self: Pin<&mut Self>,
        ctx: &mut Context<'_>,
    ) -> Poll<Result<(), <Self as futures::Sink<T>>::Error>> {
        todo!()
        // let p = Pin::new(&mut self.send).poll_close(ctx);
        // p.map(|r| {
        //     r.map_err(|e| {
        //         log::error!("Send error: {:?}", e);
        //         ProtocolError::StreamSendError(std::io::Error::other("poll_close error"))
        //     })
        // })
    }
}
