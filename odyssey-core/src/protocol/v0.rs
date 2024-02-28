
use async_session_types::{Eps, Send, Recv};
use serde::{Deserialize, Serialize};

use crate::store;

// # Protocols run between peers.

// request_store_metadata_header :: RequestStoreMetadataHeaderV0 -> Either<ProtocolError, ResponseStoreMetadataHeaderV0>
pub type StoreMetadataHeader<TypeId, StoreId> = Send<StoreMetadataHeaderRequest<StoreId>,Recv<ProtocolResult<StoreMetadataHeaderResponse<TypeId, StoreId>>, Eps>>;
pub type StoreMetadataBody = Send<StoreMetadataBodyRequest,Recv<ProtocolResult<StoreMetadataBodyResponse>, Eps>>;

//
// TODO: Do we need to do a handshake to establish a MAC key (is an encryption needed)?.
//

// # Messages sent by protocols.
pub enum MsgStoreMetadataHeader<TypeId, StoreId> {
    Request(StoreMetadataHeaderRequest<StoreId>),
    Response(StoreMetadataHeaderResponse<TypeId, StoreId>),
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

