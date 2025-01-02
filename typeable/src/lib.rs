pub mod internal;

use internal::helper_string_non_ascii;
use lazy_static::lazy_static;
use sha2::{Digest, Sha256};
use std::fmt;
pub use typeable_derive::Typeable;

use crate::internal::{helper_type_args, helper_type_constructor, helper_usize};

/// A unique identifier for a type. It is typically derived from the sha256 hash of the type's declaration.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct TypeId([u8; 32]);

impl TypeId {
    pub fn new(h: [u8; 32]) -> TypeId {
        TypeId(h)
    }

    pub fn identifier(&self) -> [u8; 32] {
        self.0
    }
}

impl fmt::Display for TypeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(f, "0x")?;
        for b in self.0 {
            write!(f, "{:02X}", b)?;
        }
        Ok(())
    }
}

/// A trait that specifies unique identifiers for types in a deterministic way that can be shared
/// on multiple machines over the network.
///
/// Requirements:
/// - 1-1 mapping between a (fully instantiated) type and its identifier, assuming no hash collisions.
/// - A type identifier should be deterministic so that it is the same on different machines (and compilation configurations).
/// - Changes to a type's semantics should result in a different type identifier. By default, type
/// identifiers are derived from type declarations, so changes to semantics like function
/// implementations should cause the type identifier to change. In practice, this may require tag
/// updates (ex. changing a tag from "v1" to "v2" when a type's implementation changes).
///
/// The following is the grammar of how the type is hashed to create its identifier.
///
/// typeable :=
///   type_name type_arg_count tag type_body
///
/// type_name := string
/// type_arg_count := u8
/// type_body :=
///     // Struct
///     '0' fields
///     // Enum
///   | '1' variant_count variant*
///
/// fields :=
///     // Named fields {}
///     '0' field_count field
///     // Unnamed fields ()
///   | '1' field_count type_ident*
///
/// field := field_name type_ident
/// field_name := string
///
/// tag := string | _
///
/// variant := variant_name fields
///
/// field_count := u8
/// char_count := u8
/// variant_count := u8
///
/// string := char_count alphanumeric
/// alphanumeric := [a-zA-Z0-9_]*
///
/// type_ident := [u8; 32]
///
pub trait Typeable {
    /// A unique identifier for a given type.
    fn type_ident() -> TypeId; // JP: This should be a const, but rust doesn't like that.
}

macro_rules! derive_typeable_primitive {
    ( $type_name: ident ) => {
        mod $type_name {
            use super::*;

            lazy_static! {
                static ref DERIVED_TYPE_ID: TypeId = {
                    let mut h = Sha256::new();
                    helper_type_constructor(&mut h, stringify!($type_name));
                    TypeId(h.finalize().into())
                };
            }

            impl Typeable for $type_name {
                fn type_ident() -> TypeId {
                    *DERIVED_TYPE_ID
                }
            }
        }
    };
}

derive_typeable_primitive!(bool);
derive_typeable_primitive!(char);
derive_typeable_primitive!(u8);
derive_typeable_primitive!(u16);
derive_typeable_primitive!(u32);
derive_typeable_primitive!(u64);
derive_typeable_primitive!(u128);
derive_typeable_primitive!(i8);
derive_typeable_primitive!(i16);
derive_typeable_primitive!(i32);
derive_typeable_primitive!(i64);
derive_typeable_primitive!(i128);
derive_typeable_primitive!(f32);
derive_typeable_primitive!(f64);

impl<T, const N: usize> Typeable for [T; N] {
    fn type_ident() -> TypeId {
        let mut h = Sha256::new();
        helper_string_non_ascii(&mut h, "[]");
        helper_type_args(&mut h, 1);
        helper_usize(&mut h, N);
        TypeId(h.finalize().into())
    }
}

impl<T> Typeable for Vec<T> {
    fn type_ident() -> TypeId {
        let mut h = Sha256::new();
        helper_type_constructor(&mut h, "Vec");
        helper_type_args(&mut h, 1);
        TypeId(h.finalize().into())
    }
}
