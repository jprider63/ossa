use bytes::{Bytes, BytesMut};
use futures;
use futures::task::{Context, Poll};
use rand::{rngs::OsRng, RngCore};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
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
    S: futures::Stream<Item = Result<BytesMut, std::io::Error>>,
{
    type Item = Result<T, ProtocolError>;
    fn poll_next(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
    ) -> Poll<std::option::Option<<Self as futures::Stream>::Item>> {
        todo!()
    }
}

impl<S, T> futures::Sink<T> for TypedStream<S, T>
where
    S: futures::Sink<Bytes, Error = std::io::Error>,
{
    type Error = ProtocolError;

    fn poll_ready(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
    ) -> Poll<Result<(), <Self as futures::Sink<T>>::Error>> {
        todo!()
    }
    fn start_send(self: Pin<&mut Self>, _: T) -> Result<(), <Self as futures::Sink<T>>::Error> {
        todo!()
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
    S: futures::Stream<Item = Result<BytesMut, std::io::Error>>,
    S: futures::Sink<Bytes, Error = std::io::Error>,
    S: Unpin,
    S: Sync,
{
}

// impl<S, T> Stream<T> for TypedStream<S>
// where
//     S:futures::Stream<Item=Result<BytesMut,std::io::Error>>,
//     S:futures::Sink<Bytes, Error=std::io::Error>,
//     S:Unpin,
//     S:Sync,

// impl<S, T> Stream<T> for S
// where
//     S:futures::Stream<Item=Result<BytesMut,std::io::Error>>,
//     S:futures::Sink<Bytes, Error=std::io::Error>,
//     S:Unpin,
//     S:Sync,
// {}

// use futures::task::{Context, Poll};
#[cfg(test)]
use futures_channel::mpsc::{UnboundedReceiver, UnboundedSender};

#[cfg(test)]
pub struct Channel<T> {
    send: UnboundedSender<T>,
    recv: UnboundedReceiver<T>,
}

#[cfg(test)]
impl<T> Channel<T> {
    pub fn new() -> Channel<T> {
        todo!()
    }
}

#[cfg(test)]
impl<T> Stream<T> for Channel<T>
where
// //     S: futures::Stream<Item = Result<BytesMut, std::io::Error>>,
// //     S: futures::Sink<Bytes, Error = std::io::Error>,
// //     S: Unpin,
    Channel<T>: Sync,
{
}

#[cfg(test)]
impl<T> futures::Stream for Channel<T> {
    type Item = Result<T, ProtocolError>;
    fn poll_next(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
    ) -> Poll<Option<<Self as futures::Stream>::Item>> {
        todo!()
    }
}

#[cfg(test)]
// impl<T> futures::Sink<bytes::Bytes> for Channel<T> {
impl<T> futures::Sink<T> for Channel<T> {
    type Error = ProtocolError;

    fn poll_ready(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
    ) -> Poll<Result<(), <Self as futures::Sink<T>>::Error>> {
        todo!()
    }
    fn start_send(self: Pin<&mut Self>, _: T) -> Result<(), <Self as futures::Sink<T>>::Error> {
        todo!()
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
