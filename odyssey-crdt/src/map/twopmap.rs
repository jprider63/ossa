use im::{OrdMap, OrdSet};
use serde::{Deserialize, Serialize};
use serde::ser::{SerializeStruct, Serializer};
use std::fmt::{self, Debug};
use typeable::Typeable;

use crate::time::CausalState;
use crate::CRDT;

/// Two phase map.
#[derive(Clone, Typeable)]
pub struct TwoPMap<K, V> {
    // JP: Drop `K`?
    map: OrdMap<K, V>,
    tombstones: OrdSet<K>,
}

// TODO: Standardized serialization.
impl<K: Serialize + Ord + Clone, V: Serialize + Clone> Serialize for TwoPMap<K, V> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut s = serializer.serialize_struct("TwoPMap", 2)?;
        s.serialize_field("map", &self.map)?;
        s.serialize_field("tombstones", &self.tombstones)?;
        s.end()
    }
}

impl<K: Ord + Debug, V: Debug> Debug for TwoPMap<K, V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        self.map.fmt(f)
    }
}

// TODO: Define CBOR properly
#[derive(Debug, Serialize, Deserialize)]
pub enum TwoPMapOp<K, V: CRDT> {
    Insert { value: V },
    Apply { key: K, operation: V::Op },
    Delete { key: K },
}

impl<K, V: CRDT> TwoPMapOp<K, V> {
    fn key<'a>(&'a self, op_time: &'a K) -> &K {
        match self {
            TwoPMapOp::Insert { .. } => op_time,
            TwoPMapOp::Apply { key, .. } => key,
            TwoPMapOp::Delete { key } => key,
        }
    }
}

impl<K: Ord + Clone, V: CRDT<Time = K> + Clone> CRDT for TwoPMap<K, V> {
    type Op = TwoPMapOp<K, V>;
    type Time = V::Time; // JP: Newtype wrap `struct TwoPMapId<V>(V::Time)`?

    fn apply<CS: CausalState<Time = Self::Time>>(
        self,
        st: &CS,
        op_time: Self::Time,
        op: Self::Op,
    ) -> Self {
        // Check if deleted.
        let is_deleted = {
            let key = op.key(&op_time);
            self.tombstones.contains(key)
        };
        if is_deleted {
            self
        } else {
            match op {
                TwoPMapOp::Insert { value } => {
                    let TwoPMap { map, tombstones } = self;
                    let map = map.update_with(op_time, value, |_, _| {
                        unreachable!("Invariant violated. Key already exists in TwoPMap.");
                    });

                    TwoPMap { map, tombstones }
                }
                TwoPMapOp::Apply { key, operation } => {
                    let TwoPMap { map, tombstones } = self;
                    let map = map.alter(|v| {
                        if let Some(v) = v {
                            Some(v.apply(st, op_time, operation))
                        } else {
                            unreachable!("Invariant violated. Key must already exist when applyting an update to a TwoPMap.")
                        }
                    }, key);

                    TwoPMap { map, tombstones }
                }
                TwoPMapOp::Delete { key } => {
                    let TwoPMap { map, tombstones } = self;
                    let map = map.without(&key);
                    let tombstones = tombstones.update(key);

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
        TwoPMapOp::Insert { value }
    }
}
