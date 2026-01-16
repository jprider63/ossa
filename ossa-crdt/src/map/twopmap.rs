use im::{OrdMap, OrdSet};
use ossa_typeable::Typeable;
use serde::de::{MapAccess, Visitor};
use serde::ser::{SerializeStruct, Serializer};
use serde::{Deserialize, Serialize};
use std::fmt::{self, Debug};
use std::marker::PhantomData;

use crate::time::CausalState;
use crate::CRDT;

/// Two phase map.
/// Invariant: All keys must be unique.
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

impl<'d, K: Clone + Ord + Deserialize<'d>, V: Clone + Deserialize<'d>> Deserialize<'d>
    for TwoPMap<K, V>
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'d>,
    {
        struct SVisitor<K, V>(PhantomData<(K, V)>);

        #[derive(Deserialize)]
        #[serde(field_identifier, rename_all = "lowercase")]
        enum Field {
            Map,
            Tombstones,
        }

        impl<'d, K: Ord + Clone + Deserialize<'d>, V: Clone + Deserialize<'d>> Visitor<'d>
            for SVisitor<K, V>
        {
            type Value = TwoPMap<K, V>;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("struct TwoPMap")
            }

            fn visit_map<M>(self, mut m: M) -> Result<TwoPMap<K, V>, M::Error>
            where
                M: MapAccess<'d>,
            {
                let mut map = None;
                let mut tombstones = None;
                while let Some(key) = m.next_key()? {
                    match key {
                        Field::Map => {
                            if map.is_some() {
                                return Err(serde::de::Error::duplicate_field("map"));
                            }
                            map = Some(m.next_value()?);
                        }
                        Field::Tombstones => {
                            if tombstones.is_some() {
                                return Err(serde::de::Error::duplicate_field("tombstones"));
                            }
                            tombstones = Some(m.next_value()?);
                        }
                    }
                }

                let map = map.ok_or_else(|| serde::de::Error::missing_field("map"))?;
                let tombstones =
                    tombstones.ok_or_else(|| serde::de::Error::missing_field("tombstones"))?;

                Ok(TwoPMap { map, tombstones })
            }
        }

        deserializer.deserialize_struct("TwoPMap", &["map", "tombstones"], SVisitor(PhantomData))
    }
}

impl<K: Ord + Debug, V: Debug> Debug for TwoPMap<K, V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        self.map.fmt(f)
    }
}

// TODO: Define CBOR properly
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum TwoPMapOp<K, V, Op> {
    Insert { key: K, value: V },
    Apply { key: K, operation: Op },
    Delete { key: K },
}

impl<K, V, Op> TwoPMapOp<K, V, Op> {
    fn key(&self) -> &K {
        match self {
            TwoPMapOp::Insert { key, .. } => key,
            TwoPMapOp::Apply { key, .. } => key,
            TwoPMapOp::Delete { key } => key,
        }
    }
}

impl<K: Ord + Clone, V: CRDT<Time = K> + Clone> CRDT for TwoPMap<K, V> {
    type Op = TwoPMapOp<K, V, V::Op>;
    type Time = V::Time; // JP: Newtype wrap `struct TwoPMapId<V>(V::Time)`?

    fn apply<CS: CausalState<Time = Self::Time>>(self, st: &CS, op: Self::Op) -> Self {
        // Check if deleted.
        let is_deleted = {
            let key = op.key();
            self.tombstones.contains(key)
        };
        if is_deleted {
            self
        } else {
            match op {
                TwoPMapOp::Insert { key, value } => {
                    let TwoPMap { map, tombstones } = self;
                    let map = map.update_with(key, value, |_, _| {
                        unreachable!("Invariant violated. Key already exists in TwoPMap.");
                    });

                    TwoPMap { map, tombstones }
                }
                TwoPMapOp::Apply { key, operation } => {
                    let TwoPMap { map, tombstones } = self;
                    let map = map.alter(|v| {
                        if let Some(v) = v {
                            Some(v.apply(st, operation))
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

    pub fn insert(key: K, value: V) -> TwoPMapOp<K, V, V::Op> {
        TwoPMapOp::Insert { key, value }
    }
}

// impl<'a, T, V: CRDT> Functor<'a, T> for TwoPMapOp<T, V>
// where
//     V::Op<T>: for<S> Functor<'a, T, Target<S> = V::Op<S>>,
// {
//     type Target<S> = TwoPMapOp<S, V>;
//
//     fn fmap<B, F>(self, f: F) -> Self::Target<B>
//     where
//         F: Fn(T) -> B + 'a
//     {
//         match self {
//             TwoPMapOp::Insert { key, value } => {
//                 TwoPMapOp::Insert {: CRDT
//                     key: f(key),
//                     value,
//                 }
//             }
//             TwoPMapOp::Apply { key, operation } => {
//                 let operation: V::Op<B> = operation.fmap::<B, _>(f);
//                 TwoPMapOp::Apply {
//                     key: f(key),
//                     operation,
//                 }
//             }
//             TwoPMapOp::Delete { key } => {
//                 TwoPMapOp::Delete { key: f(key) }
//             }
//         }
//     }
// }

// impl<K, L, V, Op> OperationFunctor<K, L> for TwoPMapOp<K, V, Op>
// where
//     Op: OperationFunctor<K, L, Target<L> = Op>,
// {
//     type Target<Time> = TwoPMapOp<Time, V, Op>;
//
//     fn fmap(self, f: impl Fn(K) -> L) -> Self::Target<L> {
//         match self {
//             TwoPMapOp::Insert { key, value } => {
//                 TwoPMapOp::Insert {
//                     key: f(key),
//                     value,
//                 }
//             }
//             TwoPMapOp::Apply { key, operation } => {
//                 let key = f(key);
//                 let operation = operation.fmap(f);
//                 TwoPMapOp::Apply {
//                     key,
//                     operation,
//                 }
//             }
//             TwoPMapOp::Delete { key } => {
//                 TwoPMapOp::Delete { key: f(key) }
//             }
//         }
//     }
// }

#[cfg(test)]
mod tests {
    use super::*;
    use crate::register::LWW;
    use crate::time::CausalState;
    use proptest::prelude::*;

    struct SimpleCausalState;
    impl CausalState for SimpleCausalState {
        type Time = u64;
        fn happens_before(&self, t1: &Self::Time, t2: &Self::Time) -> bool {
            t1 < t2
        }
    }

    #[test]
    fn test_twopmap_new() {
        let map: TwoPMap<u64, LWW<u64, i32>> = TwoPMap::new();
        assert_eq!(map.get(&1), None);
    }

    #[test]
    fn test_twopmap_insert() {
        let st = SimpleCausalState;
        let map: TwoPMap<u64, LWW<u64, i32>> = TwoPMap::new();

        let op = TwoPMapOp::Insert {
            key: 1,
            value: LWW::new(1, 42),
        };

        let map = map.apply(&st, op);
        assert_eq!(map.get(&1).map(|v| v.value()), Some(&42));
    }

    #[test]
    fn test_twopmap_insert_multiple() {
        let st = SimpleCausalState;
        let map: TwoPMap<u64, LWW<u64, i32>> = TwoPMap::new();

        let op1 = TwoPMapOp::Insert {
            key: 1,
            value: LWW::new(1, 42),
        };
        let op2 = TwoPMapOp::Insert {
            key: 2,
            value: LWW::new(2, 100),
        };

        let map = map.apply(&st, op1).apply(&st, op2);

        assert_eq!(map.get(&1).map(|v| v.value()), Some(&42));
        assert_eq!(map.get(&2).map(|v| v.value()), Some(&100));
    }

    #[test]
    fn test_twopmap_apply_nested() {
        let st = SimpleCausalState;
        let map: TwoPMap<u64, LWW<u64, i32>> = TwoPMap::new();

        // First insert a value
        let insert_op = TwoPMapOp::Insert {
            key: 1,
            value: LWW::new(1, 42),
        };
        let map = map.apply(&st, insert_op);

        // Then apply a nested update
        let update_op = TwoPMapOp::Apply {
            key: 1,
            operation: LWW::new(2, 100),
        };
        let map = map.apply(&st, update_op);

        assert_eq!(map.get(&1).map(|v| v.value()), Some(&100));
    }

    #[test]
    fn test_twopmap_delete() {
        let st = SimpleCausalState;
        let map: TwoPMap<u64, LWW<u64, i32>> = TwoPMap::new();

        let insert_op = TwoPMapOp::Insert {
            key: 1,
            value: LWW::new(1, 42),
        };
        let map = map.apply(&st, insert_op);
        assert_eq!(map.get(&1).map(|v| v.value()), Some(&42));

        let delete_op = TwoPMapOp::Delete { key: 1 };
        let map = map.apply(&st, delete_op);

        // After deletion, key should not be in the map
        assert_eq!(map.get(&1), None);
    }

    #[test]
    fn test_twopmap_delete_is_permanent() {
        let st = SimpleCausalState;
        let map: TwoPMap<u64, LWW<u64, i32>> = TwoPMap::new();

        let insert_op = TwoPMapOp::Insert {
            key: 1,
            value: LWW::new(1, 42),
        };
        let map = map.apply(&st, insert_op);

        let delete_op = TwoPMapOp::Delete { key: 1 };
        let map = map.apply(&st, delete_op);

        // Try to insert again - should be ignored due to tombstone
        let reinsert_op = TwoPMapOp::Insert {
            key: 1,
            value: LWW::new(2, 100),
        };
        let map = map.apply(&st, reinsert_op);

        assert_eq!(map.get(&1), None);
    }

    #[test]
    fn test_twopmap_update_after_delete_ignored() {
        let st = SimpleCausalState;
        let map: TwoPMap<u64, LWW<u64, i32>> = TwoPMap::new();

        let insert_op = TwoPMapOp::Insert {
            key: 1,
            value: LWW::new(1, 42),
        };
        let map = map.apply(&st, insert_op);

        let delete_op = TwoPMapOp::Delete { key: 1 };
        let map = map.apply(&st, delete_op);

        // Try to apply an update - should be ignored
        let update_op = TwoPMapOp::Apply {
            key: 1,
            operation: LWW::new(2, 100),
        };
        let map = map.apply(&st, update_op);

        assert_eq!(map.get(&1), None);
    }

    #[test]
    fn test_twopmap_convergence_insert_order() {
        let st = SimpleCausalState;
        let map: TwoPMap<u64, LWW<u64, i32>> = TwoPMap::new();

        let op1 = TwoPMapOp::Insert {
            key: 1,
            value: LWW::new(1, 10),
        };
        let op2 = TwoPMapOp::Insert {
            key: 2,
            value: LWW::new(2, 20),
        };

        // Apply in different orders
        let result1 = map.clone().apply(&st, op1.clone()).apply(&st, op2.clone());
        let result2 = map.clone().apply(&st, op2.clone()).apply(&st, op1.clone());

        // Should converge to same state
        assert_eq!(result1.get(&1).map(|v| v.value()), result2.get(&1).map(|v| v.value()));
        assert_eq!(result1.get(&2).map(|v| v.value()), result2.get(&2).map(|v| v.value()));
    }

    #[test]
    fn test_twopmap_convergence_with_deletes() {
        let st = SimpleCausalState;
        let map: TwoPMap<u64, LWW<u64, i32>> = TwoPMap::new();

        let insert_op = TwoPMapOp::Insert {
            key: 1,
            value: LWW::new(1, 42),
        };
        let update_op = TwoPMapOp::Apply {
            key: 1,
            operation: LWW::new(2, 100),
        };
        let delete_op = TwoPMapOp::Delete { key: 1 };

        // Replica A: insert, update, delete
        let replica_a = map.clone()
            .apply(&st, insert_op.clone())
            .apply(&st, update_op.clone())
            .apply(&st, delete_op.clone());

        // Replica B: insert, delete, update (update should be ignored)
        let replica_b = map.clone()
            .apply(&st, insert_op.clone())
            .apply(&st, delete_op.clone())
            .apply(&st, update_op.clone());

        // Both should converge to deleted state
        assert_eq!(replica_a.get(&1), None);
        assert_eq!(replica_b.get(&1), None);
    }

    #[test]
    fn test_twopmap_iter() {
        let st = SimpleCausalState;
        let map: TwoPMap<u64, LWW<u64, i32>> = TwoPMap::new();

        let map = map
            .apply(&st, TwoPMapOp::Insert { key: 1, value: LWW::new(1, 10) })
            .apply(&st, TwoPMapOp::Insert { key: 2, value: LWW::new(2, 20) })
            .apply(&st, TwoPMapOp::Insert { key: 3, value: LWW::new(3, 30) });

        let entries: Vec<_> = map.iter().map(|(k, v)| (*k, *v.value())).collect();
        assert_eq!(entries.len(), 3);
        assert!(entries.contains(&(1, 10)));
        assert!(entries.contains(&(2, 20)));
        assert!(entries.contains(&(3, 30)));
    }

    proptest! {
        #[test]
        fn prop_twopmap_insert_retrievable(
            key in 0u64..100,
            time in 0u64..1000,
            value in any::<i32>(),
        ) {
            let st = SimpleCausalState;
            let map: TwoPMap<u64, LWW<u64, i32>> = TwoPMap::new();

            let op = TwoPMapOp::Insert {
                key,
                value: LWW::new(time, value),
            };

            let map = map.apply(&st, op);
            prop_assert_eq!(map.get(&key).map(|v| v.value()), Some(&value));
        }

        #[test]
        fn prop_twopmap_convergence_any_order(
            k1 in 0u64..10,
            k2 in 10u64..20,
            k3 in 20u64..30,
            v1 in any::<i32>(),
            v2 in any::<i32>(),
            v3 in any::<i32>(),
        ) {
            let st = SimpleCausalState;
            let map: TwoPMap<u64, LWW<u64, i32>> = TwoPMap::new();

            let op1 = TwoPMapOp::Insert { key: k1, value: LWW::new(k1, v1) };
            let op2 = TwoPMapOp::Insert { key: k2, value: LWW::new(k2, v2) };
            let op3 = TwoPMapOp::Insert { key: k3, value: LWW::new(k3, v3) };

            // Apply in different orders
            let result_123 = map.clone()
                .apply(&st, op1.clone())
                .apply(&st, op2.clone())
                .apply(&st, op3.clone());

            let result_321 = map.clone()
                .apply(&st, op3.clone())
                .apply(&st, op2.clone())
                .apply(&st, op1.clone());

            // Should converge
            prop_assert_eq!(
                result_123.get(&k1).map(|v| v.value()),
                result_321.get(&k1).map(|v| v.value())
            );
            prop_assert_eq!(
                result_123.get(&k2).map(|v| v.value()),
                result_321.get(&k2).map(|v| v.value())
            );
            prop_assert_eq!(
                result_123.get(&k3).map(|v| v.value()),
                result_321.get(&k3).map(|v| v.value())
            );
        }

        #[test]
        fn prop_twopmap_delete_always_wins(
            key in 0u64..100,
            insert_time in 0u64..100,
            insert_value in any::<i32>(),
        ) {
            let st = SimpleCausalState;
            let map: TwoPMap<u64, LWW<u64, i32>> = TwoPMap::new();

            let insert_op = TwoPMapOp::Insert {
                key,
                value: LWW::new(insert_time, insert_value),
            };
            let delete_op = TwoPMapOp::Delete { key };

            // Delete before insert
            let result1 = map.clone()
                .apply(&st, delete_op.clone())
                .apply(&st, insert_op.clone());

            // Delete after insert
            let result2 = map.clone()
                .apply(&st, insert_op.clone())
                .apply(&st, delete_op.clone());

            // Both should result in deletion
            prop_assert_eq!(result1.get(&key), None);
            prop_assert_eq!(result2.get(&key), None);
        }
    }
}
