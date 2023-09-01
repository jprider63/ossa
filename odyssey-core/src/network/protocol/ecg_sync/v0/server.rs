
use crate::network::{ConnectionManager};
use crate::network::protocol::ecg_sync::v0::{ECGSyncError, MsgECGSyncRequest};
use crate::network::protocol::ecg_sync::v0::client::ecg;

pub(crate) async fn ecg_sync_server<StoreId, HeaderId>(conn: &ConnectionManager, store_id: &StoreId, state: &ecg::State<HeaderId>) -> Result<(), ECGSyncError>
{
    let request:MsgECGSyncRequest<HeaderId> = conn.receive().await;
    // TODO: 

    let their_tips_c = request.tip_count;
    let their_tips:Vec<HeaderId> = Vec::with_capacity(usize::from(their_tips_c));

    let our_tips = state.tips();
    let our_tips_c = u16::try_from(our_tips.len()).map_err(|e| ECGSyncError::TooManyTips(e))?;

    let mut our_tips_c_remaining = usize::from(our_tips_c);

    unimplemented!{}
}
