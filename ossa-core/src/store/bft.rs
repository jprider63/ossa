use crate::store::dag;

/// A round in the BFT strong consistency protocol.
pub type Round = u32;

/// Trait that abstracts over strongly consistent data types that require linearizability.
pub trait SCDT {
    type Operation;

    fn update(self, op: Self::Operation) -> Self;

    fn is_valid_operation(self, op: Self::Operation) -> bool;
}

pub(crate) struct State<Header: dag::ECGHeader, S> {
    dag_state: dag::State<Header, S>,
}

impl<Header: dag::ECGHeader, S> State<Header, S> {
    pub(crate) fn new() -> Self {
        Self {
            dag_state: dag::State::new(),
        }
    }
}
