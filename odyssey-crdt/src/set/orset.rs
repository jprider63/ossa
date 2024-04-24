
use std::borrow::Borrow;
use std::cmp::Ord;
use std::collections::BTreeMap;
use std::collections::BTreeSet;

// use crate::{AnnotatedOp, CRDT, OpMetadata};
// 
// pub struct ORSet<T, A> {
//     set: BTreeMap<A, BTreeSet<T>>,
// }
// 
// pub enum ORSetOp<T, A> {
//     Insert {
//         value: A,
//     },
//     Remove {
//         value: A,
//         tags: BTreeSet<T>,
//     },
// }
// 
// // TODO: CausalOrder for ORSetOp?
// 
// impl<T, A> ORSet<T, A> {
//     pub fn contains<Q>(&self, value:&Q) -> bool
//     where
//         A: Borrow<Q> + Ord,
//         Q: Ord,
//     {
//         self.set.contains_key(value)
//     }
// }
// 
// // TODO: Iter, len, is_subset, is_superset, ...
// 
// impl<M:OpMetadata + OpMetadata<Time = T>, T:Ord, A:Clone + Ord> CRDT<M> for ORSet<T, A> {
//     type Op = ORSetOp<T, A>;
// 
//     fn apply<'a>(&'a mut self, op: &'a AnnotatedOp<M, Self::Op>) {
//         match &op.operation {
//             ORSetOp::Insert {value} => {
//                 // If the value already exists, insert this operation's id/time into its set.
//                 if let Some(tags) = self.set.get_mut(value) {
//                     let inserted = tags.insert(op.metadata.time());
//                     assert!(inserted, "Precondition violated: An operation was applied more than once.");
//                 // Otherwise, insert the value into the map.
//                 } else {
//                     self.set.insert(value.clone(), BTreeSet::from([op.metadata.time()]));
//                 }
//             }
//             ORSetOp::Remove {value, tags} => {
//                 // If the value already exists, remove the specified tags.
//                 if let Some(current_tags) = self.set.get_mut(value) {
//                     // TODO: We should probably check that they all existed in the set before
//                     // deletion.
//                     current_tags.retain(|t| !tags.contains(t));
//                 } else {
//                     panic!("Precondition violated: Attempting to remove an operation that does not exist.");
//                 }
//             }
//         }
//     }
// }
// 
