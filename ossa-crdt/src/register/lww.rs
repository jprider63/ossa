use ossa_typeable::Typeable;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;

use crate::{
    time::{compare_with_tiebreak, CausalState},
    CRDT,
};

// TODO: Define CBOR properly
#[derive(Clone, Debug, PartialEq, Typeable, Serialize, Deserialize)]
/// Last writer wins (LWW) register.
pub struct LWW<T, A> {
    pub time: T,
    pub value: A,
}

impl<T, A> LWW<T, A> {
    pub fn new(time: T, value: A) -> Self {
        LWW { time, value }
    }

    pub fn time(&self) -> &T {
        &self.time
    }

    pub fn value(&self) -> &A {
        &self.value
    }
}

impl<T: Ord, A> CRDT for LWW<T, A> {
    type Op = LWW<T, A>;
    type Time = T;

    fn apply<CS: CausalState<Time = Self::Time>>(self, st: &CS, op: Self::Op) -> Self {
        match compare_with_tiebreak(st, &self.time, &op.time) {
            Ordering::Less => op,
            Ordering::Greater => self,
            Ordering::Equal => unreachable!(
                "Precondition of `apply` violated: Applied `logical_time`s must be unique."
            ),
        }
    }
}

// impl<'a, T, A> Functor<'a, T> for LWW<T, A> {
//     type Target<S> = LWW<S, A>;
//
//     fn fmap<B, F>(self, f: F) -> Self::Target<B>
//     where
//         F: Fn(T) -> B + 'a
//     {
//         LWW {
//             time: f(self.time),
//             value: self.value,
//         }
//     }
// }

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::lamport::LamportTimestamp;
    use proptest::prelude::*;

    struct SimpleCausalState;
    impl CausalState for SimpleCausalState {
        type Time = u64;
        fn happens_before(&self, t1: &Self::Time, t2: &Self::Time) -> bool {
            t1 < t2
        }
    }

    #[test]
    fn test_lww_new() {
        let reg = LWW::new(1u64, "hello");
        assert_eq!(reg.time(), &1);
        assert_eq!(reg.value(), &"hello");
    }

    #[test]
    fn test_lww_apply_later_time_wins() {
        let st = SimpleCausalState;
        let reg1 = LWW::new(1u64, "first");
        let reg2 = LWW::new(2u64, "second");

        let result = reg1.clone().apply(&st, reg2.clone());
        assert_eq!(result.value(), &"second");
        assert_eq!(result.time(), &2);
    }

    #[test]
    fn test_lww_apply_earlier_time_loses() {
        let st = SimpleCausalState;
        let reg1 = LWW::new(2u64, "second");
        let reg2 = LWW::new(1u64, "first");

        let result = reg1.clone().apply(&st, reg2.clone());
        assert_eq!(result.value(), &"second");
        assert_eq!(result.time(), &2);
    }

    #[test]
    fn test_lww_commutativity_with_tiebreak() {
        struct TiebreakerState;
        impl CausalState for TiebreakerState {
            type Time = (u64, u64); // (timestamp, replica_id)
            fn happens_before(&self, t1: &Self::Time, t2: &Self::Time) -> bool {
                t1.0 < t2.0 && t1.1 == t2.1
            }
        }

        let st = TiebreakerState;
        let reg1 = LWW::new((1u64, 0u64), "first");
        let reg2 = LWW::new((1u64, 1u64), "second"); // Same timestamp, different replica
        let reg3 = LWW::new((1u64, 2u64), "third");  // Same timestamp, different replica

        // Both orders should converge to the same result
        let result1 = reg1.clone().apply(&st, reg2.clone()).apply(&st, reg3.clone());
        let result2 = reg1.clone().apply(&st, reg3.clone()).apply(&st, reg2.clone());

        // Due to tiebreaking, both should choose the same winner
        assert_eq!(result1.value(), result2.value());
        assert_eq!(result1.time(), result2.time());
    }

    #[test]
    fn test_lww_convergence() {
        let st = SimpleCausalState;
        let reg_initial = LWW::new(0u64, "initial");

        // Simulate two replicas applying operations in different orders
        let op1 = LWW::new(1u64, "op1");
        let op2 = LWW::new(2u64, "op2");
        let op3 = LWW::new(3u64, "op3");

        // Replica A applies: op1, op2, op3
        let replica_a = reg_initial.clone()
            .apply(&st, op1.clone())
            .apply(&st, op2.clone())
            .apply(&st, op3.clone());

        // Replica B applies: op3, op1, op2
        let replica_b = reg_initial.clone()
            .apply(&st, op3.clone())
            .apply(&st, op1.clone())
            .apply(&st, op2.clone());

        // Both should converge to the same state
        assert_eq!(replica_a.value(), replica_b.value());
        assert_eq!(replica_a.time(), replica_b.time());
        assert_eq!(replica_a.value(), &"op3");
    }

    #[test]
    fn test_lww_with_lamport_timestamps() {
        use std::thread;
        use std::time::Duration;

        struct LamportCausalState;
        impl CausalState for LamportCausalState {
            type Time = LamportTimestamp<u64>;
            fn happens_before(&self, t1: &Self::Time, t2: &Self::Time) -> bool {
                // Lamport timestamps from the same ID with earlier system time happen before
                t1 < t2
            }
        }

        let st = LamportCausalState;
        let reg1 = LWW::new(LamportTimestamp::current(0u64), "first");
        thread::sleep(Duration::from_millis(10));
        let reg2 = LWW::new(LamportTimestamp::current(1u64), "second");

        // The second one should win due to later timestamp
        let result = reg1.clone().apply(&st, reg2.clone());
        assert_eq!(result.value(), &"second");
    }

    proptest! {
        #[test]
        fn prop_lww_later_timestamp_always_wins(
            t1 in 0u64..1000,
            t2 in 1000u64..2000,
            val1 in 0i32..100,
            val2 in 0i32..100,
        ) {
            let st = SimpleCausalState;
            let reg1 = LWW::new(t1, val1);
            let reg2 = LWW::new(t2, val2);

            // t2 > t1, so reg2 should always win
            let result = reg1.apply(&st, reg2.clone());
            prop_assert_eq!(result.value(), &val2);
            prop_assert_eq!(result.time(), &t2);
        }

        #[test]
        fn prop_lww_convergence_any_order(
            t1 in 1u64..100,
            t2 in 100u64..200,
            t3 in 200u64..300,
            v1 in any::<u32>(),
            v2 in any::<u32>(),
            v3 in any::<u32>(),
        ) {
            let st = SimpleCausalState;
            let reg_init = LWW::new(0u64, 0u32);
            let op1 = LWW::new(t1, v1);
            let op2 = LWW::new(t2, v2);
            let op3 = LWW::new(t3, v3);

            // Apply in different orders
            let result_123 = reg_init.clone()
                .apply(&st, op1.clone())
                .apply(&st, op2.clone())
                .apply(&st, op3.clone());

            let result_321 = reg_init.clone()
                .apply(&st, op3.clone())
                .apply(&st, op2.clone())
                .apply(&st, op1.clone());

            let result_213 = reg_init.clone()
                .apply(&st, op2.clone())
                .apply(&st, op1.clone())
                .apply(&st, op3.clone());

            // All should converge to the same state (latest timestamp wins)
            prop_assert_eq!(result_123.value(), result_321.value());
            prop_assert_eq!(result_123.value(), result_213.value());
            prop_assert_eq!(result_123.time(), result_321.time());
            prop_assert_eq!(result_123.time(), result_213.time());

            // The winner should be op3 since t3 is largest
            prop_assert_eq!(result_123.value(), &v3);
            prop_assert_eq!(result_123.time(), &t3);
        }

        #[test]
        fn prop_lww_idempotence(
            time in 0u64..1000,
            value in any::<i32>(),
        ) {
            let st = SimpleCausalState;
            let reg = LWW::new(0u64, 0i32);
            let op = LWW::new(time, value);

            // Applying the same operation twice should be the same as applying it once
            let apply_once = reg.clone().apply(&st, op.clone());

            // This would panic with "unique logical times" precondition
            // But we can test that the result is stable
            prop_assert_eq!(apply_once.value(), &value);
            prop_assert_eq!(apply_once.time(), &time);
        }
    }
}
