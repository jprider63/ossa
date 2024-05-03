use im::{
    OrdMap,
    OrdSet,
};

use crate::CRDT;

/// Two phase map.
pub struct TwoPMap<K, V> {
    map: OrdMap<K, V>,
    tombstones: OrdSet<K>,
}

pub enum TwoPMapOp<K, V: CRDT> {
    Insert {
        key: K,
        value: V,
    },
    Apply {
        key: K,
        operation: V::Op,
    },
    Delete {
        key: K,
    },
}

impl<K, V: CRDT> TwoPMapOp<K, V> {
    fn key(&self) -> &K {
        match self {
            TwoPMapOp::Insert{key, ..} => key,
            TwoPMapOp::Apply{key, ..} => key,
            TwoPMapOp::Delete{key} => key,
        }
    }
}

impl<K: Ord + Clone, V: CRDT + Clone> CRDT for TwoPMap<K, V> {
    type Op = TwoPMapOp<K, V>;
    type Time = V::Time;

    fn apply(self, op_time: Self::Time, op: Self::Op) -> Self {
        let key = op.key();

        // Check if deleted.
        if self.tombstones.contains(key) {
            self
        } else {
            match op {
                TwoPMapOp::Insert{key, value} => {
                    let TwoPMap {mut map, mut tombstones} = self;
                    map.insert(key, value);

                    TwoPMap { map, tombstones }
                }
                TwoPMapOp::Apply{key, operation} => {
                    let TwoPMap {mut map, mut tombstones} = self;
                    map.alter(|v| v.map(|v| v.apply(op_time, operation)), key);

                    TwoPMap { map, tombstones }
                }
                TwoPMapOp::Delete{key} => {
                    let TwoPMap {mut map, mut tombstones} = self;
                    map.remove(&key);
                    tombstones.insert(key);

                    TwoPMap { map, tombstones }
                }
            }
        }
    }
}

