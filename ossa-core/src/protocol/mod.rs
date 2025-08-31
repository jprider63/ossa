use serde::{Deserialize, Serialize};
use tokio::{
    net::TcpStream,
    sync::{mpsc::UnboundedReceiver, watch},
};

use crate::{
    auth::DeviceId,
    core::{OssaType, StoreStatuses},
    protocol::manager::v0::PeerManagerCommand,
    store::dag::ECGHeader,
};

pub mod heartbeat;
pub mod manager;
pub mod store_peer;
pub mod v0;

pub(crate) struct MiniProtocolArgs<StoreId, Hash, HeaderId, Header> {
    peer_id: DeviceId,
    active_stores: watch::Receiver<StoreStatuses<StoreId, Hash, HeaderId, Header>>,
    manager_channel: UnboundedReceiver<PeerManagerCommand<StoreId>>,
}

impl<StoreId, Hash, HeaderId, Header> MiniProtocolArgs<StoreId, Hash, HeaderId, Header> {
    pub(crate) fn new(
        peer_id: DeviceId,
        active_stores: watch::Receiver<StoreStatuses<StoreId, Hash, HeaderId, Header>>,
        manager_channel: UnboundedReceiver<PeerManagerCommand<StoreId>>,
    ) -> Self {
        Self {
            peer_id,
            active_stores,
            manager_channel,
        }
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

    pub(crate) async fn run_miniprotocols_server<O: OssaType>(
        &self,
        stream: TcpStream,
        args: MiniProtocolArgs<
            O::StoreId,
            O::Hash,
            <O::ECGHeader as ECGHeader>::HeaderId,
            O::ECGHeader,
        >,
    ) {
        match self {
            Version::V0 => v0::run_miniprotocols_server::<O>(stream, args).await,
        }
    }

    pub(crate) async fn run_miniprotocols_client<O: OssaType>(
        &self,
        stream: TcpStream,
        args: MiniProtocolArgs<
            O::StoreId,
            O::Hash,
            <O::ECGHeader as ECGHeader>::HeaderId,
            O::ECGHeader,
        >,
    ) {
        match self {
            Version::V0 => v0::run_miniprotocols_client::<O>(stream, args).await,
        }
    }
}

pub(crate) const LATEST_VERSION: Version = Version::V0;
