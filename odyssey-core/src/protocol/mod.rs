use serde::{Deserialize, Serialize};
use tokio::{net::TcpStream, sync::watch};

use crate::{auth::DeviceId, core::{OdysseyType, StoreStatuses}};

pub mod heartbeat;
pub mod manager;
pub mod store_peer;
pub mod v0;

pub(crate) struct MiniProtocolArgs<StoreId> {
    peer_id: DeviceId,
    active_stores: watch::Receiver<StoreStatuses<StoreId>>,
}

impl<StoreId> MiniProtocolArgs<StoreId> {
    pub(crate) fn new(peer_id: DeviceId, active_stores: watch::Receiver<StoreStatuses<StoreId>>) -> Self {
        Self { peer_id, active_stores }
    }
}

/// The protocol version.
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub enum Version {
    V0 = 0,
}

impl Version {
    pub fn as_byte(&self) -> u8 {
        *self as u8
    }

    pub async fn run_miniprotocols_server<O: OdysseyType>(&self, stream: TcpStream, args: MiniProtocolArgs<O::StoreId>) {
        match self {
            Version::V0 => v0::run_miniprotocols_server::<O>(stream, args).await,
        }
    }

    pub async fn run_miniprotocols_client<O: OdysseyType>(&self, stream: TcpStream, args: MiniProtocolArgs<O::StoreId>) {
        match self {
            Version::V0 => v0::run_miniprotocols_client::<O>(stream, args).await,
        }
    }
}

pub(crate) const LATEST_VERSION: Version = Version::V0;
