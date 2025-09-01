//! Authentication and access control.

use std::fmt::Display;

use serde::{Deserialize, Serialize};

pub mod identity;
pub mod group;

#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct DeviceId {
    auth_key: ed25519_dalek::VerifyingKey,
}

impl PartialOrd for DeviceId {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for DeviceId {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.auth_key.as_bytes().cmp(other.auth_key.as_bytes())
    }
}

impl Display for DeviceId {
    // Display devide ids as base 58.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let base58 = bs58::encode(self.auth_key.as_bytes()).into_string();
        write!(f, "{base58}")?;
        Ok(())
    }
}

impl DeviceId {
    pub(crate) fn new(auth_key: ed25519_dalek::VerifyingKey) -> Self {
        Self { auth_key }
    }
}


