use odyssey_crdt::{map::twopmap::TwoPMapOp, register::LWW};
use serde::{Deserialize, Serialize};

#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum CausalTime<Time> {
    Current { operation_position: u8 }, // Points to the current ECG node.
    Time(Time),                         // Points to another ECG node.
}

impl<Time> CausalTime<Time> {
    pub fn current_time(operation_position: u8) -> CausalTime<Time> {
        CausalTime::Current { operation_position }
    }

    pub fn time(time: Time) -> CausalTime<Time> {
        CausalTime::Time(time)
    }
}

pub trait ConcretizeTime<HeaderId> {
    type Serialized;

    fn concretize_time(src: Self::Serialized, current_header: HeaderId) -> Self;
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
