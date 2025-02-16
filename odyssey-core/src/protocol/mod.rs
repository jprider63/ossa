use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use tokio::{net::TcpStream, sync::watch};

use crate::core::OdysseyType;

pub mod heartbeat;
pub mod manager;
pub mod v0;

/// The protocol version.
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub enum Version {
    V0 = 0,
}

impl Version {
    pub fn as_byte(&self) -> u8 {
        *self as u8
    }

    pub async fn run_miniprotocols_server<O: OdysseyType>(&self, stream: TcpStream, active_stores: watch::Receiver<BTreeSet<O::StoreId>>) {
        match self {
            Version::V0 => v0::run_miniprotocols_server::<O>(stream, active_stores).await,
        }
    }

    pub async fn run_miniprotocols_client<O: OdysseyType>(&self, stream: TcpStream, active_stores: watch::Receiver<BTreeSet<O::StoreId>>) {
        match self {
            Version::V0 => v0::run_miniprotocols_client::<O>(stream, active_stores).await,
        }
    }
}

pub(crate) const LATEST_VERSION: Version = Version::V0;
