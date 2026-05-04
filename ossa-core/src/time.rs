pub use ossa_crdt::time::ConcretizeTime;
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

