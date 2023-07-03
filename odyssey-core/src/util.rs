
use bytes::{Bytes,BytesMut};
use futures;
use rand::{RngCore, rngs::OsRng};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt::Debug;

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
pub trait Stream: futures::Stream<Item=Result<BytesMut,std::io::Error>>
    + futures::Sink<Bytes, Error=std::io::Error>
    + Unpin
{}
impl<T> Stream for T
where
    T:futures::Stream<Item=Result<BytesMut,std::io::Error>>,
    T:futures::Sink<Bytes, Error=std::io::Error>,
    T:Unpin,
{}
