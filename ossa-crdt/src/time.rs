pub mod lamport;

use std::cmp::Ordering;

use void::{unreachable, Void};

pub use lamport::LamportTimestamp;

use crate::{map::twopmap::TwoPMapOp, register::LWW};

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

pub trait ConcretizeTime<HeaderId> {
    type Serialized;

    fn concretize_time(src: Self::Serialized, current_header: HeaderId) -> Self;
}

impl<HeaderId> ConcretizeTime<HeaderId> for Void {
    type Serialized = Void;

    fn concretize_time(src: Self::Serialized, _current_header: HeaderId) -> Self {
        unreachable(src)
    }
}

impl<HeaderId, T: ConcretizeTime<HeaderId>, V> ConcretizeTime<HeaderId> for LWW<T, V> {
    type Serialized = LWW<T::Serialized, V>;

    fn concretize_time(src: Self::Serialized, current_header: HeaderId) -> Self {
        LWW {
            time: T::concretize_time(src.time, current_header),
            value: src.value,
        }
    }
}

/*
impl<HeaderId, T: ConcretizeTime<HeaderId>, A> ConcretizeTime<HeaderId> for Const<T, A> {
    type Serialized = Const<T::Serialized, A>;

    fn concretize_time(src: Self::Serialized, current_header: HeaderId) -> Self {
        Self::new(src.into_part())
    }
}
*/

impl<
        HeaderId: Clone,
        K: ConcretizeTime<HeaderId>,
        V: ConcretizeTime<HeaderId>,
        Op: ConcretizeTime<HeaderId>,
    > ConcretizeTime<HeaderId> for TwoPMapOp<K, V, Op>
{
    type Serialized = TwoPMapOp<K::Serialized, V::Serialized, Op::Serialized>;

    fn concretize_time(src: Self::Serialized, current_header: HeaderId) -> Self {
        match src {
            TwoPMapOp::Insert { key, value } => TwoPMapOp::Insert {
                key: K::concretize_time(key, current_header.clone()),
                value: V::concretize_time(value, current_header),
            },
            TwoPMapOp::Apply { key, operation } => TwoPMapOp::Apply {
                key: K::concretize_time(key, current_header.clone()),
                operation: Op::concretize_time(operation, current_header),
            },
            TwoPMapOp::Delete { key } => TwoPMapOp::Delete {
                key: K::concretize_time(key, current_header),
            },
        }
    }
}
