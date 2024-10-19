pub mod lamport;

use std::cmp::Ordering;

pub use lamport::LamportTimestamp;

// TODO: Vector clock
// TODO: hashes from DAG history

// Can't use this since Rust can't handle existentials..
// pub trait CausalOrder {
//     type State;
//
//     // JP: Should this return Option<bool>? Ex, LamportTimestamp doesn't always know the causal ordering.
//     fn happens_before(state: &Self::State, t1: &Self, t2: &Self) -> bool;
// }
pub trait CausalState {
    type Time;

    // JP: Should this return Option<bool>? Ex, LamportTimestamp doesn't always know the causal ordering.
    fn happens_before(&self, t1: &Self::Time, t2: &Self::Time) -> bool;
}

pub fn concurrent<CS: CausalState>(st: &CS, t1: &CS::Time, t2: &CS::Time) -> bool {
    !CausalState::happens_before(st, t1, t2) && !CausalState::happens_before(st, t2, t1)
}

pub fn compare_with_tiebreak<CS: CausalState<Time: Ord>>(
    st: &CS,
    t1: &CS::Time,
    t2: &CS::Time,
) -> Ordering {
    if CausalState::happens_before(st, t1, t2) {
        Ordering::Less
    } else if CausalState::happens_before(st, t2, t1) {
        Ordering::Greater
    } else {
        // For concurrent operations, fall back to total order on time.
        t1.cmp(t2)
    }
}
