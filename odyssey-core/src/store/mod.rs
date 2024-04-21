pub mod ecg;
pub mod v0; // TODO: Move this to network::protocol

pub use v0::{MetadataBody, MetadataHeader, Nonce};

pub struct State<Header: ecg::ECGHeader> {
    ecg_state: ecg::State<Header>,
}

impl<Header: ecg::ECGHeader> State<Header> {
    pub fn new() -> State<Header> {
        State {
            ecg_state: ecg::State::new(),
        }
    }
}
