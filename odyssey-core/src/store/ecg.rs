
pub mod v0;

pub trait ECGHeader {
    type HeaderId;

    fn get_parent_id(&self) -> Self::HeaderId;
}

#[derive(Clone, Debug)]
pub struct State<Header:ECGHeader> {
    dependency_graph: daggy::Dag<(), ()>, // JP: Hold the operations? Depth?

    // Tips of the ECG (hashes of their headers).
    tips: Vec<Header::HeaderId>,
}

impl<Header:ECGHeader> State<Header> {
    pub fn new() -> State<Header> {
        State {
            dependency_graph: daggy::Dag::new(),
            tips: vec![],
        }
    }

    pub fn tips(&self) -> &[Header::HeaderId] {
        &self.tips
    }

    pub fn get_parents_with_depth(&self, n:&Header::HeaderId) -> Vec<(u64, Header::HeaderId)> {
        unimplemented!{}
    }

    pub fn get_parents(&self, n:&Header::HeaderId) -> Vec<Header::HeaderId> {
        unimplemented!{}
    }

    pub fn get_children_with_depth(&self, n:&Header::HeaderId) -> Vec<(u64, Header::HeaderId)> {
        unimplemented!{}
    }

    // pub fn get_children(&self, n:&HeaderId) -> Vec<HeaderId> {
    //     unimplemented!{}
    // }

    pub fn contains(&self, h:&Header::HeaderId) -> bool {
        unimplemented!{}
    }

    pub fn get_header(&self, n:&Header::HeaderId) -> Header {
        unimplemented!{}
    }

    pub fn get_header_depth(&self, n:&Header::HeaderId) -> u64 {
        unimplemented!{}
    }

    pub fn insert_header(&mut self, header_id: Header::HeaderId, header: Header) -> bool {
        unimplemented!{}
    }
}

/// Tests whether two ecg states have the same DAG.
pub(crate) fn equal_dags<Header:ECGHeader>(l: &State<Header>, r: &State<Header>) -> bool {
    unimplemented!()
}
