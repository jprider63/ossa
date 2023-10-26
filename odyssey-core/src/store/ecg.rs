
use std::cmp;
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

#[derive(Copy, Clone, Debug)]
struct NodeInfo {
    /// The index of this node in the dependency graph.
    graph_index: daggy::NodeIndex,
    /// The (minimum) depth of this node in the dependency graph.
    depth: u64,
}

#[derive(Clone, Debug)]
pub struct State<Header:ECGHeader> {
    dependency_graph: daggy::stable_dag::StableDag<Header::HeaderId, ()>, // JP: Hold the operations? Depth? Do we need StableDag?

    // root_node_idx: daggy::NodeIndex, // JP: Should we actually insert the root node?
    /// Nodes at the top of the DAG that depend on the initial state.
    root_nodes: BTreeSet<Header::HeaderId>,

    /// Mapping from header ids to node indices.
    node_info_map: BTreeMap<Header::HeaderId, NodeInfo>,

    /// Tips of the ECG (hashes of their headers).
    tips: BTreeSet<Header::HeaderId>,
}

impl<Header:ECGHeader> State<Header> {
    pub fn new() -> State<Header> {
        let mut dependency_graph = daggy::stable_dag::StableDag::new();
        // let root_node_idx = dependency_graph.add_node(InternalNode::Root);

        State {
            dependency_graph,
            root_nodes: BTreeSet::new(),
            node_info_map: BTreeMap::new(),
            tips: BTreeSet::new(),
        }
    }

    pub fn tips(&self) -> &[Header::HeaderId] {
        unimplemented!()
        // &self.tips.iter()
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
        if self.node_info_map.contains_key(&header_id) {
            return false;
        }

        let parents = header.get_parent_ids();
        let (parent_idxs, depth) = if parents.is_empty() {
            let is_new_insert = self.root_nodes.insert(header_id);
            // Check if it already existed.
            if !is_new_insert {
                // TODO: Log that the state is corrupt. Invariant violated that
                // `root_nodes` is a subset of `node_idx_map.keys()`.
                return false;
            }

            (vec![], 1)
        } else {
            let mut depth = u64::MAX;
            if let Some(parent_idxs) = parents.iter().map(|parent_id| self.node_info_map.get(&parent_id).map(|i| {
                depth = cmp::min(depth, i.depth);
                i.graph_index
              })).try_collect::<Vec<daggy::NodeIndex>>() {
                (parent_idxs, depth + 1)
            } else {
                return false;
            }
        };

        // Insert node and store its index in `node_idx_map`.
        // JP: We really want an `add_child` function that takes multiple parents.
        let node_info = NodeInfo {
            graph_index: self.dependency_graph.add_node(header_id),
            depth,
        };
        if let Err(_) = self.node_info_map.try_insert(header_id, node_info) {
            // TODO: Should be unreachable. Log this.
            return false;
        }

        // Insert edges.
        if let Err(_) = self.dependency_graph.add_edges(parent_idxs.into_iter().map(|parent_idx| (parent_idx, node_info.graph_index, ()))) {
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
