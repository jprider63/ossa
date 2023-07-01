
use serde::{Deserialize, Serialize};

pub mod v0;

/// The protocol version.
#[derive(Clone, Copy, Deserialize, Serialize)]
pub enum Version {
    V0 = 0,
}

impl Version {
    pub fn as_byte(&self) -> u8 {
        *self as u8
    }
}

pub(crate) const LATEST_VERSION: Version = Version::V0;
