
use std::collections::BTreeMap;
use std::collections::BTreeSet;

use crate::crdt::CRDT;

pub struct ORSet<T, A> {
    set: BTreeMap<A, BTreeSet<T>>,
}

pub enum ORSetOp<T, A> {
    Insert {
        value: A,
    },
    Remove {
        value: A,
        tags: BTreeSet<T>,
    },
}

// TODO: CausalOrder for ORSetOp?

// impl<T, A> CRDT for ORSet<T, A> {
// }

