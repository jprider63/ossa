
use crate::network::{ConnectionManager};
use crate::network::protocol::ecg_sync::v0::{ECGSyncError, MsgECGSyncRequest};


// TODO: Move this somewhere else. store::state::ecg?
pub mod ecg {
    pub struct State<HeaderId> {
        // Tips of the ECG (hashes of their headers).
        tips: Vec<HeaderId>,
    }

    impl<HeaderId> State<HeaderId> {
        pub fn tips(&self) -> &[HeaderId] {
            &self.tips
        }
    }
}

// TODO: Have this utilize the KeepAlive session type.
/// Sync the headers of the eventual consistency graph.
/// Finds the least common ancestor (meet) of the graphs.
/// Then we share/receive all the known headers after that point (in batches of size 32).
pub(crate) async fn ecg_sync_client<StoreId, HeaderId>(conn: &ConnectionManager, store_id: &StoreId, state: &ecg::State<HeaderId>) -> Result<(), ECGSyncError>
where HeaderId:Clone
{
    // TODO:
    // - Get cached peer state.
    // - Retrieve snapshot of store's state.

    let our_tips = state.tips();
    let tips_c = u16::try_from(our_tips.len()).map_err(|e| ECGSyncError::TooManyTips(e))?;

    // let mut sync_state = ECGSyncState::new(our_tips);
    // let haves = sync_state.initial_request();
    let haves = our_tips.to_vec();

    conn.send(MsgECGSyncRequest{
        tip_count: tips_c,
        have: haves,
    }).await;

    // let response = conn.receive().await;
    // if let Some() = sync_state.initial_response();


    // while let Some()


    // let sent_have = vec![]; // TODO
    
    // conn.send(MsgECGSyncRequest{
    //     tips: our_tips,
    //     have: sent_have,
    // }).await;


    Ok(())
}
