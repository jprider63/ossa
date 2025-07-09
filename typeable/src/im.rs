use crate::{TypeId, Typeable};
use im::{OrdMap, OrdSet};
use sha2::{Digest, Sha256};

use crate::internal::{helper_type_args_count, helper_type_constructor, helper_type_ident};

impl<K: Typeable, V: Typeable> Typeable for OrdMap<K, V> {
    fn type_ident() -> TypeId {
        let mut h = Sha256::new();
        helper_type_constructor(&mut h, "im__OrdMap");
        helper_type_args_count(&mut h, 2);
        helper_type_ident::<K>(&mut h);
        helper_type_ident::<V>(&mut h);
        TypeId(h.finalize().into())
    }
}

impl<V: Typeable> Typeable for OrdSet<V> {
    fn type_ident() -> TypeId {
        let mut h = Sha256::new();
        helper_type_constructor(&mut h, "im__OrdSet");
        helper_type_args_count(&mut h, 1);
        helper_type_ident::<V>(&mut h);
        TypeId(h.finalize().into())
    }
}
