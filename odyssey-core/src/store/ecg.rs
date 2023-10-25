
use std::collections::{BTreeMap, BTreeSet};

pub mod v0;

/// Trait that ECG headers (nodes?) must implement.
pub trait ECGHeader {
    type HeaderId: Ord + Copy;

    /// Return the parents ids of a node. If an empty slice is returned, the root node is the
    /// parent.
    fn get_parent_ids(&self) -> &[Self::HeaderId];

    fn validate_header(&self, header_id: Self::HeaderId) -> bool;
}

// /// An internal type for nodes that are actually stored in the DAG.
// enum InternalNode<N> {
//     Root,
//     Node(N),
// }

#[derive(Clone, Debug)]
pub struct State<Header:ECGHeader> {
    dependency_graph: daggy::stable_dag::StableDag<Header::HeaderId, ()>, // JP: Hold the operations? Depth? Do we need StableDag?

    // root_node_idx: daggy::NodeIndex, // JP: Should we actually insert the root node?
    /// Nodes at the top of the DAG that depend on the initial state.
    root_nodes: BTreeSet<Header::HeaderId>,

    /// Mapping from header ids to node indices.
    node_idx_map: BTreeMap<Header::HeaderId, daggy::NodeIndex>,

    /// Tips of the ECG (hashes of their headers).
    tips: Vec<Header::HeaderId>,
}

impl<Header:ECGHeader> State<Header> {
    pub fn new() -> State<Header> {
        let mut dependency_graph = daggy::stable_dag::StableDag::new();
        // let root_node_idx = dependency_graph.add_node(InternalNode::Root);

        State {
            dependency_graph,
            root_nodes: BTreeSet::new(),
            node_idx_map: BTreeMap::new(),
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
        // Validate header.
        if !header.validate_header(header_id) {
            return false;
        }

        // Check that the header is not already in the dependency_graph.
        if self.node_idx_map.contains_key(&header_id) {
            return false;
        }

        let parents = header.get_parent_ids();
        let parent_idxs = if parents.is_empty() {
            let is_new_insert = self.root_nodes.insert(header_id);
            // Check if it already existed.
            if !is_new_insert {
                // TODO: Log that the state is corrupt. Invariant violated that
                // `root_nodes` is a subset of `node_idx_map.keys()`.
                return false;
            }

            vec![]
        } else {
            if let Some(parent_idxs) = parents.iter().map(|parent_id| self.node_idx_map.get(&parent_id).map(|x| *x)).try_collect::<Vec<daggy::NodeIndex>>() {
                parent_idxs
            } else {
                return false;
            }
        };

        // Insert node and store its index in `node_idx_map`.
        // JP: We really want an `add_child` function that takes multiple parents.
        let idx = self.dependency_graph.add_node(header_id);
        if let Err(_) = self.node_idx_map.try_insert(header_id, idx) {
            // TODO: Should be unreachable. Log this.
            return false;
        }

        // Insert edges.
        if let Err(_) = self.dependency_graph.add_edges(parent_idxs.into_iter().map(|parent_idx| (parent_idx, idx, ()))) {
            // TODO: Unreachable? Log this.
            return false;
        }

        true
    }
}

/// Tests whether two ecg states have the same DAG.
pub(crate) fn equal_dags<Header:ECGHeader>(l: &State<Header>, r: &State<Header>) -> bool {
    unimplemented!()
}
