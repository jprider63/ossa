// #![feature(non_lifetime_binders)]

pub mod map;
pub mod register;
pub mod set;
pub mod text;
pub mod time;

use crate::time::CausalState;

// // JP: What should this be called? LogicalOp? MetaOp?
// pub struct AnnotatedOp<M:OpMetadata, Op> {
//     metadata: M,
//     operation: Op,
// }
//
// pub trait OpMetadata {
//     type Time;
//
//     /// Logical time, serving as a unique identifier for this operation.
//     fn time(&self) -> Self::Time;
// }

pub trait CRDT {
    type Op; // <Time>; // Required due to lack of higher kinded types.
    type Time; // TODO: Delete this??

    // TODO: enabled...

    // Mut or return Self?
    // fn apply<'a>(&'a mut self, op: &'a AnnotatedOp<M,Self::Op>); // -> &'a Self;

    // Preconditions:
    // - All `logical_time`s of applied operations must be unique in all subsequent calls to `apply`.
    fn apply<CS: CausalState<Time = Self::Time>>(
        self,
        causal_state: &CS,
        op: Self::Op,
    ) -> Self;

    // lawCommutativity :: concurrent t1 t2 => x.apply(t1, op1).apply(t2, op2) == x.apply(t2, op2).apply(t1, op1)
}

// /// A poor man's functor that is used to modify the times used in CRDT operations.
// /// This is necessary since serialized operations may contain references to the current time but current time should never appear in memory CRDTs.
// /// `Op<T1> -> (T1 -> T2) -> Op<T2>`
// pub trait OperationFunctor<S, T> {
//     type Target<Time>;
// 
//     fn fmap(self, f: impl Fn(S) -> T) -> Self::Target<T>;
// }

pub trait ConcretizeTime<HeaderId> {
    type Serialized;

    fn concretize_time(src: Self::Serialized, header_id: HeaderId) -> Self;
}

// TODO: Need to connect the history causal ordering w/ the operation causal ordering/invariants
// and whether or not an operation is enabled/valid
