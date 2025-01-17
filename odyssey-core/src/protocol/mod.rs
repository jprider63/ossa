use serde::{Deserialize, Serialize};
use tokio::{
    net::TcpStream,
    runtime::Runtime,
};

pub mod heartbeat;
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

    pub async fn run_miniprotocols_server(&self, stream: TcpStream) {
        match self {
            Version::V0 => {
                v0::run_miniprotocols_server(stream).await
            }
        }
    }

    pub async fn run_miniprotocols_client(&self, stream: TcpStream) {
        match self {
            Version::V0 => {
                v0::run_miniprotocols_client(stream).await
            }
        }
    }
}

pub(crate) const LATEST_VERSION: Version = Version::V0;
