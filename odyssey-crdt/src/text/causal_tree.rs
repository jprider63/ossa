use im::Vector;
use std::cmp::Ordering;
use std::fmt::Debug;

use crate::{
    time::{compare_with_tiebreak, CausalState}, OperationFunctor, CRDT
};

#[derive(Clone)]
pub struct CausalTree<T, A> {
    atom: Atom<T, A>,
    children: Vector<CausalTree<T, A>>, // JP: Use a Map, ordered by atom, here instead??
}

#[derive(Debug)]
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
    type Op<Time> = CausalTreeOp<Time, A>;
    type Time = T;

    fn apply<CS: CausalState<Time = Self::Time>>(self, st: &CS, op: Self::Op<T>) -> Self {
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
    let children = children.into_iter().map(|child| {
        if let Some(op) = op_m.take() {
            let (updated_child, op_ret) = insert_in_weave(st, child, op);
            op_m = op_ret;
            updated_child
        } else {
            child
        }
    }).collect();

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

impl<T, U, V> OperationFunctor<T, U> for CausalTreeOp<T, V> {
    type Target<S> = CausalTreeOp<S, V>;

    fn fmap(self, f: impl Fn(T) -> U) -> Self::Target<U> {
        let atom = Atom { id: f(self.atom.id), letter: self.atom.letter };
        CausalTreeOp {
            parent_id: f(self.parent_id),
            atom,
        }
    }
}
