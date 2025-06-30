use std::collections::BTreeSet;
use std::marker::Send;

// use async_session_types::{Eps, Recv, Send};
use bytes::{BufMut, Bytes, BytesMut};
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::fmt::Debug;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    runtime::Runtime,
    sync::{mpsc::{self, Receiver, Sender, UnboundedSender}, watch},
};
use tokio_util::sync::{PollSendError, PollSender};
use tokio_stream::wrappers::ReceiverStream;

use crate::{auth::DeviceId, core::{OdysseyType, StoreStatuses}, network::{
    multiplexer::{run_miniprotocol_async, Multiplexer, MultiplexerCommand, Party, StreamId},
    protocol::MiniProtocol,
}};
use crate::protocol::MiniProtocolArgs;
use crate::protocol::heartbeat::v0::Heartbeat;
use crate::protocol::manager::v0::Manager;
use crate::store;
use crate::util;

// Required since Rust can't handle proper existentials.
pub(crate) enum MiniProtocols<StoreId, Hash, HeaderId, Header> {
    Heartbeat(Heartbeat),
    Manager(Manager<StoreId, Hash, HeaderId, Header>),
}

impl<StoreId: Send + Sync + Copy + AsRef<[u8]> + Ord + Debug + Serialize + for<'a> Deserialize<'a>, Hash, HeaderId, Header> MiniProtocols<StoreId, Hash, HeaderId, Header> {
    pub(crate) async fn run_async<O: OdysseyType>(
        self,
        is_client: bool,
        stream_id: StreamId,
        sender: Sender<(StreamId, Bytes)>,
        receiver: Receiver<BytesMut>,
    ) {
        match self {
            MiniProtocols::Heartbeat(p) => run_miniprotocol_async::<_, O>(p, is_client, stream_id, sender, receiver).await,
            MiniProtocols::Manager(p) => run_miniprotocol_async::<_, O>(p, is_client, stream_id, sender, receiver).await,
        }
    }
}

// Multiplexer:
//  StreamId (u32)
//  DataLength (u32?)
//
// Miniprotocols:
// 0 - Heartbeat (Server)
// 1 - StreamManagement Client (AdvertiseStores, CloseConnection, TerminateConnection, CreateStream, CloseStream? (probably not))
// 2 - StreamManagement Server
// ...
// N - (N is odd for client, even for server):
//     - StoreSync i
/// Miniprotocols initially run when connected for V0.
fn initial_miniprotocols<StoreId, Hash, HeaderId, Header>(party: Party, args: MiniProtocolArgs<StoreId>, multiplexer_cmd_send: UnboundedSender<MultiplexerCommand>) -> Vec<MiniProtocols<StoreId, Hash, HeaderId, Header>> {
    let (client_chan, server_chan) = if let Party::Client = party {
        (Some(args.manager_channel), None)
    } else {
        (None, Some(args.manager_channel))
    };

    // Order impacts stream id in multiplexer!
    vec![
        MiniProtocols::Heartbeat(Heartbeat {}),
        MiniProtocols::Manager(Manager::new(Party::Client, args.peer_id, args.active_stores.clone(), client_chan, 1, multiplexer_cmd_send.clone())),
        MiniProtocols::Manager(Manager::new(Party::Server, args.peer_id, args.active_stores, server_chan, 2, multiplexer_cmd_send)),
    ]
}

pub(crate) async fn run_miniprotocols_server<O: OdysseyType>(stream: TcpStream, args: MiniProtocolArgs<O::StoreId>) {
    run_miniprotocols::<O>(stream, args, Party::Server).await
}

pub(crate) async fn run_miniprotocols_client<O: OdysseyType>(stream: TcpStream, args: MiniProtocolArgs<O::StoreId>) {
    run_miniprotocols::<O>(stream, args, Party::Client).await
}

async fn run_miniprotocols<O: OdysseyType>(stream: TcpStream, args: MiniProtocolArgs<O::StoreId>, party: Party) {
    // Start multiplexer.
    let (mux_cmd_send, mux_cmd_recv) = mpsc::unbounded_channel();
    let multiplexer = Multiplexer::new(party, mux_cmd_recv);

    multiplexer
        .run_with_miniprotocols::<O>(stream, initial_miniprotocols(party, args, mux_cmd_send))
        .await;
}

// # Protocols run between peers.

// request_store_metadata_header :: RequestStoreMetadataHeaderV0 -> Either<ProtocolError, ResponseStoreMetadataHeaderV0>
// pub type StoreMetadataHeader<StoreId> = Send<
//     StoreMetadataHeaderRequest<StoreId>,
//     Recv<ProtocolResult<StoreMetadataHeaderResponse<StoreId>>, Eps>,
// >;
// pub type StoreMetadataBody =
//     Send<StoreMetadataBodyRequest, Recv<ProtocolResult<StoreMetadataBodyResponse>, Eps>>;

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
    pub body: Option<StoreMetadataBodyResponse<StoreId>>,
}

pub type StoreMetadataBodyRequest = (); // TODO: Eventually request certain chunks.
pub type StoreMetadataBodyResponse<StoreId> = store::v0::MetadataBody<StoreId>; // TODO: Eventually request certain chunks.

pub type ProtocolResult<T> = Result<T, ProtocolError>;
pub type ProtocolError = String; // TODO: Eventually more informative error type.
