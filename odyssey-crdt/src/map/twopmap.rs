use im::{
    OrdMap,
    OrdSet,
};

use crate::CRDT;
use crate::time::CausalOrder;

/// Two phase map.
#[derive(Clone)]
pub struct TwoPMap<K, V> { // JP: Drop `K`?
    map: OrdMap<K, V>,
    tombstones: OrdSet<K>,
}

#[derive(Debug)]
pub enum TwoPMapOp<K, V: CRDT> {
    Insert {
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
    fn key<'a>(&'a self, op_time: &'a K) -> &K {
        match self {
            TwoPMapOp::Insert{..} => op_time,
            TwoPMapOp::Apply{key, ..} => key,
            TwoPMapOp::Delete{key} => key,
        }
    }
}

impl<K: Ord + Clone + CausalOrder, V: CRDT<Time = K> + Clone> CRDT for TwoPMap<K, V> {
    type Op = TwoPMapOp<K, V>;
    type Time = V::Time; // JP: Newtype wrap `struct TwoPMapId<V>(V::Time)`?

    fn apply(self, op_time: Self::Time, op: Self::Op) -> Self {
        // Check if deleted.
        let is_deleted = {
            let key = op.key(&op_time);
            self.tombstones.contains(key)
        };
        if is_deleted {
            self
        } else {
            match op {
                TwoPMapOp::Insert{value} => {
                    let TwoPMap {mut map, mut tombstones} = self;
                    map.insert(op_time, value);

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

impl<K: Ord, V: CRDT> TwoPMap<K, V> {
    pub fn new() -> TwoPMap<K, V> {
        TwoPMap {
            map: OrdMap::new(),
            tombstones: OrdSet::new(),
        }
    }

    pub fn get(&self, key: &K) -> Option<&V> {
        self.map.get(key)
    }

    pub fn iter(&self) -> im::ordmap::Iter<'_, K, V> {
        self.map.iter()
    }

    pub fn insert(value: V) -> TwoPMapOp<K, V> {
        TwoPMapOp::Insert{ value }
    }
}
