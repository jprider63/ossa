use crate::store::dag;

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
