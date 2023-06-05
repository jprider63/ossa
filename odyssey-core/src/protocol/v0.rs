
use async_session_types::{Eps, Send, Recv};

use crate::store;

// # Protocols run between peers.

// request_store_metadata_header :: RequestStoreMetadataHeaderV0 -> Either<ProtocolError, ResponseStoreMetadataHeaderV0>
pub type StoreMetadataHeader<TypeId, StoreId> = Send<StoreMetadataHeaderRequest<StoreId>,Recv<ProtocolResult<StoreMetadataHeaderResponse<TypeId, StoreId>>, Eps>>;
pub type StoreMetadataBody = Send<StoreMetadataBodyRequest,Recv<ProtocolResult<StoreMetadataBodyResponse>, Eps>>;

//
// TODO: Do we need to do a handshake to establish a MAC key (is an encryption needed)?.
//

// # Messages sent by protocols.

pub struct StoreMetadataHeaderRequest<StoreId> {
    store_id: StoreId,
    body_request: Option<StoreMetadataBodyRequest>,
}

pub struct StoreMetadataHeaderResponse<TypeId, StoreId> {
    header: store::v0::MetadataHeader<TypeId, StoreId>,
    body: Option<StoreMetadataBodyResponse>,
}

pub type StoreMetadataBodyRequest = (); // TODO: Eventually request certain chunks.
pub type StoreMetadataBodyResponse = store::v0::MetadataBody; // TODO: Eventually request certain chunks.

pub type ProtocolResult<T> = Result<T, ProtocolError>;
pub type ProtocolError = String; // TODO: Eventually more informative error type.

