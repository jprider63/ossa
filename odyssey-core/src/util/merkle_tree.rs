
/// Binary merkle tree.
pub struct MerkleTree<N> {
    leaf_count: u64, // Don't serialize this.
    // Flattened nodes of the merkle tree. Bottom nodes first, from left to right. Empty nodes are excluded. Root node is excluded?
    nodes: Vec<Vec<N>>,
}

// TODO: Closed form solution?
fn node_count_for_leaf_count(leaf_count: u64) -> u64 {
    if leaf_count <= 1 {
        0
    } else {
        // let mut sum = leaf_count;
        leaf_count + node_count_for_leaf_count(leaf_count.div_ceil(2))
    }
}

impl<Hash> MerkleTree<Hash> {
    pub(crate) fn new_with_leaves(leaves: Vec<Hash>) -> MerkleTree<Hash> {
        let leaf_count = leaves.len() as u64;
        let mut nodes = leaves;
        let mut position = 0;
        // JP: Probably not worth doing this.
        // nodes.reserve_exact(node_count_for_leaf_count(leaf_count) as usize - nodes.len()); //

        let mut prev_layer_count = leaf_count;
        let mut layer_count = prev_layer_count.div_ceil(2);

        for i in 0..layer_count {

        }


        MerkleTree {
            leaf_count,
            nodes,
        }
    }

    // // Height, excluding root node layer.
    // fn height(&self) -> u64 {
    //     // Compute the ceiling of the log2.
    //     self.leaf_count.next_power_of_two().ilog2() as u64
    // }
}

impl<N> MerkleTree<Option<N>> {
    pub(crate) fn new_with_capacity(leaf_count: u64) -> MerkleTree<Option<N>> {
        let nodes = todo!();
        Self {
            leaf_count,
            nodes,
        }
    }

    /*
    pub(crate) fn missing_indices<'a>(&'a self) -> impl Iterator<Item = u64> + 'a {
        self.nodes
            .iter()
            .enumerate()
            .filter_map(|h| {
                if h.1.is_none() {
                    Some(h.0 as u64)
                } else {
                    None
                }
            })
    }
    */
}

#[cfg(test)]
mod test {
    use super::*;

    /*
    #[test]
    fn height_tests() {
        let pairs = [
            (0, 0),
            (1, 0),
            (2, 1),
            (3, 2),
            (4, 2),
            (5, 3),
            (6, 3),
            (7, 3),
            (8, 3),
            (9, 4),
            (10, 4),
        ];

        for (count, height) in pairs {
            let t = MerkleTree::<Option<()>>::new_with_capacity(count);
            assert_eq!(t.height(), height);
        }
    }
    */

    #[test]
    fn node_count_for_leaf_count_tests() {
        let pairs = [
            (0, 0),
            (1, 0),
            (2, 2),
            (3, 5),
            (4, 6),
            (5, 10),
            (6, 11),
            (7, 13),
            (8, 14),
            (9, 19),
            (10, 20),
        ];
        for (input, expected_output) in pairs {
            let res = node_count_for_leaf_count(input);
            assert_eq!(expected_output, res);
        }
    }
}
