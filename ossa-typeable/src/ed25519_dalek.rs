use sha2::{Digest, Sha256};

use crate::{internal::helper_type_constructor, TypeId, Typeable};

impl Typeable for ed25519_dalek::VerifyingKey {
    fn type_ident() -> TypeId {
        let mut h = Sha256::new();
        helper_type_constructor(&mut h, "ed25519_dalek__VerifyingKey");
        TypeId(h.finalize().into())
    }
}
