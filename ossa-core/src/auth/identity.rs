
use std::collections::BTreeMap;

use rand_core::OsRng;

use crate::{store::{bft::SCDT, StoreRef}, util::Sha256Hash};

#[derive(Debug, Clone)]
/// A device's private keys.
pub struct DevicePrivateKeys {
    // Authentication key.
    auth_key: ed25519_dalek::SigningKey,
    // Signing key.
    // Encryption key.
}

impl DevicePrivateKeys {
    pub(crate) fn auth_key(&self) -> &ed25519_dalek::SigningKey {
        &self.auth_key
    }

    pub fn generate_device_keys() -> DevicePrivateKeys {
        let mut rng = OsRng;
        let auth_key = ed25519_dalek::SigningKey::generate(&mut rng);
    
        DevicePrivateKeys { auth_key }
    }
}

/// A device's public keys.
#[derive(Debug, PartialEq, Eq)]
pub struct Device {
    // Authentication key.
    auth_key: ed25519_dalek::VerifyingKey,
    // Signing key.
    // Encryption key.
    // Voting key.
}

impl PartialOrd for Device {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Device {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.auth_key.as_bytes().cmp(other.auth_key.as_bytes())
        // TODO: Compare other fields as they are added
    }
}

#[derive(Debug)]
pub enum DeviceRole {
    Validator,
    Proposer,
}

pub type IdentityId = StoreRef<Sha256Hash, Identity, ()>;

#[derive(Debug)]
pub struct Identity {
    // JP: DeviceId instead for key??
    devices: BTreeMap<Device, DeviceRole>,
}

pub enum IdentityOp {
    AddDevice {
        keys: Device,
        role: DeviceRole,
    },
    RemoveDevice {
        device: Device, // DeviceId?
    },
    UpdateDevice {
        device: Device, // DeviceId?
        role: DeviceRole,
    },
}

impl SCDT for Identity {
    type Operation = IdentityOp;

    fn update(mut self, op: Self::Operation) -> Self {
        match op {
            IdentityOp::AddDevice { keys, role } => self.devices.insert(keys, role),
            IdentityOp::RemoveDevice { device } => self.devices.remove(&device),
            IdentityOp::UpdateDevice { device, role } => self.devices.insert(device, role),
        };

        self
    }

    fn is_valid_operation(self, op: Self::Operation) -> bool {
        match op {
            IdentityOp::AddDevice { keys, role: _ } => {
                !self.devices.contains_key(&keys)
            }
            IdentityOp::RemoveDevice { device } => {
                self.devices.contains_key(&device)
            }
            IdentityOp::UpdateDevice { device, role: _ } => {
                self.devices.contains_key(&device)
            }
        }
    }
}

