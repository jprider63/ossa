
use futures::stream::StreamExt;
use serde::{Deserialize, Serialize};
use tokio::net::TcpStream;
use tokio_util::codec::{self, LengthDelimitedCodec};

use crate::protocol::v0::StoreMetadataHeaderRequest;

pub enum ProtocolVersion {
    V0,
}

pub(crate) fn run_handshake_server<Transport>(stream: &Transport) -> ProtocolVersion {
    // TODO: Implement this and make it abstract.

    ProtocolVersion::V0
}

// TODO: Generalize the argument.
// pub(crate) async fn run_store_metadata_server<'a, StoreId:Deserialize<'a>>(stream: &mut codec::Framed<TcpStream, LengthDelimitedCodec>) -> () {
pub(crate) async fn run_store_metadata_server<StoreId>(stream: &mut codec::Framed<TcpStream, LengthDelimitedCodec>) -> ()
where
  StoreId:for<'a> Deserialize<'a>
{
    let received = match stream.next().await {
        None => {
            log::error!("Failed to receive data from peer");
            return
        }
        Some(Err(err)) => {
            log::error!("Failed to receive data from peer: {}", err);
            return
        }
        Some(Ok(bytes)) => bytes,
    };

    let req : StoreMetadataHeaderRequest<StoreId> = match serde_cbor::from_slice(&received) {
        Err(err) => {
            log::error!("Failed to parse StoreMetadataHeaderRequest: {}", err);
            // TODO: Respond with failure message.
            return
        }
        Ok(r) => r,
    };

    ()
}

