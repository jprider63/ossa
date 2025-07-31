#![feature(btree_extract_if)]
#![feature(iterator_try_collect, map_try_insert)]
#![feature(impl_trait_in_assoc_type)]
#![feature(type_alias_impl_trait)]

// #![deny(unused_must_use)]

pub mod auth;
pub mod core;
pub mod network;
pub mod protocol;
pub mod storage;
pub mod store;
pub mod time;
pub mod util;

pub use core::{Ossa, OssaConfig};
