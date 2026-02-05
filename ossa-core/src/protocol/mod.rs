use serde::{Deserialize, Serialize};
use tokio::{
    net::TcpStream,
    sync::{mpsc::UnboundedReceiver, watch},
};

use crate::{
    auth::DeviceId,
    core::{OssaType, StoreStatuses},
    protocol::manager::v0::PeerManagerCommand,
    store::dag::DAGHeader,
};

pub mod heartbeat;
pub mod manager;
pub mod store_peer;
pub mod store_sc_dag;
pub mod store_bft_sync;
pub mod v0;

pub(crate) struct MiniProtocolArgs<StoreId, Hash, SHeaderId, SHeader, THeaderId, THeader> {
    peer_id: DeviceId,
    active_stores:
        watch::Receiver<StoreStatuses<StoreId, Hash, SHeaderId, SHeader, THeaderId, THeader>>,
    manager_channel: UnboundedReceiver<PeerManagerCommand<StoreId>>,
}

impl<StoreId, Hash, SHeaderId, SHeader, THeaderId, THeader>
    MiniProtocolArgs<StoreId, Hash, SHeaderId, SHeader, THeaderId, THeader>
{
    pub(crate) fn new(
        peer_id: DeviceId,
        active_stores: watch::Receiver<
            StoreStatuses<StoreId, Hash, SHeaderId, SHeader, THeaderId, THeader>,
        >,
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
            <O::SCGHeader as DAGHeader>::HeaderId,
            O::SCGHeader,
            <O::ECGHeader as DAGHeader>::HeaderId,
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
            <O::SCGHeader as DAGHeader>::HeaderId,
            O::SCGHeader,
            <O::ECGHeader as DAGHeader>::HeaderId,
            O::ECGHeader,
        >,
    ) {
        match self {
            Version::V0 => v0::run_miniprotocols_client::<O>(stream, args).await,
        }
    }
}

pub(crate) const LATEST_VERSION: Version = Version::V0;
