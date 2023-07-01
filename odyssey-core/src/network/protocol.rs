
use futures::{SinkExt,StreamExt};
use serde::{Deserialize, Serialize};
use serde_cbor::to_vec;
use std::marker::Send;
use tokio::net::TcpStream;
use tokio_util::codec::{self, LengthDelimitedCodec};

use crate::protocol::Version;
use crate::protocol::v0::{StoreMetadataHeaderRequest, StoreMetadataHeaderResponse};
use crate::store::v0::MetadataHeader;

// pub enum ProtocolVersion {
//     V0,
// }

pub(crate) fn run_handshake_server<Transport>(stream: &Transport) -> Version {
    // TODO: Implement this and make it abstract.

    Version::V0
}

// TODO: Generalize the argument.
// pub(crate) async fn run_store_metadata_server<'a, StoreId:Deserialize<'a>>(stream: &mut codec::Framed<TcpStream, LengthDelimitedCodec>) -> () {
pub(crate) async fn run_store_metadata_server<StoreId>(stream: &mut codec::Framed<TcpStream, LengthDelimitedCodec>) -> ()
where
  StoreId:for<'a> Deserialize<'a> + Send
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

    // TODO: Proper response.
    let response = StoreMetadataHeaderResponse {
        header: MetadataHeader {
            nonce: [0;32],
            protocol_version: Version::V0,
            store_type: [1;32],
            body_size: 0,
            body_hash: [2;32],
        },
        body: None,
    };
    // TODO: to_writer instead?
    // serde_cbor::to_writer(&stream, &response);
    let cbor = match serde_cbor::to_vec(&response) {
        Err(err) => {
            log::error!("Failed to serialize StoreMetadataHeaderResponse: {}", err);
            // TODO: Respond with failure message.
            return
        }
        Ok(res) => res,
    };
    match stream.send(cbor.into()).await {
        Err(err) => {
            log::error!("Failed to send StoreMetadataHeaderResponse: {}", err);
            return
        }
        Ok(()) => (),
    };

    ()
}

