
pub mod crdt;

pub trait CausalOrder {
    fn happens_before(op1: &Self, op2: &Self) -> bool;
}

pub fn concurrent<Op:CausalOrder>(op1: &Op, op2: &Op) -> bool {
    !CausalOrder::happens_before(op1, op2) && !CausalOrder::happens_before(op2, op1)
}

// TODO: Vector clock
// TODO: hashes from DAG history

pub struct LamportTimestamp<Id> {
    timestamp: u64,
    id: Id,
}

impl<Id> CausalOrder for LamportTimestamp<Id> {
    fn happens_before(_op1: &Self, _op2: &Self) -> bool {
        // We don't know for sure whether or not op1 happens before op2.
        // Technically, we could: op1.id == op2.id && op1 < op2
        false
    }
}

