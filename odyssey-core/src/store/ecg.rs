
#[derive(Clone, Debug)]
pub struct State<HeaderId> {
    dependency_graph: daggy::Dag<(), ()>, // JP: Hold the operations? Depth?

    // Tips of the ECG (hashes of their headers).
    tips: Vec<HeaderId>,
}

impl<HeaderId> PartialEq for State<HeaderId> {
    fn eq(&self, other: &Self) -> bool{
        unimplemented!{}
    }
}

impl<HeaderId> State<HeaderId> {
    pub fn new() -> State<HeaderId> {
        State {
            dependency_graph: daggy::Dag::new(),
            tips: vec![],
        }
    }

    pub fn tips(&self) -> &[HeaderId] {
        &self.tips
    }

    pub fn get_parents_with_depth(&self, n:&HeaderId) -> Vec<(u64, HeaderId)> {
        unimplemented!{}
    }

    pub fn get_parents(&self, n:&HeaderId) -> Vec<HeaderId> {
        unimplemented!{}
    }

    pub fn get_children_with_depth(&self, n:&HeaderId) -> Vec<(u64, HeaderId)> {
        unimplemented!{}
    }

    // pub fn get_children(&self, n:&HeaderId) -> Vec<HeaderId> {
    //     unimplemented!{}
    // }

    pub fn contains(&self, h:&HeaderId) -> bool {
        unimplemented!{}
    }

    pub fn get_header<Header>(&self, n:&HeaderId) -> Header {
        unimplemented!{}
    }

    pub fn get_header_depth(&self, n:&HeaderId) -> u64 {
        unimplemented!{}
    }

    pub fn insert_header<Header>(&mut self, header: Header) -> bool {
        unimplemented!{}
    }
}

/// Tests whether two ecg states have the same DAG.
pub(crate) fn equal_dags<HeaderId>(l: &State<HeaderId>, r: &State<HeaderId>) -> bool {
    unimplemented!()
}
