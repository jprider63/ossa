use im::Vector;
use std::cmp::Ordering;

use crate::{
    time::{compare_with_tiebreak, CausalState},
    CRDT,
};

#[derive(Clone)]
pub struct CausalTree<T, A> {
    atom: Atom<T, A>,
    children: Vector<CausalTree<T, A>>, // JP: Use a Map, ordered by atom, here instead??
}

pub struct CausalTreeOp<T, A> {
    parent_id: T,
    // atom: Atom<T, A>,
    letter: Letter<A>,
}

#[derive(Clone)]
struct Atom<T, A> {
    id: T,
    letter: Letter<A>,
}

#[derive(Clone)]
enum Letter<A> {
    Letter(A),
    Delete,
    /// Root node. Should only be used as the initial node on creation.
    Root,
}

impl<T: Eq + Ord + Clone, A: Clone> CRDT for CausalTree<T, A> {
    type Op = CausalTreeOp<T, A>;
    type Time = T;

    fn apply<CS: CausalState<Time = Self::Time>>(self, st: &CS, op_time: T, op: Self::Op) -> Self {
        insert_in_weave(st, &self, &op_time, &op)
            .expect("Precondition of `apply` violated: Operation must only be applied when all of its parents have been applied.")
    }
}

fn insert_in_weave<T: Eq + Ord + Clone, A: Clone, CS: CausalState<Time = T>>(
    st: &CS,
    weave: &CausalTree<T, A>,
    op_time: &T,
    op: &CausalTreeOp<T, A>,
) -> Option<CausalTree<T, A>> {
    if weave.atom.id == op.parent_id {
        let atom = Atom {
            id: op_time.clone(),
            letter: op.letter.clone(),
        };
        let children = insert_atom(st, weave.children.clone(), atom);
        let ct = CausalTree {
            atom: weave.atom.clone(),
            children,
        };
        Some(ct)
    } else {
        let children_m = insert_in_weave_children(st, weave.children.clone(), op_time, op);
        children_m.map(|children| CausalTree {
            atom: weave.atom.clone(),
            children,
        })
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
    mut children: Vector<CausalTree<T, A>>,
    op_time: &T,
    op: &CausalTreeOp<T, A>,
) -> Option<Vector<CausalTree<T, A>>> {
    // JP: Why does iter require clone?
    for mut child in children.iter_mut() {
        if let Some(updated_child) = insert_in_weave(st, child, op_time, op) {
            *child = updated_child;
            return Some(children);
        }
    }

    None
}
