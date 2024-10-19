use std::cmp::Ordering::{self, Greater, Less};
use std::marker::PhantomData;
use std::time::SystemTime;

use crate::time::CausalState;

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

struct LamportState<Id>(PhantomData<Id>);

impl<Id: Ord> CausalState for LamportState<Id> {
    type Time = LamportTimestamp<Id>;

    fn happens_before(&self, t1: &Self::Time, t2: &Self::Time) -> bool {
        t1.id == t2.id && t1.timestamp < t2.timestamp
    }
}

// impl<Id:Ord> CausalOrder for LamportTimestamp<Id> {
//     type State = ();
//
//     fn happens_before(_: &(), t1: &Self, t2: &Self) -> bool {
//         t1.id == t2.id && t1.timestamp < t2.timestamp
//     }
// }

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
