
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_cbor::to_vec;
use std::any::type_name;
use std::fmt::Debug;
use std::marker::Send;
use tokio::net::TcpStream;
use tokio_util::codec::{self, LengthDelimitedCodec};

use crate::protocol::Version;
use crate::protocol::v0::{MsgStoreMetadataHeader, StoreMetadataHeaderRequest, StoreMetadataHeaderResponse};
use crate::store::v0::MetadataHeader;
use crate::util::Stream;

pub mod ecg_sync;
pub mod keep_alive;

// pub enum ProtocolVersion {
//     V0,
// }

pub(crate) fn run_handshake_server<TypeId, StoreId, S:Stream<MsgStoreMetadataHeader<TypeId, StoreId>>>(stream: &S) -> Version {
    // TODO: Implement this and make it abstract.

    Version::V0
}

pub(crate) fn run_handshake_client<TypeId, StoreId, S:Stream<MsgStoreMetadataHeader<TypeId, StoreId>>>(stream: &S) -> Version {
    // TODO: Implement this and make it abstract.

    Version::V0
}

// TODO: Generalize the argument.
// pub(crate) async fn run_store_metadata_server<'a, StoreId:Deserialize<'a>>(stream: &mut codec::Framed<TcpStream, LengthDelimitedCodec>) -> () {
pub(crate) async fn run_store_metadata_server<TypeId, StoreId, S:Stream<MsgStoreMetadataHeader<TypeId, StoreId>>>(stream: &mut S) -> Result<(), ProtocolError>
where
  StoreId:for<'a> Deserialize<'a> + Send + Debug
{
    let req : StoreMetadataHeaderRequest<StoreId> = receive(stream).await?;
    log::info!("Received request: {:?}", req);

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

    send(stream, &response).await
}

pub async fn run_store_metadata_client<TypeId, StoreId, S:Stream<MsgStoreMetadataHeader<TypeId, StoreId>>>(stream: &mut codec::Framed<TcpStream, LengthDelimitedCodec>, request: &StoreMetadataHeaderRequest<StoreId>) -> Result<StoreMetadataHeaderResponse<TypeId, StoreId>, ProtocolError>
where
    StoreId: Serialize + for<'a> Deserialize<'a>,
    TypeId: for<'a> Deserialize<'a>,
{
    send(stream, request).await?;

    receive(stream).await
}

#[derive(Debug)]
pub enum ProtocolError {
    SerializationError(serde_cbor::Error),
    DeserializationError(serde_cbor::Error),
    StreamSendError(std::io::Error),
    ReceivedNoData, // Connection closed?
    StreamReceiveError(std::io::Error),
}

/// Send a message as CBOR over the given stream.
async fn send<T, S>(stream: &mut S, message: &T) -> Result<(), ProtocolError>
where
    S: Stream<T>,
    T: Serialize,
{
    // TODO: to_writer instead?
    // serde_cbor::to_writer(&stream, &response);
    match serde_cbor::to_vec(&message) {
        Err(err) => {
            // TODO: Push the error up the stack instead of recording it here?
            log::error!("Failed to serialize {}: {}", type_name::<T>(), err);
            // TODO: Respond with failure message?
            Err(ProtocolError::SerializationError(err))
        }
        Ok(cbor) => {
            match stream.send(cbor.into()).await {
                Err(err) => {
                    // TODO: Push the error up the stack instead of recording it here?
                    log::error!("Failed to send {}: {}", type_name::<T>(), err);
                    Err(ProtocolError::StreamSendError(err))
                }
                Ok(()) => Ok(()),
            }
        }
    }
}

/// Receive a message as CBOR from the given stream.
async fn receive<S, T>(stream: &mut S) -> Result<T, ProtocolError>
where
    S: Stream<T>,
    T: for<'a> Deserialize<'a>,
{
    match stream.next().await {
        None => {
            log::error!("Failed to receive data from peer"); // Closed connection?
            Err(ProtocolError::ReceivedNoData)
        }
        Some(Err(err)) => {
            log::error!("Failed to receive data from peer: {}", err);
            Err(ProtocolError::StreamReceiveError(err))
        }
        Some(Ok(bytes)) => {
            match serde_cbor::from_slice(&bytes) {
                Err(err) => {
                    log::error!("Failed to parse {}: {}", type_name::<T>(), err);
                    // TODO: Respond with failure message.
                    Err(ProtocolError::DeserializationError(err))
                }
                Ok(msg) => Ok(msg),
            }
        }
    }

}

