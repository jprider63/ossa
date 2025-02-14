use odyssey_crdt::CRDT;
use serde::Serialize;
use std::collections::BTreeSet;
use typeable::{TypeId, Typeable};

use crate::util;

pub mod ecg;
pub mod v0; // TODO: Move this to network::protocol

pub use v0::{MetadataBody, MetadataHeader, Nonce};


pub struct State<Header: ecg::ECGHeader<T>, T: CRDT, Hash> {
    pub(crate) store_header: MetadataHeader<Hash>,
    pub(crate) ecg_state: ecg::State<Header, T>,
    pub(crate) decrypted_state: Option<DecryptedState<Header, T>>, // JP: Is this actually used?
                                                                   // Does it make sense?
}

pub struct DecryptedState<Header: ecg::ECGHeader<T>, T: CRDT> {
    /// Latest ECG application state we've seen.
    latest_state: T,

    /// Headers corresponding to the latest ECG application state.
    latest_headers: BTreeSet<Header::HeaderId>,
}

impl<Header: ecg::ECGHeader<T>, T: CRDT, Hash: util::Hash> State<Header, T, Hash> {
    pub fn new(initial_state: T) -> State<Header, T, Hash>
    where
        T: Serialize + Typeable,
    {
        let init_body = MetadataBody::new(&initial_state);
        let store_header = MetadataHeader::generate::<T>(&init_body);
        let decrypted_state = DecryptedState {
            latest_state: initial_state,
            latest_headers: BTreeSet::new(),
        };

        State {
            store_header,
            ecg_state: ecg::State::new(),
            decrypted_state: Some(decrypted_state),
        }
    }

    pub fn store_id(&self) -> Hash {
        self.store_header.store_id()
    }
}
