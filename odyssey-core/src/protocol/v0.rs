use async_session_types::{Eps, Recv, Send};
use bytes::{BytesMut, BufMut};
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    runtime::Runtime,
};

use crate::protocol::heartbeat;
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
//     - SyncStore i
pub(crate) async fn run_miniprotocols_server(mut stream: TcpStream) {
    // Start multiplexer.

    // Create window (buffered channel?) for each miniprotocol.
    let (mut heartbeat_server_channel, heartbeat_protocol_channel) = util::Channel::new_pair(10);

    // Spawn async for each miniprotocol.
    let heartbeat_handle = tokio::spawn(async move {
        heartbeat::v0::run_server_wrapper(heartbeat_protocol_channel).await
    });

    // TODO: back pressure
    let mut state = ();

    // TODO: Do some load balancing between miniprotocols?
    // TODO: Pipelining
    
    loop {
        let mut buf = BytesMut::with_capacity(4096);

        // Wait on data from client or data to send.
        tokio::select! {
            msg_e = heartbeat_server_channel.next() => {
                match msg_e {
                    None => {
                        todo!()
                    }
                    Some(Err(_e)) => {
                        todo!()
                    }
                    Some(Ok(mut msg)) => {
                        // TODO: Send stream id, etc.
                        stream.write_buf(&mut msg).await;
                    }
                }
            }
            result = stream.read_buf(&mut buf) => {
                // TODO: Check length, read stream id, etc.
                heartbeat_server_channel.send(buf).await; // TODO: This currently blocks if the
                                                          // channel is full
            }
        }


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
}

pub(crate) async fn run_miniprotocols_client(mut stream: TcpStream) {
    // Start multiplexer.
    // TODO: Abstract everything behind a multiplexer abstraction.

    // Create window (buffered channel?) for each miniprotocol.
    let (mut heartbeat_client_channel, heartbeat_protocol_channel) = util::Channel::new_pair(10);

    // Spawn async for each miniprotocol.
    let heartbeat_handle = tokio::spawn(async move {
        heartbeat::v0::run_client_wrapper(heartbeat_protocol_channel).await
    });

    // TODO: back pressure
    let mut state = ();

    // TODO: Do some load balancing between miniprotocols?
    // TODO: Pipelining
    
    loop {
        let mut buf = BytesMut::with_capacity(4096);

        // Wait on data from client or data to send.
        tokio::select! {
            msg_e = heartbeat_client_channel.next() => {
                match msg_e {
                    None => {
                        todo!()
                    }
                    Some(Err(_e)) => {
                        todo!()
                    }
                    Some(Ok(mut msg)) => {
                        // TODO: Send stream id, etc.
                        stream.write_buf(&mut msg).await;
                    }
                }
            }
            result = stream.read_buf(&mut buf) => {
                // TODO: Check length, read stream id, etc.
                heartbeat_client_channel.send(buf).await; // TODO: This currently blocks if the
                                                          // channel is full
            }
        }
    }
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
