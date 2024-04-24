use std::time::SystemTime;

use crate::time::CausalOrder;

#[derive(Clone, Debug)]
pub struct LamportTimestamp<Id> {
    timestamp: SystemTime,
    id: Id,
}

impl<Id> LamportTimestamp<Id> {
    pub fn current(user: Id) -> Self {
        LamportTimestamp {
            timestamp: SystemTime::now(),
            id: user,
        }
    }
}

impl<Id:Ord> CausalOrder for LamportTimestamp<Id> {
    fn happens_before(t1: &Self, t2: &Self) -> bool {
        if t1.timestamp < t2.timestamp {
            true
        } else if t2.timestamp < t1.timestamp {
            false
        } else {
            t1.id < t2.id
        }
    }
}

