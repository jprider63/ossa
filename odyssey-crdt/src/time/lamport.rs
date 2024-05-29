use std::cmp::Ordering::{self, Greater, Less};
use std::time::SystemTime;

use crate::time::CausalOrder;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
        t1.id == t2.id && t1.timestamp < t2.timestamp
    }
}

impl<Id: PartialOrd> PartialOrd for LamportTimestamp<Id> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        if self.id == other.id {
            self.timestamp.partial_cmp(&other.timestamp)
        } else {
            None
        }
    }
}

impl<Id: Ord> Ord for LamportTimestamp<Id> {
    fn cmp(&self, other: &Self) -> Ordering {
        if self.timestamp < other.timestamp {
            Less
        } else if other.timestamp < self.timestamp {
            Greater
        } else {
            self.id.cmp(&other.id)
        }
    }
}

