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
