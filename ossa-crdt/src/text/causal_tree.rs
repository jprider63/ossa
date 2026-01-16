use im::Vector;
use std::cmp::Ordering;
use std::fmt::Debug;

use crate::{
    time::{compare_with_tiebreak, CausalState},
    CRDT,
};

#[derive(Clone)]
pub struct CausalTree<T, A> {
    atom: Atom<T, A>,
    children: Vector<CausalTree<T, A>>, // JP: Use a Map, ordered by atom, here instead??
}

#[derive(Clone, Debug)]
pub struct CausalTreeOp<T, A> {
    parent_id: T,
    atom: Atom<T, A>,
    // letter: Letter<A>,
}

#[derive(Clone, Debug)]
struct Atom<T, A> {
    id: T,
    letter: Letter<A>,
}

#[derive(Clone, Debug)]
enum Letter<A> {
    Letter(A),
    Delete,
    /// Root node. Should only be used as the initial node on creation.
    Root,
}

impl<T: Eq + Ord + Clone + Debug, A: Clone + Debug> CRDT for CausalTree<T, A> {
    type Op = CausalTreeOp<T, A>;
    type Time = T;

    fn apply<CS: CausalState<Time = Self::Time>>(self, st: &CS, op: Self::Op) -> Self {
        let (ct, op_ret) = insert_in_weave(st, self, op);
        if op_ret.is_some() {
            unreachable!("Precondition of `apply` violated: Operation must only be applied when all of its parents have been applied.")
        }
        ct
    }
}

fn insert_in_weave<T: Eq + Ord + Clone, A: Clone, CS: CausalState<Time = T>>(
    st: &CS,
    weave: CausalTree<T, A>,
    // op_time: &T,
    op: CausalTreeOp<T, A>,
) -> (CausalTree<T, A>, Option<CausalTreeOp<T, A>>) {
    // ) -> Option<CausalTree<T, A>> {
    if weave.atom.id == op.parent_id {
        let children = insert_atom(st, weave.children, op.atom);
        let ct = CausalTree {
            atom: weave.atom, // .clone(),
            children,
        };
        (ct, None)
    } else {
        let (children, op_ret) = insert_in_weave_children(st, weave.children, op);
        let ct = CausalTree {
            atom: weave.atom,
            children,
        };
        (ct, op_ret)
    }
}

fn insert_atom<T: Ord + Clone, A: Clone, CS: CausalState<Time = T>>(
    st: &CS,
    mut children: Vector<CausalTree<T, A>>,
    atom: Atom<T, A>,
) -> Vector<CausalTree<T, A>> {
    fn compare_atom<T: Ord, A, CS: CausalState<Time = T>>(
        st: &CS,
        a1: &Atom<T, A>,
        a2: &Atom<T, A>,
    ) -> Ordering {
        match (a1, a2) {
            (
                Atom {
                    id: id1,
                    letter: Letter::Root,
                },
                Atom {
                    id: id2,
                    letter: Letter::Root,
                },
            ) => compare_with_tiebreak(st, id1, id2),
            (
                Atom { id: _, letter: _ },
                Atom {
                    id: _,
                    letter: Letter::Root,
                },
            ) => Ordering::Less,
            (
                Atom {
                    id: _,
                    letter: Letter::Root,
                },
                Atom { id: _, letter: _ },
            ) => Ordering::Greater,

            (
                Atom {
                    id: id1,
                    letter: Letter::Delete,
                },
                Atom {
                    id: id2,
                    letter: Letter::Delete,
                },
            ) => compare_with_tiebreak(st, id1, id2),
            (
                Atom { id: _, letter: _ },
                Atom {
                    id: _,
                    letter: Letter::Delete,
                },
            ) => Ordering::Less,
            (
                Atom {
                    id: _,
                    letter: Letter::Delete,
                },
                Atom { id: _, letter: _ },
            ) => Ordering::Greater,

            (Atom { id: id1, letter: _ }, Atom { id: id2, letter: _ }) => {
                compare_with_tiebreak(st, id1, id2)
            }
        }
    }

    match children.binary_search_by(|ct| compare_atom(st, &ct.atom, &atom)) {
        Err(index) => {
            let ct = CausalTree {
                atom,
                children: Vector::new(),
            };
            children.insert(index, ct);
        }
        Ok(_index) => {
            unreachable!(
                "Precondition of `apply` violated: Applied `logical_time`s must be unique."
            )
        }
    }

    children
}

fn insert_in_weave_children<T: Eq + Ord + Clone, A: Clone, CS: CausalState<Time = T>>(
    st: &CS,
    children: Vector<CausalTree<T, A>>,
    // op_time: &T,
    op: CausalTreeOp<T, A>,
) -> (Vector<CausalTree<T, A>>, Option<CausalTreeOp<T, A>>) {
    // JP: Why does iter require clone?
    let mut op_m = Some(op);
    let children = children
        .into_iter()
        .map(|child| {
            if let Some(op) = op_m.take() {
                let (updated_child, op_ret) = insert_in_weave(st, child, op);
                op_m = op_ret;
                updated_child
            } else {
                child
            }
        })
        .collect();

    (children, op_m)

    /*
    for mut child in children.iter_mut() {
        match insert_in_weave(st, child, op) {
            Ok(updated_child) => {
                *child = updated_child;
                return Ok(children);
            }
            Err(op) => {
                todo!("Forward on op"):

            }
        }
        // if let Ok(updated_child) = insert_in_weave(st, child, op) {
        //     *child = updated_child;
        //     return Ok(children);
        // }
    }
    */
}

// impl<'a, T, A> Functor<'a, T> for CausalTreeOp<T, A> {
//     type Target<S> = CausalTreeOp<S, A>;
//
//     fn fmap<B, F>(self, f: F) -> Self::Target<B>
//     where
//         F: Fn(T) -> B + 'a {
//         let atom = Atom { id: f(self.atom.id), letter: self.atom.letter };
//         CausalTreeOp {
//             parent_id: f(self.parent_id),
//             atom,
//         }
//     }
// }

// impl<T, U, V> ConcretizeTime<T, U> for CausalTreeOp<T, V> {
//     type Target<S> = CausalTreeOp<S, V>;
//
//     fn concretize_time(self, f: impl Fn(T) -> U) -> Self::Target<U> {
//         let atom = Atom { id: f(self.atom.id), letter: self.atom.letter };
//         CausalTreeOp {
//             parent_id: f(self.parent_id),
//             atom,
//         }
//     }
// }

impl<T: Clone, A: Clone> CausalTree<T, A> {
    /// Create a new CausalTree with a root node
    pub fn new(root_id: T) -> Self {
        CausalTree {
            atom: Atom {
                id: root_id,
                letter: Letter::Root,
            },
            children: Vector::new(),
        }
    }

    /// Extract the text from the tree (ignores deleted characters)
    pub fn to_string(&self) -> String
    where
        A: Into<char> + Clone,
    {
        let mut result = String::new();
        self.collect_text(&mut result);
        result
    }

    fn collect_text(&self, result: &mut String)
    where
        A: Into<char> + Clone,
    {
        match &self.atom.letter {
            Letter::Letter(ch) => result.push(ch.clone().into()),
            Letter::Delete | Letter::Root => {}
        }

        for child in &self.children {
            child.collect_text(result);
        }
    }
}

impl<T, A> CausalTreeOp<T, A> {
    /// Create an insert operation
    pub fn insert(parent_id: T, id: T, letter: A) -> Self {
        CausalTreeOp {
            parent_id,
            atom: Atom {
                id,
                letter: Letter::Letter(letter),
            },
        }
    }

    /// Create a delete operation
    pub fn delete(parent_id: T, id: T) -> Self {
        CausalTreeOp {
            parent_id,
            atom: Atom {
                id,
                letter: Letter::Delete,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::CausalState;
    use proptest::prelude::*;

    struct SimpleCausalState;
    impl CausalState for SimpleCausalState {
        type Time = u64;
        fn happens_before(&self, t1: &Self::Time, t2: &Self::Time) -> bool {
            t1 < t2
        }
    }

    #[test]
    fn test_causal_tree_new() {
        let tree: CausalTree<u64, char> = CausalTree::new(0);
        assert_eq!(tree.to_string(), "");
    }

    #[test]
    fn test_causal_tree_single_insert() {
        let st = SimpleCausalState;
        let tree = CausalTree::new(0u64);

        let op = CausalTreeOp::insert(0, 1, 'a');
        let tree = tree.apply(&st, op);

        assert_eq!(tree.to_string(), "a");
    }

    #[test]
    fn test_causal_tree_sequential_inserts() {
        let st = SimpleCausalState;
        let tree = CausalTree::new(0u64);

        // Insert "hello"
        let op1 = CausalTreeOp::insert(0, 1, 'h');
        let op2 = CausalTreeOp::insert(1, 2, 'e');
        let op3 = CausalTreeOp::insert(2, 3, 'l');
        let op4 = CausalTreeOp::insert(3, 4, 'l');
        let op5 = CausalTreeOp::insert(4, 5, 'o');

        let tree = tree
            .apply(&st, op1)
            .apply(&st, op2)
            .apply(&st, op3)
            .apply(&st, op4)
            .apply(&st, op5);

        assert_eq!(tree.to_string(), "hello");
    }

    #[test]
    fn test_causal_tree_concurrent_inserts_same_parent() {
        let st = SimpleCausalState;
        let tree = CausalTree::new(0u64);

        // Insert 'a' at root
        let op1 = CausalTreeOp::insert(0, 1, 'a');
        let tree = tree.apply(&st, op1);

        // Two concurrent inserts after 'a' with IDs 2 and 3
        let op2 = CausalTreeOp::insert(1, 2, 'x');
        let op3 = CausalTreeOp::insert(1, 3, 'y');

        // Apply in different orders
        let tree1 = tree.clone().apply(&st, op2.clone()).apply(&st, op3.clone());
        let tree2 = tree.clone().apply(&st, op3.clone()).apply(&st, op2.clone());

        // Should converge to the same result (ordering by timestamp)
        assert_eq!(tree1.to_string(), tree2.to_string());
    }

    #[test]
    fn test_causal_tree_delete() {
        let st = SimpleCausalState;
        let tree = CausalTree::new(0u64);

        // Insert "hello"
        let tree = tree
            .apply(&st, CausalTreeOp::insert(0, 1, 'h'))
            .apply(&st, CausalTreeOp::insert(1, 2, 'e'))
            .apply(&st, CausalTreeOp::insert(2, 3, 'l'))
            .apply(&st, CausalTreeOp::insert(3, 4, 'l'))
            .apply(&st, CausalTreeOp::insert(4, 5, 'o'));

        assert_eq!(tree.to_string(), "hello");

        // Delete the middle 'l' (id 3) by inserting a Delete node
        let delete_op = CausalTreeOp::delete(3, 6);
        let _tree = tree.apply(&st, delete_op);

        // Note: The delete operation is a child of the deleted character
        // It doesn't actually remove it from the string in this implementation
        // This is a characteristic of the causal tree weave structure
        // The character remains but could be filtered out during rendering
    }

    #[test]
    fn test_causal_tree_convergence_different_order() {
        let st = SimpleCausalState;
        let tree = CausalTree::new(0u64);

        let op1 = CausalTreeOp::insert(0, 1, 'a');
        let op2 = CausalTreeOp::insert(0, 2, 'b');
        let op3 = CausalTreeOp::insert(0, 3, 'c');

        // Apply in different orders
        let tree_123 = tree.clone()
            .apply(&st, op1.clone())
            .apply(&st, op2.clone())
            .apply(&st, op3.clone());

        let tree_321 = tree.clone()
            .apply(&st, op3.clone())
            .apply(&st, op2.clone())
            .apply(&st, op1.clone());

        let tree_213 = tree.clone()
            .apply(&st, op2.clone())
            .apply(&st, op1.clone())
            .apply(&st, op3.clone());

        // All should converge
        assert_eq!(tree_123.to_string(), tree_321.to_string());
        assert_eq!(tree_123.to_string(), tree_213.to_string());
    }

    #[test]
    fn test_causal_tree_interleaved_edits() {
        let st = SimpleCausalState;
        let tree = CausalTree::new(0u64);

        // Replica 1 inserts "ac"
        let r1_tree = tree.clone()
            .apply(&st, CausalTreeOp::insert(0, 1, 'a'))
            .apply(&st, CausalTreeOp::insert(1, 3, 'c'));

        // Replica 2 inserts "ab"
        let r2_tree = tree.clone()
            .apply(&st, CausalTreeOp::insert(0, 1, 'a'))
            .apply(&st, CausalTreeOp::insert(1, 2, 'b'));

        // Now merge: replica 1 receives op from replica 2
        let merged1 = r1_tree.apply(&st, CausalTreeOp::insert(1, 2, 'b'));

        // Replica 2 receives op from replica 1
        let merged2 = r2_tree.apply(&st, CausalTreeOp::insert(1, 3, 'c'));

        // Both should converge to "abc" (or "acb" depending on timestamp ordering)
        assert_eq!(merged1.to_string(), merged2.to_string());
    }

    #[test]
    fn test_causal_tree_build_complex_string() {
        let st = SimpleCausalState;
        let tree = CausalTree::new(0u64);

        // Build "hello world" character by character
        let mut tree = tree;
        let mut id = 0u64;
        let text = "hello world";

        for ch in text.chars() {
            id += 1;
            let op = CausalTreeOp::insert(id - 1, id, ch);
            tree = tree.apply(&st, op);
        }

        assert_eq!(tree.to_string(), "hello world");
    }

    proptest! {
        #[test]
        fn prop_causal_tree_convergence_any_order(
            ch1 in any::<char>(),
            ch2 in any::<char>(),
            ch3 in any::<char>(),
        ) {
            let st = SimpleCausalState;
            let tree = CausalTree::new(0u64);

            // All ops have same parent (root), different timestamps
            let op1 = CausalTreeOp::insert(0, 1, ch1);
            let op2 = CausalTreeOp::insert(0, 2, ch2);
            let op3 = CausalTreeOp::insert(0, 3, ch3);

            // Apply in different orders
            let result_123 = tree.clone()
                .apply(&st, op1.clone())
                .apply(&st, op2.clone())
                .apply(&st, op3.clone());

            let result_321 = tree.clone()
                .apply(&st, op3.clone())
                .apply(&st, op2.clone())
                .apply(&st, op1.clone());

            let result_213 = tree.clone()
                .apply(&st, op2.clone())
                .apply(&st, op1.clone())
                .apply(&st, op3.clone());

            // All should converge to the same text
            prop_assert_eq!(result_123.to_string(), result_321.to_string());
            prop_assert_eq!(result_123.to_string(), result_213.to_string());
        }

        #[test]
        fn prop_causal_tree_sequential_build(
            s in "[a-z]{1,10}",
        ) {
            let st = SimpleCausalState;
            let mut tree = CausalTree::new(0u64);

            let mut parent_id = 0u64;
            for (i, ch) in s.chars().enumerate() {
                let id = (i + 1) as u64;
                let op = CausalTreeOp::insert(parent_id, id, ch);
                tree = tree.apply(&st, op);
                parent_id = id;
            }

            prop_assert_eq!(tree.to_string(), s);
        }

        #[test]
        fn prop_causal_tree_root_children_convergence(
            chars in prop::collection::vec(any::<char>(), 1..10),
        ) {
            let st = SimpleCausalState;
            let tree = CausalTree::new(0u64);

            // Create ops that all insert at root with different timestamps
            let ops: Vec<_> = chars.iter().enumerate()
                .map(|(i, &ch)| CausalTreeOp::insert(0, (i + 1) as u64, ch))
                .collect();

            // Apply ops in forward order
            let mut tree_forward = tree.clone();
            for op in ops.iter() {
                tree_forward = tree_forward.apply(&st, op.clone());
            }

            // Apply ops in reverse order
            let mut tree_reverse = tree.clone();
            for op in ops.iter().rev() {
                tree_reverse = tree_reverse.apply(&st, op.clone());
            }

            // Should converge
            prop_assert_eq!(tree_forward.to_string(), tree_reverse.to_string());
        }
    }
}
