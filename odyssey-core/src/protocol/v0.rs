use std::collections::BTreeSet;

use async_session_types::{Eps, Recv, Send};
use bytes::{BufMut, Bytes, BytesMut};
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    runtime::Runtime,
    sync::{mpsc::{self, Receiver, Sender}, watch},
};
use tokio_util::sync::{PollSendError, PollSender};
use tokio_stream::wrappers::ReceiverStream;

use crate::network::{
    multiplexer::{Multiplexer, Party, StreamId, spawn_miniprotocol_async},
    protocol::MiniProtocol,
};
use crate::protocol::heartbeat::v0::Heartbeat;
use crate::protocol::manager::v0::Manager;
use crate::store;
use crate::util;

// Required since Rust can't handle proper existentials.
pub(crate) enum MiniProtocols {
    Heartbeat(Heartbeat),
    Manager(Manager),
}

impl MiniProtocols {
    pub(crate) async fn spawn_async(
        self,
        is_client: bool,
        stream_id: StreamId,
        sender: Sender<(StreamId, Bytes)>,
        receiver: Receiver<BytesMut>,
    ) {
        match self {
            MiniProtocols::Heartbeat(p) => spawn_miniprotocol_async(p, is_client, stream_id, sender, receiver).await,
            MiniProtocols::Manager(p) => spawn_miniprotocol_async(p, is_client, stream_id, sender, receiver).await,
        }
    }
}

// Multiplexer:
//  StreamId (u32)
//  DataLength (u32?)
//
// Miniprotocols:
// 0 - Heartbeat
// 1 - StreamManagement Client (AdvertiseStores, CloseConnection, TerminateConnection, CreateStream, CloseStream? (probably not))
// 2 - StreamManagement Server
// ...
// N - (N is odd for server, even for client):
//     - StoreSync i
/// Miniprotocols initially run when connected for V0.
fn initial_miniprotocols() -> Vec<MiniProtocols> {
    // Order impacts stream id in multiplexer!
    vec![
        MiniProtocols::Heartbeat(Heartbeat {}),
        MiniProtocols::Manager(Manager::new(Party::Client)),
        MiniProtocols::Manager(Manager::new(Party::Server)),
    ]
}

pub(crate) async fn run_miniprotocols_server<StoreId>(stream: TcpStream, active_stores: watch::Receiver<BTreeSet<StoreId>>) {
    // Start multiplexer.
    let multiplexer = Multiplexer::new(Party::Server);

    multiplexer
        .run_with_miniprotocols(stream, initial_miniprotocols(), active_stores)
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

pub(crate) async fn run_miniprotocols_client<StoreId>(stream: TcpStream, active_stores: watch::Receiver<BTreeSet<StoreId>>) {
    // Start multiplexer.
    let multiplexer = Multiplexer::new(Party::Client);

    multiplexer
        .run_with_miniprotocols(stream, initial_miniprotocols(), active_stores)
        .await;
}

// # Protocols run between peers.

// request_store_metadata_header :: RequestStoreMetadataHeaderV0 -> Either<ProtocolError, ResponseStoreMetadataHeaderV0>
pub type StoreMetadataHeader<StoreId> = Send<
    StoreMetadataHeaderRequest<StoreId>,
    Recv<ProtocolResult<StoreMetadataHeaderResponse<StoreId>>, Eps>,
>;
pub type StoreMetadataBody =
    Send<StoreMetadataBodyRequest, Recv<ProtocolResult<StoreMetadataBodyResponse>, Eps>>;

//
// TODO: Do we need to do a handshake to establish a MAC key (is an encryption needed)?.
//

// # Messages sent by protocols.
#[derive(Debug, Serialize, Deserialize)]
pub enum MsgStoreMetadataHeader<StoreId> {
    Request(StoreMetadataHeaderRequest<StoreId>),
    Response(StoreMetadataHeaderResponse<StoreId>),
}

impl<StoreId> Into<MsgStoreMetadataHeader<StoreId>>
    for StoreMetadataHeaderRequest<StoreId>
{
    fn into(self) -> MsgStoreMetadataHeader<StoreId> {
        MsgStoreMetadataHeader::Request(self)
    }
}
impl<StoreId> Into<MsgStoreMetadataHeader<StoreId>>
    for StoreMetadataHeaderResponse<StoreId>
{
    fn into(self) -> MsgStoreMetadataHeader<StoreId> {
        MsgStoreMetadataHeader::Response(self)
    }
}
impl<StoreId> TryInto<StoreMetadataHeaderRequest<StoreId>>
    for MsgStoreMetadataHeader<StoreId>
{
    type Error = ();
    fn try_into(self) -> Result<StoreMetadataHeaderRequest<StoreId>, ()> {
        match self {
            MsgStoreMetadataHeader::Request(r) => Ok(r),
            MsgStoreMetadataHeader::Response(_) => Err(()),
        }
    }
}
impl<StoreId> TryInto<StoreMetadataHeaderResponse<StoreId>>
    for MsgStoreMetadataHeader<StoreId>
{
    type Error = ();
    fn try_into(self) -> Result<StoreMetadataHeaderResponse<StoreId>, ()> {
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
pub struct StoreMetadataHeaderResponse<StoreId> {
    pub header: store::v0::MetadataHeader<StoreId>,
    pub body: Option<StoreMetadataBodyResponse>,
}

pub type StoreMetadataBodyRequest = (); // TODO: Eventually request certain chunks.
pub type StoreMetadataBodyResponse = store::v0::MetadataBody; // TODO: Eventually request certain chunks.

pub type ProtocolResult<T> = Result<T, ProtocolError>;
pub type ProtocolError = String; // TODO: Eventually more informative error type.
