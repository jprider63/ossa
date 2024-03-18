use bytes::{Bytes, BytesMut};
use futures;
use futures::task::{Context, Poll};
use rand::{rngs::OsRng, RngCore};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::any::type_name;
use std::fmt::Debug;
use std::marker::PhantomData;
use std::pin::Pin;

use crate::network::protocol::ProtocolError;
use crate::store::Nonce;

/// Generate a random nonce.
pub(crate) fn generate_nonce() -> Nonce {
    let mut nonce = [0; 32];
    OsRng.fill_bytes(&mut nonce);
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

pub trait Hash: PartialEq + AsRef<[u8]> {
    type HashState;

    fn new() -> Self::HashState;
    fn update(state: &mut Self::HashState, data: impl AsRef<[u8]>);
    fn finalize(state: Self::HashState) -> Self;
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Sha256Hash(pub [u8; 32]);

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

// TODO: Generalize the error and stream.
pub trait Stream<T>:
      futures::Stream<Item=Result<T,ProtocolError>> // Result<BytesMut,std::io::Error>>
    + futures::Sink<T, Error=ProtocolError>
    + Unpin
    + Send // JP: This is needed for async_recursion. Not sure if this makes sense in practice.
    + Sync // JP: This is needed for async_recursion. Not sure if this makes sense in practice.
{}

// #[derive(Unpin)]
// #[derive(Sync)]
// TODO: Move this somewhere else
pub struct TypedStream<S, T> {
    stream: S,
    phantom: PhantomData<T>,
}

// TODO: Why is this necessary?
unsafe impl<S, T> Send for TypedStream<S, T> where S: Send {}
// TODO: Why is this necessary?
unsafe impl<S, T> Sync for TypedStream<S, T> where S: Sync {}
// TODO: Why is this necessary?
impl<S, T> Unpin for TypedStream<S, T> where S: Unpin {}

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
        p.map(|o| o.map(|t| match t {
            Ok(bytes) => serde_cbor::from_slice(&bytes).map_err(|err| {
                // log::error!("Failed to parse type {}: {}", type_name::<T>(), err);
                ProtocolError::DeserializationError(err)
            }),
            Err(err) => Err(ProtocolError::StreamReceiveError(err)),
        }))
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
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
    ) -> Poll<Result<(), <Self as futures::Sink<T>>::Error>> {
        todo!()
    }
    fn poll_close(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
    ) -> Poll<Result<(), <Self as futures::Sink<T>>::Error>> {
        todo!()
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

#[cfg(test)]
use futures_channel::mpsc::{UnboundedReceiver, UnboundedSender};

#[cfg(test)]
pub struct Channel<T> {
    send: UnboundedSender<T>,
    recv: UnboundedReceiver<T>,
}

#[cfg(test)]
impl<T> Channel<T> {
    pub fn new_pair() -> (Channel<T>, Channel<T>) {
        let (send1, recv1) = futures_channel::mpsc::unbounded();
        let (send2, recv2) = futures_channel::mpsc::unbounded();
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

#[cfg(test)]
impl<T> Stream<T> for Channel<T>
where
    Channel<T>: Sync,
    Channel<T>: Send,
{
}

#[cfg(test)]
impl<T> futures::Stream for Channel<T> {
    type Item = Result<T, ProtocolError>;

    fn poll_next(
        mut self: Pin<&mut Self>,
        ctx: &mut Context<'_>,
    ) -> Poll<Option<Result<T, ProtocolError>>> {
        let p = futures::Stream::poll_next(Pin::new(&mut self.recv), ctx);
        p.map(|o| o.map(|t| Ok(t)))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.recv.size_hint()
    }
}

#[cfg(test)]
impl<T> futures::Sink<T> for Channel<T> {
    type Error = ProtocolError;

    fn poll_ready(
        mut self: Pin<&mut Self>,
        ctx: &mut Context<'_>,
    ) -> Poll<Result<(), <Self as futures::Sink<T>>::Error>> {
        let p = Pin::new(&mut self.send).poll_ready(ctx);
        p.map(|r| {
            r.map_err(|e| {
                log::error!("Send error: {:?}", e);
                ProtocolError::StreamSendError(std::io::Error::other("poll_ready error"))
            })
        })
    }

    fn start_send(
        mut self: Pin<&mut Self>,
        x: T,
    ) -> Result<(), <Self as futures::Sink<T>>::Error> {
        let p = Pin::new(&mut self.send).start_send(x);
        p.map_err(|e| {
            log::error!("Send error: {:?}", e);
            ProtocolError::StreamSendError(std::io::Error::other("start_send error"))
        })
    }

    fn poll_flush(
        mut self: Pin<&mut Self>,
        ctx: &mut Context<'_>,
    ) -> Poll<Result<(), <Self as futures::Sink<T>>::Error>> {
        let p = Pin::new(&mut self.send).poll_flush(ctx);
        p.map(|r| {
            r.map_err(|e| {
                log::error!("Send error: {:?}", e);
                ProtocolError::StreamSendError(std::io::Error::other("poll_flush error"))
            })
        })
    }

    fn poll_close(
        mut self: Pin<&mut Self>,
        ctx: &mut Context<'_>,
    ) -> Poll<Result<(), <Self as futures::Sink<T>>::Error>> {
        let p = Pin::new(&mut self.send).poll_close(ctx);
        p.map(|r| {
            r.map_err(|e| {
                log::error!("Send error: {:?}", e);
                ProtocolError::StreamSendError(std::io::Error::other("poll_close error"))
            })
        })
    }
}
