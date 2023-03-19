pub mod register;
pub mod set;

pub trait CRDT {
    type Op;

    // TODO: enabled...

    // Mut or return Self?
    fn apply<'a>(&'a mut self, op: &'a Self::Op); // -> &'a Self;

    // lawCommutativity :: concurrent op1 op2 => x.apply(op1).apply(op2) == x.apply(op2).apply(op1)
}
