use std::marker::PhantomData;

use ossa_typeable::Typeable;
use serde::{Deserialize, Serialize};
use void::Void;

use crate::CRDT;


/// A CRDT that is constant and cannot be updated.
#[derive(Clone, Debug, Typeable, Serialize, Deserialize)]
pub struct Const<T, A> {
    value: A,
    phantom: PhantomData<fn(T)>,
}

impl<T, A> Const<T, A> {
    pub fn new(value: A) -> Self {
        Const {
            value,
            phantom: PhantomData,
        }
    }

    pub fn value(&self) -> &A {
        &self.value
    }
}

impl<T, A> CRDT for Const<T, A> {
    type Op = Void;

    type Time = T;

    fn apply<CS: crate::time::CausalState<Time = Self::Time>>(self, _causal_state: &CS, _op: Self::Op) -> Self {
        self
    }
}
