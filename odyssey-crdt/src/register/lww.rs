use serde::{Deserialize, Serialize};
use std::cmp::Ordering;

use crate::{
    time::{compare_with_tiebreak, CausalState},
    CRDT,
};

// TODO: Define CBOR properly
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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
    type Op = A;
    type Time = T;

    fn apply<CS: CausalState<Time = Self::Time>>(
        self,
        st: &CS,
        op_time: Self::Time,
        op: Self::Op,
    ) -> Self {
        match compare_with_tiebreak(st, &self.time, &op_time) {
            Ordering::Less => LWW {
                time: op_time,
                value: op,
            },
            Ordering::Greater => self,
            Ordering::Equal => unreachable!(
                "Precondition of `apply` violated: Applied `logical_time`s must be unique."
            ),
        }
    }
}
