use odyssey_crdt::CRDT;

pub mod ecg;
pub mod v0; // TODO: Move this to network::protocol

pub use v0::{MetadataBody, MetadataHeader, Nonce};

use std::collections::BTreeSet;

pub struct State<Header: ecg::ECGHeader<T>, T: CRDT> {
    ecg_state: ecg::State<Header, T>,
    decrypted_state: Option<DecryptedState<Header, T>>,
}

pub struct DecryptedState<Header: ecg::ECGHeader<T>, T: CRDT> {
    /// Latest ECG application state we've seen.
    latest_state: T,

    /// Headers corresponding to the latest ECG application state.
    latest_headers: BTreeSet<Header::HeaderId>,
}

impl<Header: ecg::ECGHeader<T>, T: CRDT> State<Header, T> {
    pub fn new(initial_state: T) -> State<Header, T> {
        let decrypted_state = DecryptedState {
            latest_state: initial_state,
            latest_headers: BTreeSet::new(),
        };

        State {
            ecg_state: ecg::State::new(),
            decrypted_state: Some(decrypted_state),
        }
    }
}
