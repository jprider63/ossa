use daggy::petgraph::visit::{Bfs, EdgeRef, IntoEdgeReferences, IntoNodeReferences, NodeRef, Reversed};
use daggy::stable_dag::StableDag;
use daggy::Walker;
use odyssey_crdt::CRDT;
use std::cmp::{self, Reverse};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Debug;
use std::marker::PhantomData;

pub mod v0;

/// Trait that ECG headers (nodes?) must implement.
pub trait ECGHeader<T:CRDT> {
    type HeaderId: Ord + Copy + Debug;

    // /// Type identifying operations that implements CausalOrder so that it can be used as CRDT::Time.
    // type OperationId;

    /// Type associated with this header that implements ECGBody.
    type Body;

    /// Return the parents ids of a node. If an empty slice is returned, the root node is the
    /// parent.
    fn get_parent_ids(&self) -> &[Self::HeaderId];

    /// Computes the identifier of the header.
    fn get_header_id(&self) -> Self::HeaderId;

    fn validate_header(&self, header_id: Self::HeaderId) -> bool;

    fn new_header(parents: BTreeSet<Self::HeaderId>, body: &Self::Body) -> Self;

    // TODO: Can we return the following instead? impl Iterator<(T::Time, Item = T::Time)>
    fn zip_operations_with_time(&self, body: Self::Body) -> Vec<(T::Time, T::Op)>
    where
        <Self as ECGHeader<T>>::Body: ECGBody<T>,
    ;

    /// Retrieve the times for each operation in this ECG header and body.
    // TODO: Can we return the following instead? impl Iterator<Item = T::Time>
    fn get_operation_times(&self, body: &Self::Body) -> Vec<T::Time>;
}

pub trait ECGBody<T:CRDT> {
    /// Create a new body from a vector of operations.
    fn new_body(operations: Vec<T::Op>) -> Self;

    /// The operations in this body.
    fn operations(self) -> impl Iterator<Item = T::Op>;

    /// The number of operations in this body.
    fn operations_count(&self) -> u8;
}

#[derive(Clone, Debug)]
struct NodeInfo<Header> {
    /// The index of this node in the dependency graph.
    graph_index: daggy::NodeIndex,
    /// The (minimum) depth of this node in the dependency graph.
    depth: u64,
    /// The header this node is storing.
    header: Header,
}

#[derive(Debug)]
pub struct State<Header: ECGHeader<T>, T: CRDT> {
    dependency_graph: StableDag<Header::HeaderId, ()>, // JP: Hold the operations? Depth? Do we need StableDag?

    /// Nodes at the top of the DAG that depend on the initial state.
    root_nodes: BTreeSet<Header::HeaderId>,

    /// Mapping from header ids to node indices.
    node_info_map: BTreeMap<Header::HeaderId, NodeInfo<Header>>,

    /// Tips of the ECG (hashes of their headers).
    tips: BTreeSet<Header::HeaderId>,

    phantom: PhantomData<T>,
}

impl<Header: ECGHeader<T> + Clone, T: CRDT> Clone for State<Header, T> {
    fn clone(&self) -> Self {
        State {
            dependency_graph: self.dependency_graph.clone(),
            root_nodes: self.root_nodes.clone(),
            node_info_map: self.node_info_map.clone(),
            tips: self.tips.clone(),
            phantom: PhantomData,
        }
    }
}

impl<Header: ECGHeader<T>, T: CRDT> State<Header, T> {
    pub fn new() -> State<Header, T> {
        State {
            dependency_graph: StableDag::new(),
            root_nodes: BTreeSet::new(),
            node_info_map: BTreeMap::new(),
            tips: BTreeSet::new(),
            phantom: PhantomData,
        }
    }

    pub fn tips(&self) -> &BTreeSet<Header::HeaderId> {
        &self.tips
    }

    pub fn is_root_node(&self, h: &Header::HeaderId) -> bool {
        self.root_nodes.contains(h)
    }

    /// Returns the parents of the given node (with their depths) if it exists. If the returned array is
    /// empty, the node is a root node.
    pub fn get_parents_with_depth(
        &self,
        h: &Header::HeaderId,
    ) -> Option<Vec<(u64, Header::HeaderId)>> {
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

    /// Returns the parents of the given node if it exists. If the returned array is
    /// empty, the node is a root node.
    pub fn get_parents(&self, h: &Header::HeaderId) -> Option<Vec<Header::HeaderId>> {
        let node_info = self.node_info_map.get(h)?;
        self.dependency_graph
            .parents(node_info.graph_index)
            .iter(&self.dependency_graph)
            .map(|(_, parent_idx)| self.dependency_graph.node_weight(parent_idx).map(|i| *i))
            .try_collect()
    }

    /// Returns the children of the given node (with their depths) if it exists. If the returned array is
    /// empty, the node is a leaf node.
    pub fn get_children_with_depth(
        &self,
        h: &Header::HeaderId,
    ) -> Option<Vec<(Reverse<u64>, Header::HeaderId)>> {
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

    // pub fn get_children(&self, n:&HeaderId) -> Vec<HeaderId> {
    //     unimplemented!{}
    // }

    pub fn contains(&self, h: &Header::HeaderId) -> bool {
        if let Some(_node_info) = self.node_info_map.get(h) {
            true
        } else {
            false
        }
    }

    pub fn get_header(&self, n: &Header::HeaderId) -> Option<&Header> {
        self.node_info_map.get(n).map(|i| &i.header)
    }

    pub fn get_header_depth(&self, n: &Header::HeaderId) -> Option<u64> {
        self.node_info_map.get(n).map(|i| i.depth)
    }

    pub fn insert_header(&mut self, header: Header) -> bool {
        let header_id = header.get_header_id();

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

            // Update tips since the new node is a leaf.
            self.tips.insert(header_id);

            (vec![], 1)
        } else {
            let mut depth = u64::MAX;
            if let Some(parent_idxs) = parents
                .iter()
                .map(|parent_id| {
                    self.node_info_map.get(&parent_id).map(|i| {
                        depth = cmp::min(depth, i.depth);
                        i.graph_index
                    })
                })
                .try_collect::<Vec<daggy::NodeIndex>>()
            {
                // If any parents were previously a tip, remove from tips.
                parents.iter().for_each(|parent_id| {
                    self.tips.remove(parent_id);
                });
                // Insert as tip since received headers must (currently) be a leaf.
                self.tips.insert(header_id);

                (parent_idxs, depth + 1)
            } else {
                // They sent us a header when we don't know the parent.
                return false;
            }
        };

        // Insert node and store its index in `node_idx_map`.
        // JP: We really want an `add_child` function that takes multiple parents.
        let graph_index = self.dependency_graph.add_node(header_id);
        let node_info = NodeInfo {
            graph_index: graph_index.clone(),
            depth,
            header,
        };
        if let Err(_) = self.node_info_map.try_insert(header_id, node_info) {
            // TODO: Should be unreachable. Log this.
            return false;
        }

        // Insert edges.
        if let Err(_) = self.dependency_graph.add_edges(
            parent_idxs
                .into_iter()
                .map(|parent_idx| (parent_idx, graph_index, ())),
        ) {
            // TODO: Unreachable? Log this.
            return false;
        }

        true
    }

    /// Perform a BFS to check if `ancestor` is an ancestor of `descendent`. Returns `None` if
    /// either header id is not in the graph.
    fn is_ancestor_of(&self, ancestor: &Header::HeaderId, descendent: &Header::HeaderId) -> Option<bool> {
        let anid = self.node_info_map.get(ancestor)?.graph_index;
        let dnid = self.node_info_map.get(descendent)?.graph_index;

        let mut bfs = Bfs::new(&Reversed(&self.dependency_graph), dnid);
        while let Some(nid) = bfs.next(&self.dependency_graph) {
            if nid == anid {
                return Some(true);
            }
        }

        Some(false)
    }
}

/// Tests whether two ecg states have the same DAG.
#[cfg(test)]
pub(crate) fn equal_dags<Header: ECGHeader>(l: &State<Header>, r: &State<Header>) -> bool
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

    let node_set_left: BTreeSet<_> = nodes(&l.dependency_graph);
    let node_set_right = nodes(&r.dependency_graph);
    let edge_set_left: BTreeSet<_> = edges(&l.dependency_graph);
    let edge_set_right = edges(&r.dependency_graph);

    l.root_nodes == r.root_nodes
        && l.tips == r.tips
        && edge_set_left == edge_set_right
        && node_set_left == node_set_right
}

#[cfg(test)]
pub(crate) fn print_dag<Header: ECGHeader>(s: &State<Header>) {
    use petgraph::dot::{Config, Dot};
    use petgraph::stable_graph::StableDiGraph;

    let mut g = s
        .dependency_graph
        .map(|_i, n| format!("{:?}", n), |_i, e| e);

    // Add root node.
    let root = g.add_node("".to_string());
    for n in &s.root_nodes {
        g.add_edge(root, s.node_info_map[n].graph_index, &());
    }

    let g: StableDiGraph<_, _> = g.into();
    let d = Dot::with_config(&g, &[Config::EdgeNoLabel]);
    println!("{:?}", d);
}
