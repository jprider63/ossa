pub mod register;
pub mod set;

// JP: What should this be called? LogicalOp? MetaOp?
pub struct AnnotatedOp<M:OpMetadata, Op> {
    metadata: M,
    operation: Op,
}

pub trait OpMetadata {
    type Time;

    fn time(&self) -> Self::Time;
}

pub trait CRDT<M:OpMetadata> {
    type Op;

    // TODO: enabled...

    // Mut or return Self?
    fn apply<'a>(&'a mut self, op: &'a AnnotatedOp<M,Self::Op>); // -> &'a Self;

    // lawCommutativity :: concurrent op1.time() op2.time() => x.apply(op1).apply(op2) == x.apply(op2).apply(op1)
}





pub trait CausalOrder {
    fn happens_before(op1: &Self, op2: &Self) -> bool;
}

pub fn concurrent<Op:CausalOrder>(op1: &Op, op2: &Op) -> bool {
    !CausalOrder::happens_before(op1, op2) && !CausalOrder::happens_before(op2, op1)
}









// TODO: Put this somewhere else?


// TODO: Vector clock
// TODO: hashes from DAG history
// 
// pub struct LamportTimestamp<Id> {
//     timestamp: u64,
//     id: Id,
// }
// 
// impl<Id> CausalOrder for LamportTimestamp<Id> {
//     fn happens_before(_op1: &Self, _op2: &Self) -> bool {
//         // We don't know for sure whether or not op1 happens before op2.
//         // Technically, we could: op1.id == op2.id && op1 < op2
//         false
//     }
// }

