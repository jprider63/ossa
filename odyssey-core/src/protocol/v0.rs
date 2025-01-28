use async_session_types::{Eps, Recv, Send};
use bytes::{BufMut, BytesMut};
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    runtime::Runtime,
};

use crate::network::{
    multiplexer::{Multiplexer, Party},
    protocol::MiniProtocol,
};
use crate::protocol::heartbeat::v0::Heartbeat;
use crate::store;
use crate::util;

// Multiplexer:
//  StreamId (u32)
//  DataLength (u32?)
//
// Miniprotocols:
// 0 - StreamManagement (CloseConnection, TerminateConnection, CreateStream, CloseStream? (probably not))
// 1 - Heartbeat
// 2 - AdvertiseStores
// ...
// N - (N is odd for server, even for client):
//     - StoreSync i
/// Miniprotocols initially run when connected for V0.
fn initial_miniprotocols() -> Vec<impl MiniProtocol> {
    vec![Heartbeat {}]
}

pub(crate) async fn run_miniprotocols_server(mut stream: TcpStream) {
    // Start multiplexer.
    let multiplexer = Multiplexer::new(Party::Server);

    multiplexer
        .run_with_miniprotocols(stream, initial_miniprotocols())
        .await;

    // // Wait for the socket to be readable
    // match stream.readable().await {
    //     Ok(()) => {},
    //     Err(e) => todo!(),
    // }

    // // Try to read data, this may still fail with `WouldBlock`
    // // if the readiness event is a false positive.
    // match stream.try_read_buf(&mut buf) {
    //     Ok(0) => break,
    //     Ok(n) => {
    //         println!("read {} bytes", n);

    //     }
    //     Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
    //         continue;
    //     }
    //     Err(e) => {
    //         todo!("handle this error"); // return Err(e.into());
    //     }
    // }

    // // Wait for the socket to be writable
    // stream.writable().await?;
}

pub(crate) async fn run_miniprotocols_client(mut stream: TcpStream) {
    // Start multiplexer.
    let multiplexer = Multiplexer::new(Party::Client);

    multiplexer
        .run_with_miniprotocols(stream, initial_miniprotocols())
        .await;
}

// # Protocols run between peers.

// request_store_metadata_header :: RequestStoreMetadataHeaderV0 -> Either<ProtocolError, ResponseStoreMetadataHeaderV0>
pub type StoreMetadataHeader<TypeId, StoreId> = Send<
    StoreMetadataHeaderRequest<StoreId>,
    Recv<ProtocolResult<StoreMetadataHeaderResponse<TypeId, StoreId>>, Eps>,
>;
pub type StoreMetadataBody =
    Send<StoreMetadataBodyRequest, Recv<ProtocolResult<StoreMetadataBodyResponse>, Eps>>;

//
// TODO: Do we need to do a handshake to establish a MAC key (is an encryption needed)?.
//

// # Messages sent by protocols.
#[derive(Debug, Serialize, Deserialize)]
pub enum MsgStoreMetadataHeader<TypeId, StoreId> {
    Request(StoreMetadataHeaderRequest<StoreId>),
    Response(StoreMetadataHeaderResponse<TypeId, StoreId>),
}

impl<TypeId, StoreId> Into<MsgStoreMetadataHeader<TypeId, StoreId>>
    for StoreMetadataHeaderRequest<StoreId>
{
    fn into(self) -> MsgStoreMetadataHeader<TypeId, StoreId> {
        MsgStoreMetadataHeader::Request(self)
    }
}
impl<TypeId, StoreId> Into<MsgStoreMetadataHeader<TypeId, StoreId>>
    for StoreMetadataHeaderResponse<TypeId, StoreId>
{
    fn into(self) -> MsgStoreMetadataHeader<TypeId, StoreId> {
        MsgStoreMetadataHeader::Response(self)
    }
}
impl<TypeId, StoreId> TryInto<StoreMetadataHeaderRequest<StoreId>>
    for MsgStoreMetadataHeader<TypeId, StoreId>
{
    type Error = ();
    fn try_into(self) -> Result<StoreMetadataHeaderRequest<StoreId>, ()> {
        match self {
            MsgStoreMetadataHeader::Request(r) => Ok(r),
            MsgStoreMetadataHeader::Response(_) => Err(()),
        }
    }
}
impl<TypeId, StoreId> TryInto<StoreMetadataHeaderResponse<TypeId, StoreId>>
    for MsgStoreMetadataHeader<TypeId, StoreId>
{
    type Error = ();
    fn try_into(self) -> Result<StoreMetadataHeaderResponse<TypeId, StoreId>, ()> {
        match self {
            MsgStoreMetadataHeader::Response(r) => Ok(r),
            MsgStoreMetadataHeader::Request(_) => Err(()),
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct StoreMetadataHeaderRequest<StoreId> {
    pub store_id: StoreId,
    pub body_request: Option<StoreMetadataBodyRequest>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct StoreMetadataHeaderResponse<TypeId, StoreId> {
    pub header: store::v0::MetadataHeader<TypeId, StoreId>,
    pub body: Option<StoreMetadataBodyResponse>,
}

pub type StoreMetadataBodyRequest = (); // TODO: Eventually request certain chunks.
pub type StoreMetadataBodyResponse = store::v0::MetadataBody; // TODO: Eventually request certain chunks.

pub type ProtocolResult<T> = Result<T, ProtocolError>;
pub type ProtocolError = String; // TODO: Eventually more informative error type.
