
use crate::network::{ConnectionManager};
use crate::network::protocol::ecg_sync::v0::{ECGSyncError, MAX_HAVE_HEADERS, MsgECGSyncRequest};
// use std::collections::BTreeMap;


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
    let our_tips_c = u16::try_from(our_tips.len()).map_err(|e| ECGSyncError::TooManyTips(e))?;
    // JP: Set a max value for our_tips_c?

    // TODO:
    // sort tips by depth
    // zip tips with \x -> (x,0)
    // Add tips to BFS queue

    let mut our_tips_c_remaining = usize::from(our_tips_c);
    // let mut sent_haves = BTreeSet::new();

    // Loop:

    // let mut sync_state = ECGSyncState::new(our_tips);
    // let haves = sync_state.initial_request();
    let mut haves = Vec::with_capacity(MAX_HAVE_HEADERS);

    // Send any remaining tips. This is a no-op when our_tips_c_remaining is 0;
    let i = usize::from(our_tips_c) - our_tips_c_remaining;
    haves.extend_from_slice(&our_tips[i..i+MAX_HAVE_HEADERS]);
    our_tips_c_remaining = our_tips_c_remaining - haves.len();
    
    // TODO: Fill remaining slots in `haves` with ancestors.
    // - Order tips by depth (priority queue?).
    // - BFS to send ancestors (at exponential depths, ex: 0, 1, 2, 4, 8) until slots are
    // - full or we reach the root. If a header is already sent/proposed, skip it.
    // JP: Could just use the BFS instead of keeping track of tips remaining if we don't reorder.

    conn.send(MsgECGSyncRequest{
        tip_count: our_tips_c,
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
