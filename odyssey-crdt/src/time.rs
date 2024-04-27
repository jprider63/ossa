pub mod lamport;

use std::cmp::Ordering;

pub use lamport::LamportTimestamp;

// TODO: Vector clock
// TODO: hashes from DAG history


// JP: Just use PartialOrd?
pub trait CausalOrder {
    // JP: Should this return Option<bool>? Ex, LamportTimestamp doesn't always know the causal ordering.
    fn happens_before(t1: &Self, t2: &Self) -> bool;
}

pub fn concurrent<Op:CausalOrder>(t1: &Op, t2: &Op) -> bool {
    !CausalOrder::happens_before(t1, t2) && !CausalOrder::happens_before(t2, t1)
}

pub fn compare_with_tiebreak<T:CausalOrder + Ord>(t1: &T, t2: &T) -> Ordering {
    if CausalOrder::happens_before(t1, t2) {
        Ordering::Less
    } else if CausalOrder::happens_before(t2, t1) {
        Ordering::Greater
    } else {
        // For concurrent operations, fall back to total order on time.
        t1.cmp(t2)
    }
}
