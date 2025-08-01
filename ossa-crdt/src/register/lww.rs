use ossa_typeable::Typeable;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;

use crate::{
    time::{compare_with_tiebreak, CausalState},
    CRDT,
};

// TODO: Define CBOR properly
#[derive(Clone, Debug, PartialEq, Typeable, Serialize, Deserialize)]
/// Last writer wins (LWW) register.
pub struct LWW<T, A> {
    pub time: T,
    pub value: A,
}

impl<T, A> LWW<T, A> {
    pub fn new(time: T, value: A) -> Self {
        LWW { time, value }
    }

    pub fn time(&self) -> &T {
        &self.time
    }

    pub fn value(&self) -> &A {
        &self.value
    }
}

impl<T: Ord, A> CRDT for LWW<T, A> {
    type Op = LWW<T, A>;
    type Time = T;

    fn apply<CS: CausalState<Time = Self::Time>>(self, st: &CS, op: Self::Op) -> Self {
        match compare_with_tiebreak(st, &self.time, &op.time) {
            Ordering::Less => op,
            Ordering::Greater => self,
            Ordering::Equal => unreachable!(
                "Precondition of `apply` violated: Applied `logical_time`s must be unique."
            ),
        }
    }
}

// impl<'a, T, A> Functor<'a, T> for LWW<T, A> {
//     type Target<S> = LWW<S, A>;
//
//     fn fmap<B, F>(self, f: F) -> Self::Target<B>
//     where
//         F: Fn(T) -> B + 'a
//     {
//         LWW {
//             time: f(self.time),
//             value: self.value,
//         }
//     }
// }
