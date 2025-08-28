use std::fmt::Display;

use rand_core::OsRng;
use serde::{Deserialize, Serialize};

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

#[derive(Debug, Clone)]
pub struct Identity {
    // Authentication key.
    auth_key: ed25519_dalek::SigningKey,
    // Signing key.
    // Encryption key.
}

impl Identity {
    pub(crate) fn auth_key(&self) -> &ed25519_dalek::SigningKey {
        &self.auth_key
    }
}

pub fn generate_identity() -> Identity {
    let mut rng = OsRng;
    let auth_key = ed25519_dalek::SigningKey::generate(&mut rng);

    Identity { auth_key }
}
