
use crate::{AnnotatedOp, CRDT, OpMetadata};

#[derive(Clone)]
/// Last writer wins (LWW) register.
pub struct LWW<T, A> {
    time: T,
    value: A,
}

impl<M:OpMetadata + OpMetadata<Time = T>, T:Ord, A:Clone> CRDT<M> for LWW<T, A> {
    type Op = A;

    fn apply<'a>(&'a mut self, op: &'a AnnotatedOp<M, Self::Op>) {
        let op_time = op.metadata.time();
        if op_time > self.time {
            *self = LWW {
                time: op_time,
                value: op.operation.clone(),
            }
        }
    }
}
