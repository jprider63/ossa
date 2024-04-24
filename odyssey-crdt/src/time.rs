pub mod lamport;

pub use lamport::LamportTimestamp;

// TODO: Vector clock
// TODO: hashes from DAG history


// JP: Just use PartialOrd?
pub trait CausalOrder {
    fn happens_before(t1: &Self, t2: &Self) -> bool;
}

pub fn concurrent<Op:CausalOrder>(t1: &Op, t2: &Op) -> bool {
    !CausalOrder::happens_before(t1, t2) && !CausalOrder::happens_before(t2, t1)
}










