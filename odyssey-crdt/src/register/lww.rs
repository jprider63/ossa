use std::cmp::Ordering;

use crate::{
    CRDT,
    time::{CausalOrder, compare_with_tiebreak},
};

#[derive(Clone, Debug, PartialEq)]
/// Last writer wins (LWW) register.
pub struct LWW<T, A> {
    time: T,
    value: A,
}

impl<T, A> LWW<T, A> {
    pub fn new(time: T, value: A) -> Self {
        LWW {
            time,
            value,
        }
    }

    pub fn time(&self) -> &T {
        &self.time
    }

    pub fn value(&self) -> &A {
        &self.value
    }
}

impl<T:CausalOrder + Ord, A> CRDT for LWW<T, A> {
    type Op = A;
    type Time = T;

    fn apply(self, op_time: T, op: Self::Op) -> Self {
        match compare_with_tiebreak(&self.time, &op_time) {
            Ordering::Less =>
                LWW {
                    time: op_time,
                    value: op,
                },
            Ordering::Greater =>
                self,
            Ordering::Equal =>
                unreachable!("Precondition of `apply` violated: Applied `logical_time`s must be unique."),
        }
    }
}
