
use std::{
    cmp,
    fmt::Debug,
};

use crate::util::Hash;

/// Binary merkle tree.
/// Second pre-image attacks aren't possible since the tree's size and shape are fixed. An attacker cannot swap a leaf with a branch node.
#[derive(Debug)]
pub struct MerkleTree<N> {
    // Flattened BFS representation of merkle tree. Root is at index 0.
    nodes: Vec<N>,
}

// fn prev_power_of_two(n: u64) -> u64 {
//     // Handling the case of 0 input:
//     if n == 0 {
//         0
//     } else {
//         // Mask by the highest bit.
//         let highest_bit_set_idx = 63 - n.leading_zeros();
//         (1 << highest_bit_set_idx) & n
//     }
// }

fn node_count_for_leaf_count(leaf_count: u64) -> u64 {
    2 * leaf_count - 1
}
// // TODO: Closed form solution?
//     if leaf_count <= 1 {
//         leaf_count
//     } else {
//         let pow2 = prev_power_of_two(leaf_count);
//         let remaining = leaf_count - pow2;
//         2 * pow2 - 1 + node_count_for_leaf_count(remaining)
//     }

fn is_leaf(leaf_count: u64, index: u64) -> bool {
    index >= leaf_count - 1
}

impl<H: Hash + Debug> MerkleTree<H> {
    /// Precondition: Leaves must be non-empty.
    fn from_leaves(leaves: Vec<H>) -> MerkleTree<H> {
        if leaves.is_empty() {
            panic!("Precondition violated: Leaves must be non-empty.");
        }

        let leaf_count = leaves.len() as u64;
        let capacity = node_count_for_leaf_count(leaf_count);
        let mut nodes: Vec<Option<_>> = (0..capacity).map(|_| None).collect();

        // Build the tree from the bottom up.
        let mut leaves = leaves.into_iter();
        let bottom_layer = leaf_count.next_power_of_two().ilog2() as u64 + 1;
        for layer in (0..bottom_layer).rev() {
            let start = (1 << layer) - 1; // 2^layer - 1;
            let end = cmp::min((1 << (layer + 1)) - 1, capacity);
            for i in start..end {
                let i = i as usize;

                #[cfg(test)]
                assert_eq!(nodes[i], None);

                let h = if is_leaf(leaf_count, i as u64) {
                    leaves.next().unwrap()
                } else {
                    let left = nodes[2*i + 1].unwrap();
                    let right = nodes[2*i + 2].unwrap();

                    let mut h = H::new();
                    H::update(&mut h, left);
                    H::update(&mut h, right);
                    H::finalize(h)
                };
                nodes[i] = Some(h);
            }
        }

        let Some(nodes) = nodes.into_iter().collect() else {
            unreachable!("Failed to initialize MerkleTree");
        };

        MerkleTree {
            nodes,
        }
    }

    pub fn from_chunks<I: Iterator<Item = A>, A: AsRef<[u8]>>(chunks: I) -> MerkleTree<H> {
        let leaves: Vec<H> = chunks.map(|c| {
            let mut h = H::new();
            H::update(&mut h, c);
            H::finalize(h)
        }).collect();

        if leaves.is_empty() {
            let empty: [&[u8]; 1] = [&[]];
            return MerkleTree::from_chunks(empty.iter());
        }

        MerkleTree::from_leaves(leaves)
    }

    pub fn merkle_root(&self) -> H {
        self.nodes[0]
    }

    // If the index corresponds to a branch node, return the child indices. Otherwise, if the index
    // corresponds to a leaf node, return `None`.
    fn child_indices(&self, index: u64) -> Option<(u64, u64)> {
        let left = 2*index + 1;
        let right = 2*index + 2;

        if right < self.nodes.len() as u64 {
            Some((left, right))
        } else {
            None
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
        let nodes = (0..node_count_for_leaf_count(leaf_count)).map(|_| None).collect();
        Self {
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
    use std::str::FromStr;

    use super::*;

    use crate::util::Sha256Hash;

    #[test]
    fn merkle_test() {
        let chunks: Vec<&[u8]> = vec![];
        let root = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        merkle_test_helper(chunks, root);

        let chunks: Vec<&[u8]> = vec![b""];
        let root = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        merkle_test_helper(chunks, root);

        let chunks = vec![b"hello", b"world"];
        let root = "7305DB9B2ABCCD706C256DB3D97E5FF48D677CFE4D3A5904AFB7DA0E3950E1E2";
        merkle_test_helper(chunks, root);
    }

    fn merkle_test_helper<A: AsRef<[u8]>>(chunks: Vec<A>, expected_root: &str) {
        let expected_root = Sha256Hash::from_str(expected_root).unwrap();

        let mt = MerkleTree::<Sha256Hash>::from_chunks(chunks.iter());
        println!("{mt:?}");
        let root = mt.merkle_root();

        assert_eq!(root, expected_root);
    }

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

    #[test]
    fn node_count_for_leaf_count_tests() {
        let pairs = [
            (0, 0),
            (1, 1),
            (2, 3),
            (3, 4),
            (4, 7),
            (5, 8),
            (6, 10),
            (7, 11),
            (8, 15),
            (9, 16),
            (10, 18),
        ];
        for (input, expected_output) in pairs {
            let res = node_count_for_leaf_count(input);
            assert_eq!(expected_output, res);
        }
    }
    */
}
