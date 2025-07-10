
use crate::util::Hash;

/// Binary merkle tree.
/// We aren't concerned about second pre-image attacks since the tree's size and structure are fixed.
#[derive(Debug)]
pub struct MerkleTree<N> {
    // Layers of the tree, from bottom to top. Root node is excluded?
    layers: Vec<Vec<N>>,
}

// // TODO: Closed form solution?
// fn node_count_for_leaf_count(leaf_count: u64) -> u64 {
//     if leaf_count <= 1 {
//         0
//     } else {
//         // let mut sum = leaf_count;
//         leaf_count + node_count_for_leaf_count(leaf_count.div_ceil(2))
//     }
// }

impl<H: Hash> MerkleTree<H> {
    /// Precondition: Leaves must be non-empty.
    fn from_leaves(leaves: Vec<H>) -> MerkleTree<H> {
        if leaves.is_empty() {
            panic!("Precondition violated: Leaves must be non-empty.");
        }

        let mut layers = vec![];
        layers.push(leaves);

        loop {
            let prev_layer = layers.last().unwrap();
            if prev_layer.len() <= 1 {
                break;
            }
            
            let mut layer = vec![];
            for i in 0..(prev_layer.len() / 2) {
                let mut h = H::new();
                let left = prev_layer[2*i];
                H::update(&mut h, left);
                if let Some(right) = prev_layer.get(2*i + 1) {
                    H::update(&mut h, right);
                }
                let h = H::finalize(h);
                layer.push(h);
            }

            layers.push(layer);
        }

        MerkleTree {
            layers
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
        self.layers
            .last()
            .expect("Invariant violated: layers must be non-empty.")
            [0]
    }

    // // Height, excluding root node layer.
    // fn height(&self) -> u64 {
    //     // Compute the ceiling of the log2.
    //     self.leaf_count.next_power_of_two().ilog2() as u64
    // }
}

impl<N> MerkleTree<Option<N>> {
    pub(crate) fn new_with_capacity(leaf_count: u64) -> MerkleTree<Option<N>> {
        todo!();
        // let nodes = todo!(); // TODO
        // Self {
        //     leaf_count,
        //     nodes,
        // }
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
        let chunks = vec![];
        let root = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        merkle_test_helper(chunks, root);

    }

    fn merkle_test_helper(chunks: Vec<&[u8]>, expected_root: &str) {
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
    */
}
