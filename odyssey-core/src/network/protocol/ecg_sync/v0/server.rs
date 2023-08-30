
use crate::network::{ConnectionManager};
use crate::network::protocol::ecg_sync::v0::{ECGSyncError, MsgECGSyncRequest};
use crate::network::protocol::ecg_sync::v0::client::ecg;

pub(crate) async fn ecg_sync_server<StoreId, HeaderId>(conn: &ConnectionManager, store_id: &StoreId, state: &ecg::State<HeaderId>) -> Result<(), ECGSyncError>
{
    // let request = conn.receive().await;

    unimplemented!{}
}
