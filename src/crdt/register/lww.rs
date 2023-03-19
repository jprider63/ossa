
use std::cmp::PartialOrd;

use crate::crdt::CRDT;

#[derive(Clone)]
/// Last writer wins (LWW) register.
pub struct LWW<T, A> {
    time: T,
    value: A,
}

impl<T:PartialOrd, A> CRDT for LWW<T, A> where LWW<T, A>:Clone {
    type Op = LWW<T, A>;

    fn apply<'a>(&'a mut self, op: &'a Self::Op) { // -> &'a Self {
        if op.time > self.time {
            *self = op.clone()
        }
        // &self
    }
}
