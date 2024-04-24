
use crate::{CRDT, time::CausalOrder};

#[derive(Clone, Debug)]
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

    fn apply<'a>(self, op_time: T, op: Self::Op) -> Self {
        if CausalOrder::happens_before(&self.time, &op_time) {
            LWW {
                time: op_time,
                value: op,
            }
        } else if CausalOrder::happens_before(&op_time, &self.time) {
            self
        } else {
            // For concurrent operations, fall back to total order on time.
            if self.time < op_time {
                LWW {
                    time: op_time,
                    value: op,
                }
            } else if op_time < self.time {
                self
            } else {
                unreachable!("Precondition of `apply` violated. Applied `logical_time`s must be unique.")
            }
        }
    }
}
