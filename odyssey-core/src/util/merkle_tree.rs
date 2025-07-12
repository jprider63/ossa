use std::{cmp, fmt::Debug};

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

impl<H> MerkleTree<H> {
    /// Precondition: Leaves must be non-empty.
    fn from_leaves(leaves: Vec<H>) -> MerkleTree<H>
    where
        H: Hash + Debug,
    {
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
            for i_64 in start..end {
                let i: usize = i_64.try_into().expect("TODO");

                #[cfg(test)]
                assert_eq!(nodes[i], None);

                let h = if is_leaf(leaf_count, i_64) {
                    leaves.next().unwrap()
                } else {
                    let left = nodes[2 * i + 1].unwrap();
                    let right = nodes[2 * i + 2].unwrap();

                    let mut h = H::new();
                    H::update(&mut h, left);
                    H::update(&mut h, right);
                    H::finalize(h)
                };
                nodes[i] = Some(h);
            }
        }

        #[cfg(test)]
        assert_eq!(leaves.next(), None);

        let Some(nodes) = nodes.into_iter().collect() else {
            unreachable!("Failed to initialize MerkleTree");
        };

        MerkleTree { nodes }
    }

    pub fn from_chunks<I: Iterator<Item = A>, A: AsRef<[u8]>>(chunks: I) -> MerkleTree<H>
    where
        H: Hash + Debug,
    {
        let leaves: Vec<H> = chunks
            .map(|c| {
                let mut h = H::new();
                H::update(&mut h, c);
                H::finalize(h)
            })
            .collect();

        if leaves.is_empty() {
            let empty: [&[u8]; 1] = [&[]];
            return MerkleTree::from_chunks(empty.iter());
        }

        MerkleTree::from_leaves(leaves)
    }

    pub fn merkle_root(&self) -> H
    where
        H: Copy,
    {
        self.nodes[0]
    }

    /*
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
    */

    /// Gets the node at the given index.
    pub fn get(&self, index: u64) -> Option<&H> {
        let index: usize = index.try_into().expect("TODO");
        self.nodes.get(index)
    }

    fn leaf_count(&self) -> u64 {
        (self.nodes.len() as u64 + 1) / 2
    }

    // Precondition: leaf_index < leaf_count
    pub(crate) fn validate_chunk<A: AsRef<[u8]>>(&self, leaf_index: u64, chunk: A) -> bool
    where
        H: Hash,
    {
        let index = self.index_for_leaf(leaf_index);
        let index: usize = index.try_into().unwrap();
        let expected_hash = self.nodes[index];

        let mut h = H::new();
        H::update(&mut h, chunk);
        let h = H::finalize(h);

        expected_hash == h
    }

    fn index_for_leaf(&self, leaf_index: u64) -> u64 {
        let leaf_count = self.leaf_count();
        assert!(leaf_index < leaf_count);

        let bottom_layer = leaf_count.next_power_of_two().ilog2() as u64;
        let start = (1 << bottom_layer) - 1; // 2^layer - 1;
        let mut index = start + leaf_index;
        let nodes_count = self.nodes.len() as u64;
        if index >= nodes_count {
            index -= leaf_count;
        }
        index
    }
}

impl<N> MerkleTree<Potential<N>> {
    pub(crate) fn new_with_capacity(merkle_root: N, leaf_count: u64) -> MerkleTree<Potential<N>> {
        let mut nodes: Vec<_> = (0..node_count_for_leaf_count(leaf_count))
            .map(|_| Potential::None)
            .collect();
        nodes[0] = Potential::Verified(merkle_root);

        Self { nodes }
    }

    pub(crate) fn missing_indices<'a>(&'a self) -> impl Iterator<Item = u64> + 'a {
        self.nodes.iter().enumerate().filter_map(|h| {
            if h.1.is_none() {
                Some(h.0 as u64)
            } else {
                None
            }
        })
    }

    /// Sets the potential value for the node in the merkle tree. Returns false if the proposed value is invalid (Could result in false positives if the sibling node was sent by another peer).
    pub(crate) fn set(&mut self, index: u64, potential_value: N) -> bool
    where
        N: Hash,
    {
        let index: usize = index.try_into().unwrap();
        let Some(node) = self.nodes.get_mut(index) else {
            return false;
        };
        match node {
            Potential::None => {
                *node = Potential::Unverified(potential_value);
            }
            Potential::Unverified(_prev) => {
                // Keep the old value, but don't penalize them either way.
                return true;
            }
            Potential::Verified(prev) => {
                return prev == &potential_value;
            }
        }

        if index == 0 {
            unreachable!("We always initialize the root so we will have already returned.");
        }
        let parent_index = (index - 1) / 2;
        match self.validate_children(parent_index as u64) {
            Some(v) => v,
            None => true,
        }
    }

    // Precondition: Child nodes must not be verified.
    fn validate_children(&mut self, index: u64) -> Option<bool>
    where
        N: Hash,
    {
        if is_leaf(self.leaf_count(), index) {
            return None;
        }

        let index: usize = index.try_into().unwrap();
        let Potential::Verified(expected_hash) = self.nodes[index] else {
            return None;
        };
        let left_index = 2 * index + 1;
        let right_index = 2 * index + 2;
        let (left, right) = match (&self.nodes[left_index], &self.nodes[right_index]) {
            (Potential::None, _) => return None,
            (_, Potential::None) => return None,
            (Potential::Unverified(left), Potential::Unverified(right)) => (*left, *right),
            _ => unreachable!("Invariant: Sibling nodes must both be verified. Precondition: Child nodes must not be verified.")
        };
        let is_valid = {
            let mut h = N::new();
            N::update(&mut h, left);
            N::update(&mut h, right);
            expected_hash == N::finalize(h)
        };

        if is_valid {
            self.nodes[left_index] = Potential::Verified(left);
            self.nodes[right_index] = Potential::Verified(right);

            // Recursively check children if valid.
            self.validate_children(left_index as u64);
            self.validate_children(right_index as u64);

            Some(true)
        } else {
            self.nodes[left_index] = Potential::None;
            self.nodes[right_index] = Potential::None;

            Some(false)
        }
    }

    /// Tries to complete the merkle tree by checking if all nodes have been verified.
    pub(crate) fn try_complete(&self) -> Option<MerkleTree<N>>
    where
        N: Copy,
    {
        let nodes = self
            .nodes
            .iter()
            .map(|n| match n {
                Potential::Verified(v) => Some(*v),
                _ => None,
            })
            .collect::<Option<Vec<_>>>()?;

        Some(MerkleTree { nodes })
    }
}

#[derive(Debug)]
pub enum Potential<T> {
    None,
    Unverified(T), // Track who sent it?
    Verified(T),
}

impl<T> Potential<T> {
    fn is_none(&self) -> bool {
        if let Potential::None = self {
            true
        } else {
            false
        }
    }
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

        let chunks = vec![b"0"];
        let root = "5FECEB66FFC86F38D952786C6D696C79C2DBC239DD4E91B46729D73A27FB57E9";
        merkle_test_helper(chunks, root);

        let chunks = vec![b"0", b"1"];
        let root = "B9B10A1BC77D2A241D120324DB7F3B81B2EDB67EB8E9CF02AF9C95D30329AEF5";
        merkle_test_helper(chunks, root);

        let chunks = vec![b"0", b"1", b"2"];
        let root = "C80F77387D860FA469920D7AC2F8A959EF83A651F76DC54923734ED76DAAEF53";
        merkle_test_helper(chunks, root);

        let chunks = vec![b"0", b"1", b"2", b"3"];
        let root = "C478FEAD0C89B79540638F844C8819D9A4281763AF9272C7F3968776B6052345";
        merkle_test_helper(chunks, root);

        let chunks = vec![b"0", b"1", b"2", b"3", b"4"];
        let root = "C9CF4B52254B6397C6027A5DBD97CE2CF0F9340651E421C4184100AF7248EA92";
        merkle_test_helper(chunks, root);
    }

    fn merkle_test_helper<A: AsRef<[u8]> + Debug>(chunks: Vec<A>, expected_root: &str) {
        let expected_root = Sha256Hash::from_str(expected_root).unwrap();

        let mt = MerkleTree::<Sha256Hash>::from_chunks(chunks.iter());
        println!("{mt:?}");
        let root = mt.merkle_root();

        assert_eq!(root, expected_root);

        for (i, chunk) in chunks.iter().enumerate() {
            assert!(
                mt.validate_chunk(i as u64, chunk),
                "Chunk validation failed for chunk ({i}): {chunk:?}"
            );
        }
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
