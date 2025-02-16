use bytes::Bytes;
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_cbor::to_vec;
use std::any::type_name;
use std::collections::BTreeSet;
use std::fmt::Debug;
use std::future::Future;
use std::marker::Send;
use tokio::net::TcpStream;
use tokio::sync::watch;
use tokio_util::{
    codec::{self, LengthDelimitedCodec},
    sync::PollSendError,
};

use crate::{core::OdysseyType, network::multiplexer};
use crate::protocol::v0::{
    MsgStoreMetadataHeader, StoreMetadataHeaderRequest, StoreMetadataHeaderResponse,
};
use crate::protocol::Version;
use crate::store::v0::MetadataHeader;
use crate::util::Stream;

pub mod ecg_sync;
pub mod keep_alive;

pub(crate) trait MiniProtocol: Send {
    type Message: Serialize + for<'a> Deserialize<'a> + Send;

    fn run_client<S: Stream<Self::Message>, O: OdysseyType>(self, stream: S, active_stores: watch::Receiver<BTreeSet<O::StoreId>>,) -> impl Future<Output = ()> + Send;
    fn run_server<S: Stream<Self::Message>, O: OdysseyType>(self, stream: S, active_stores: watch::Receiver<BTreeSet<O::StoreId>>,) -> impl Future<Output = ()> + Send;
}

// pub enum ProtocolVersion {
//     V0,
// }

type MsgHandshake = ();

pub(crate) async fn run_handshake_server<S: Stream<MsgHandshake>>(stream: &S) -> Version {
    // TODO: Implement this and make it abstract.

    Version::V0
}

pub(crate) async fn run_handshake_client<S: Stream<MsgHandshake>>(stream: &S) -> Version {
    // TODO: Implement this and make it abstract.

    Version::V0
}

// TODO: Generalize the argument.
// pub(crate) async fn run_store_metadata_server<'a, StoreId:Deserialize<'a>>(stream: &mut codec::Framed<TcpStream, LengthDelimitedCodec>) -> () {
pub(crate) async fn run_store_metadata_server<
    StoreId,
    S: Stream<MsgStoreMetadataHeader<StoreId>>,
>(
    stream: &mut S,
) -> Result<(), ProtocolError>
where
    StoreId: for<'a> Deserialize<'a> + Send + Debug,
{
    let req: StoreMetadataHeaderRequest<StoreId> = receive(stream).await?;
    log::info!("Received request: {:?}", req);

    // TODO: Proper response.
    let response: StoreMetadataHeaderResponse<StoreId> = StoreMetadataHeaderResponse {
        header: MetadataHeader {
            nonce: [0; 32],
            protocol_version: Version::V0,
            store_type: todo!(), // [1;32],
            body_size: 0,
            body_hash: todo!(), // [2;32],
        },
        body: None,
    };

    send(stream, response).await
}

// pub async fn run_store_metadata_client<TypeId, StoreId, S:Stream<MsgStoreMetadataHeader<TypeId, StoreId>>>(stream: &mut codec::Framed<TcpStream, LengthDelimitedCodec>, request: &StoreMetadataHeaderRequest<StoreId>) -> Result<StoreMetadataHeaderResponse<TypeId, StoreId>, ProtocolError>
pub async fn run_store_metadata_client<
    StoreId,
    S: Stream<MsgStoreMetadataHeader<StoreId>>,
>(
    stream: &mut S,
    request: StoreMetadataHeaderRequest<StoreId>,
) -> Result<StoreMetadataHeaderResponse<StoreId>, ProtocolError>
where
    StoreId: Serialize + for<'a> Deserialize<'a> + Debug,
{
    send(stream, request).await?;

    receive(stream).await
}

#[derive(Debug)]
pub enum ProtocolError {
    SerializationError(serde_cbor::Error),
    DeserializationError(serde_cbor::Error),
    ReceivedNoData, // Connection closed?
    StreamSendError(std::io::Error),
    StreamReceiveError(std::io::Error),
    ProtocolDeviation, // Temporary?
    ChannelSendError(PollSendError<(multiplexer::StreamId, Bytes)>),
}

/// Send a message over the given stream.
pub(crate) async fn send<S, T, U>(stream: &mut S, message: T) -> Result<(), ProtocolError>
where
    S: Stream<U>,
    T: Into<U>,
{
    match stream.send(message.into()).await {
        Err(err) => {
            // TODO: Push the error up the stack instead of recording it here?
            log::error!("Failed to send {}: {:?}", type_name::<T>(), err);
            Err(err)
        }
        Ok(()) => Ok(()),
    }
}

/// Receive a message from the given stream.
pub(crate) async fn receive<S, T, U>(stream: &mut S) -> Result<U, ProtocolError>
where
    S: Stream<T>,
    U: Debug,
    T: TryInto<U>,
{
    match stream.next().await {
        None => {
            log::error!("Failed to receive data from peer"); // Closed connection?
            Err(ProtocolError::ReceivedNoData)
        }
        Some(Err(err)) => {
            log::error!("Error while receiving data from peer: {:?}", err);
            Err(err)
        }
        Some(Ok(msg)) => {
            match msg.try_into() {
                Err(err) => {
                    log::error!("Received unexpected data from peer"); // : {:?}", err);
                    Err(ProtocolError::ProtocolDeviation)
                }
                Ok(msg) => {
                    log::debug!("Received data from peer: {:?}", msg);
                    Ok(msg)
                }
            }
        }
    }
}
