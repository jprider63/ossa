use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use typeable::Typeable;

use crate::{
    time::{compare_with_tiebreak, CausalState}, OperationFunctor, CRDT
};

// TODO: Define CBOR properly
#[derive(Clone, Debug, PartialEq, Typeable, Serialize, Deserialize)]
/// Last writer wins (LWW) register.
pub struct LWW<T, A> {
    time: T,
    value: A,
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
    type Op<Time> = LWW<Time, A>;
    type Time = T;

    fn apply<CS: CausalState<Time = Self::Time>>(
        self,
        st: &CS,
        op: Self::Op<T>,
    ) -> Self {
        match compare_with_tiebreak(st, &self.time, &op.time) {
            Ordering::Less => op,
            Ordering::Greater => self,
            Ordering::Equal => unreachable!(
                "Precondition of `apply` violated: Applied `logical_time`s must be unique."
            ),
        }
    }
}

impl<T: Ord, A> OperationFunctor for LWW<T, A> {
    fn fmap<T1, T2>(op: <Self as CRDT>::Op<T1>, f: impl Fn(T1) -> T2) -> <Self as CRDT>::Op<T2> {
        LWW {
            time: f(op.time),
            value: op.value,
        }
    }
}
