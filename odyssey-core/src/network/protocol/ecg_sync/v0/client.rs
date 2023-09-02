
use crate::network::{ConnectionManager};
use crate::network::protocol::ecg_sync::v0::{ECGSyncError, MAX_HAVE_HEADERS, MsgECGSyncRequest, MsgECGSyncResponse, prepare_haves, ecg};
// use std::collections::BTreeMap;
use std::collections::VecDeque;




// TODO: Have this utilize session types.
/// Sync the headers of the eventual consistency graph.
/// Finds the least common ancestor (meet) of the graphs.
/// Then we share/receive all the known headers after that point (in batches of size 32).
pub(crate) async fn ecg_sync_client<StoreId, HeaderId>(conn: &ConnectionManager, store_id: &StoreId, state: &ecg::State<HeaderId>) -> Result<(), ECGSyncError>
where HeaderId:Copy
{
    // TODO:
    // - Get cached peer state.
    // - Retrieve snapshot of store's state.

    let our_tips = state.tips();
    let our_tips_c = u16::try_from(our_tips.len()).map_err(|e| ECGSyncError::TooManyTips(e))?;
    // JP: Set a max value for our_tips_c?

    // Initialize the queue with our tips, zipped with distance 0.
    let mut queue = VecDeque::new();
    queue.extend(our_tips.iter().map(|x| (*x,0)));
    // JP: Use a priority queue based on descending depth instead?

    let mut haves = Vec::with_capacity(MAX_HAVE_HEADERS.into());
    prepare_haves(state, &mut queue, &mut haves);

    // let mut our_tips_c_remaining = usize::from(our_tips_c);
    // let mut sent_haves = BTreeSet::new();

    // Loop:

    // let mut sync_state = ECGSyncState::new(our_tips);
    // let haves = sync_state.initial_request();


    // Send any remaining tips. This is a no-op when our_tips_c_remaining is 0;
    // let i = usize::from(our_tips_c) - our_tips_c_remaining;
    // haves.extend_from_slice(&our_tips[i..i+MAX_HAVE_HEADERS]);
    // our_tips_c_remaining = our_tips_c_remaining - haves.len();
    
    // TODO: Fill remaining slots in `haves` with ancestors.
    // - Order tips by depth (priority queue?).
    // - BFS to send ancestors (at exponential depths, ex: 0, 1, 2, 4, 8) until slots are
    // - full or we reach the root. If a header is already sent/proposed, skip it.
    // JP: Could just use the BFS instead of keeping track of tips remaining if we don't reorder.

    // let sent = haves;
    let req: MsgECGSyncRequest<HeaderId> = MsgECGSyncRequest {
        tip_count: our_tips_c,
        have: haves.iter().map(|x| x.0).collect(),
    };
    conn.send(req).await;

    let response: MsgECGSyncResponse<HeaderId> = conn.receive().await;
    // if let Some() = sync_state.initial_response();


    // while let Some()


    // let sent_have = vec![]; // TODO
    
    // conn.send(MsgECGSyncRequest{
    //     tips: our_tips,
    //     have: sent_have,
    // }).await;


    Ok(())
}
