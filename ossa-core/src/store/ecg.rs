#[cfg(test)]
use daggy::petgraph::visit::{EdgeRef, IntoEdgeReferences, IntoNodeReferences, NodeRef};
use daggy::stable_dag::StableDag;
use daggy::Walker;
use ossa_crdt::CRDT;
use std::cmp::{self, Reverse};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt::Debug;
use std::marker::PhantomData;
use tracing::{debug, error};

pub mod v0;

/// Trait that ECG headers (nodes?) must implement.
pub trait ECGHeader {
    type HeaderId: Ord + Copy + Debug;

    // /// Type identifying operations that implements CausalOrder so that it can be used as CRDT::Time.
    // type OperationId;

    // /// Type associated with this header that implements ECGBody.
    // type Body;

    /// Return the parents ids of a node. If an empty slice is returned, the root node is the
    /// parent.
    fn get_parent_ids(&self) -> &[Self::HeaderId];

    /// Computes the identifier of the header.
    fn get_header_id(&self) -> Self::HeaderId;

    fn validate_header(&self, header_id: Self::HeaderId) -> bool;

    // // TODO: Can we return the following instead? impl Iterator<(T::Time, Item = T::Time)>
    // fn zip_operations_with_time<T>(&self, body: Self::Body) -> Vec<(T::Time, T::Op)>
    // where
    //     T: CRDT + Sized,
    //     <Self as ECGHeader>::Body: ECGBody<T>;

    // /// Retrieve the times for each operation in this ECG header and body.
    // // TODO: Can we return the following instead? impl Iterator<Item = T::Time>
    // fn get_operation_times<T>(&self, body: &Self::Body) -> Vec<T::Time> where T: CRDT;
}

pub trait ECGBody<Op, SerializedOp> {
    /// Header type associated with this body.
    type Header: ECGHeader;

    /// Create a new body from a vector of operations.
    // fn new_body(operations: Vec<T::Op<CausalTime<T::Time>>>) -> Self;
    fn new_body(operations: Vec<SerializedOp>) -> Self;

    /// The operations in this body.
    fn operations(
        self,
        header_id: <Self::Header as ECGHeader>::HeaderId,
    ) -> impl Iterator<Item = Op>;

    /// The number of operations in this body.
    fn operations_count(&self) -> u8;

    // fn new_header(&self, parents: BTreeSet<<Self::Header as ECGHeader>::HeaderId>) -> Self::Header
    fn new_header(&self, parents: BTreeSet<<Self::Header as ECGHeader>::HeaderId>) -> Self::Header;
    // fn new_header<HeaderId>(&self, parents: BTreeSet<HeaderId>) -> Self::Header
    // where
    //     // Self::Header: ECGHeader;
    //     Self::Header: ECGHeader<HeaderId = HeaderId>;

    // // TODO: Can we return the following instead? impl Iterator<(T::Time, Item = T::Time)>
    // fn zip_operations_with_time(self, header: &Self::Header) -> Vec<(T::Time, T::Op<T::Time>)>;
    // // where
    // // T: CRDT + Sized,
    // // <Self as ECGHeader>::Body: ECGBody<T>;

    // /// Retrieve the times for each operation in this ECG header and body.
    // // TODO: Can we return the following instead? impl Iterator<Item = T::Time>
    // fn get_operation_times(&self, header: &Self::Header) -> Vec<T::Time>;
}

// Serialized ECG body
pub(crate) type RawECGBody = Vec<u8>;

#[derive(Clone, Debug)]
pub(crate) struct NodeInfo<Header> {
    /// The index of this node in the dependency graph.
    graph_index: daggy::NodeIndex,
    /// The (minimum) depth of this node in the dependency graph.
    depth: u64,
    /// The header this node is storing.
    header: Header,
    /// Raw serialized and potentially encrypted operations.
    operations: RawECGBody,
}

impl<Header> NodeInfo<Header> {
    pub(crate) fn header(&self) -> &Header {
        &self.header
    }

    pub(crate) fn operations(&self) -> &Vec<u8> {
        &self.operations
    }
}

#[derive(Clone, Debug)]
pub struct UntypedState<HeaderId, Header> {
    dependency_graph: StableDag<HeaderId, ()>, // JP: Hold the operations? Depth? Do we need StableDag?

    /// Nodes at the top of the DAG that depend on the initial state.
    root_nodes: BTreeSet<HeaderId>,

    /// Mapping from header ids to node indices.
    node_info_map: BTreeMap<HeaderId, NodeInfo<Header>>,

    /// Tips of the ECG (hashes of their headers).
    /// Invariant: All of these headers are in `node_info_map`.
    tips: BTreeSet<HeaderId>,
}

impl<HeaderId, Header> UntypedState<HeaderId, Header> {
    pub fn tips(&self) -> &BTreeSet<HeaderId> {
        &self.tips
    }

    pub fn contains(&self, h: &HeaderId) -> bool
    where
        HeaderId: Ord,
    {
        if let Some(_node_info) = self.node_info_map.get(h) {
            true
        } else {
            false
        }
    }

    pub fn get_parents(&self, h: &HeaderId) -> Option<Vec<HeaderId>>
    where
        HeaderId: Ord + Copy,
    {
        let node_info = self.node_info_map.get(h)?;
        self.dependency_graph
            .parents(node_info.graph_index)
            .iter(&self.dependency_graph)
            .map(|(_, parent_idx)| self.dependency_graph.node_weight(parent_idx).map(|i| *i))
            .try_collect()
    }

    pub fn get_parents_with_depth(&self, h: &HeaderId) -> Option<Vec<(u64, HeaderId)>>
    where
        HeaderId: Ord + Copy,
    {
        let node_info = self.node_info_map.get(h)?;
        self.dependency_graph
            .parents(node_info.graph_index)
            .iter(&self.dependency_graph)
            .map(|(_, parent_idx)| {
                self.dependency_graph
                    .node_weight(parent_idx)
                    .and_then(|parent_id| {
                        self.node_info_map
                            .get(parent_id)
                            .map(|i| (i.depth, *parent_id))
                    })
            })
            .try_collect()
    }

    /// Returns the children of the given node (with their depths) if it exists. If the returned array is
    /// empty, the node is a leaf node.
    pub fn get_children_with_depth(&self, h: &HeaderId) -> Option<Vec<(Reverse<u64>, HeaderId)>>
    where
        HeaderId: Ord + Copy,
    {
        let node_info = self.node_info_map.get(h)?;
        self.dependency_graph
            .children(node_info.graph_index)
            .iter(&self.dependency_graph)
            .map(|(_, child_idx)| {
                self.dependency_graph
                    .node_weight(child_idx)
                    .and_then(|child_id| {
                        self.node_info_map
                            .get(child_id)
                            .map(|i| (Reverse(i.depth), *child_id))
                    })
            })
            .try_collect()
    }

    pub(crate) fn get_header_depth(&self, n: &HeaderId) -> Option<u64>
    where
        HeaderId: Ord,
    {
        self.node_info_map.get(n).map(|i| i.depth)
    }

    pub(crate) fn get_header(&self, n: &HeaderId) -> Option<&Header>
    where
        HeaderId: Ord,
    {
        self.node_info_map.get(n).map(|i| &i.header)
    }

    pub(crate) fn get_node(&self, n: &HeaderId) -> Option<&NodeInfo<Header>>
    where
        HeaderId: Ord,
    {
        self.node_info_map.get(n)
    }

    pub(crate) fn is_root_node(&self, h: &HeaderId) -> bool
    where
        HeaderId: Ord,
    {
        self.root_nodes.contains(h)
    }

    pub fn get_root_nodes_with_depth<'a>(
        &'a self,
    ) -> impl Iterator<Item = (Reverse<u64>, HeaderId)> + 'a
    where
        HeaderId: Copy,
    {
        // All root nodes have depth 1.
        self.root_nodes.iter().map(|h| (Reverse(1), *h))
    }
}

#[derive(Debug)]
pub struct State<Header: ECGHeader, T> {
    pub(crate) state: UntypedState<Header::HeaderId, Header>,

    phantom: PhantomData<fn(T)>, // TODO: Delete T?
}

impl<Header: ECGHeader + Clone, T: CRDT> Clone for State<Header, T> {
    fn clone(&self) -> Self {
        let state = self.state.clone();
        State {
            state,
            phantom: PhantomData,
        }
    }
}

impl<Header: ECGHeader, T: CRDT> State<Header, T> {
    pub fn new() -> State<Header, T> {
        let state = UntypedState {
            dependency_graph: StableDag::new(),
            root_nodes: BTreeSet::new(),
            node_info_map: BTreeMap::new(),
            tips: BTreeSet::new(),
        };
        State {
            state,
            phantom: PhantomData,
        }
    }

    pub fn tips(&self) -> &BTreeSet<Header::HeaderId> {
        &self.state.tips
    }

    pub fn is_root_node(&self, h: &Header::HeaderId) -> bool {
        self.state.is_root_node(h)
    }

    /// Returns the parents of the given node (with their depths) if it exists. If the returned array is
    /// empty, the node is a root node.
    pub fn get_parents_with_depth(
        &self,
        h: &Header::HeaderId,
    ) -> Option<Vec<(u64, Header::HeaderId)>> {
        self.state.get_parents_with_depth(h)
    }

    /// Returns the parents of the given node if it exists. If the returned array is
    /// empty, the node is a root node.
    pub fn get_parents(&self, h: &Header::HeaderId) -> Option<Vec<Header::HeaderId>> {
        self.state.get_parents(h)
    }

    /// Returns the children of the given node (with their depths) if it exists. If the returned array is
    /// empty, the node is a leaf node.
    pub fn get_children_with_depth(
        &self,
        h: &Header::HeaderId,
    ) -> Option<Vec<(Reverse<u64>, Header::HeaderId)>> {
        self.state.get_children_with_depth(h)
    }

    // pub fn get_children(&self, n:&HeaderId) -> Vec<HeaderId> {
    //     unimplemented!{}
    // }

    pub fn contains(&self, h: &Header::HeaderId) -> bool {
        self.state.contains(h)
    }

    pub fn get_header(&self, n: &Header::HeaderId) -> Option<&Header> {
        self.state.get_header(n)
    }

    pub fn get_header_depth(&self, n: &Header::HeaderId) -> Option<u64> {
        self.state.get_header_depth(n)
    }

    pub fn insert_header(&mut self, header: Header, operations: RawECGBody) -> bool {
        let header_id = header.get_header_id();

        // Validate header.
        if !header.validate_header(header_id) {
            debug!("Invalid header: {header_id:?}");
            return false;
        }

        // Check that the header is not already in the dependency_graph.
        if self.state.node_info_map.contains_key(&header_id) {
            debug!("Already have header: {header_id:?}");
            return false;
        }

        let parents = header.get_parent_ids();
        let (parent_idxs, depth) = if parents.is_empty() {
            let is_new_insert = self.state.root_nodes.insert(header_id);
            // Check if it already existed.
            if !is_new_insert {
                // TODO: Log that the state is corrupt. Invariant violated that
                // `root_nodes` is a subset of `node_idx_map.keys()`.
                error!("Invariant violated: Header already existed in root_nodes but not in node_info_map: {header_id:?}");
                return false;
            }

            // Update tips since the new node is a leaf.
            self.state.tips.insert(header_id);

            (vec![], 1)
        } else {
            let mut depth = u64::MAX;
            if let Some(parent_idxs) = parents
                .iter()
                .map(|parent_id| {
                    self.state.node_info_map.get(&parent_id).map(|i| {
                        depth = cmp::min(depth, i.depth);
                        i.graph_index
                    })
                })
                .try_collect::<Vec<daggy::NodeIndex>>()
            {
                // If any parents were previously a tip, remove from tips.
                parents.iter().for_each(|parent_id| {
                    self.state.tips.remove(parent_id);
                });
                // Insert as tip since received headers must (currently) be a leaf.
                self.state.tips.insert(header_id);

                (parent_idxs, depth + 1)
            } else {
                // They sent us a header when we don't know the parent.
                error!("They sent us a header but we don't know its parents: {header_id:?}");
                return false;
            }
        };

        // Insert node and store its index in `node_idx_map`.
        // JP: We really want an `add_child` function that takes multiple parents.
        let graph_index = self.state.dependency_graph.add_node(header_id);
        let node_info = NodeInfo {
            graph_index: graph_index.clone(),
            depth,
            header,
            operations,
        };
        if let Err(_) = self.state.node_info_map.try_insert(header_id, node_info) {
            // TODO: Should be unreachable. Log this.
            error!("Unreachable: We already checked that it doesn't exist in node_info_map: {header_id:?}");
            return false;
        }

        // Insert edges.
        if let Err(_) = self.state.dependency_graph.add_edges(
            parent_idxs
                .into_iter()
                .map(|parent_idx| (parent_idx, graph_index, ())),
        ) {
            // TODO: Unreachable? Log this.
            error!("Invariant violated: Header already existed in dependency_graph but not in node_info_map: {header_id:?}");
            return false;
        }

        true
    }

    /// Perform a BFS to check if `ancestor` is an ancestor of `descendent`. Returns `None` if
    /// either header id is not in the graph.
    fn is_ancestor_of(
        &self,
        ancestor: &Header::HeaderId,
        descendent: &Header::HeaderId,
    ) -> Option<bool> {
        let anid = self.state.node_info_map.get(ancestor)?.graph_index;
        let dnid = self.state.node_info_map.get(descendent)?.graph_index;

        let mut queue = VecDeque::from([dnid]);
        let mut visited = BTreeSet::from([dnid]);

        while let Some(nid) = queue.pop_front() {
            if nid == anid {
                return Some(true);
            }

            for (_, pid) in self
                .state
                .dependency_graph
                .parents(nid)
                .iter(&self.state.dependency_graph)
            {
                if !visited.contains(&pid) {
                    visited.insert(pid);
                    queue.push_back(pid);
                }
            }
        }

        Some(false)
    }

    pub fn state(&self) -> &UntypedState<Header::HeaderId, Header> {
        &self.state
    }
}

/// Tests whether two ecg states have the same DAG.
#[cfg(test)]
pub(crate) fn equal_dags<Header: ECGHeader, T>(l: &State<Header, T>, r: &State<Header, T>) -> bool
where
    Header::HeaderId: Copy,
{
    let edges = |g: &StableDag<Header::HeaderId, ()>| {
        g.edge_references()
            .map(|e| {
                let n1 = g.node_weight(e.source()).unwrap();
                let n2 = g.node_weight(e.target()).unwrap();
                (*n1, *n2)
            })
            .collect()
    };
    let nodes =
        |g: &StableDag<Header::HeaderId, ()>| g.node_references().map(|n| *n.weight()).collect();

    let node_set_left: BTreeSet<_> = nodes(&l.state.dependency_graph);
    let node_set_right = nodes(&r.state.dependency_graph);
    let edge_set_left: BTreeSet<_> = edges(&l.state.dependency_graph);
    let edge_set_right = edges(&r.state.dependency_graph);

    l.state.root_nodes == r.state.root_nodes
        && l.state.tips == r.state.tips
        && edge_set_left == edge_set_right
        && node_set_left == node_set_right
}

#[cfg(test)]
pub(crate) fn print_dag<Header: ECGHeader, T>(s: &State<Header, T>) {
    use petgraph::dot::{Config, Dot};
    use petgraph::stable_graph::StableDiGraph;

    let mut g = s
        .state
        .dependency_graph
        .map(|_i, n| format!("{:?}", n), |_i, e| e);

    // Add root node.
    let root = g.add_node("".to_string());
    for n in &s.state.root_nodes {
        g.add_edge(root, s.state.node_info_map[n].graph_index, &());
    }

    let g: StableDiGraph<_, _> = g.into();
    let d = Dot::with_config(&g, &[Config::EdgeNoLabel]);
    println!("{:?}", d);
}
