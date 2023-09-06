
use crate::network::{ConnectionManager};
use crate::network::protocol::ecg_sync::v0::{ECGSyncError, MAX_DELIVER_HEADERS, MAX_HAVE_HEADERS, MsgECGSync, MsgECGSyncRequest, MsgECGSyncResponse, handle_received_have, handle_received_headers, prepare_haves, prepare_headers, ecg};
use std::cmp::min;
use std::collections::{BinaryHeap, BTreeSet};



// TODO: Have this utilize session types.
/// Sync the headers of the eventual consistency graph.
/// Finds the least common ancestor (meet) of the graphs.
/// Then we share/receive all the known headers after that point (in batches of size 32).
pub(crate) async fn ecg_sync_client<StoreId, HeaderId>(conn: &ConnectionManager, store_id: &StoreId, state: &ecg::State<HeaderId>) -> Result<(), ECGSyncError>
where HeaderId:Copy + Ord
{
    // TODO:
    // - Get cached peer state.
    // - Retrieve snapshot of store's state.

    let our_tips = state.tips();
    let our_tips_c = u16::try_from(our_tips.len()).map_err(|e| ECGSyncError::TooManyTips(e))?;
    // JP: Set a max value for our_tips_c?

    // Initialize the queue with our tips, zipped with distance 0.
    let mut queue = BinaryHeap::new();
    queue.extend(our_tips.iter().map(|x| (true, state.get_header_depth(x), *x, 0)));
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
    // JP: Set (and check) max value for tips?

    let their_tips_c = response.tip_count;
    let mut their_tips:Vec<HeaderId> = Vec::with_capacity(usize::from(their_tips_c));
    let mut their_tips_remaining = usize::from(their_tips_c);

    // Receive (and verify) the headers they sent to us
    let all_valid = handle_received_headers(state, response.sync.headers);
    // TODO: Record and exit if they sent invalid headers? Or tit for tat?

    // Headers they know.
    let mut their_known = BTreeSet::new();

    // Queue of headers to potentially send.
    // JP: Priority queue by depth?
    let mut send_queue = BinaryHeap::new();

    // TODO: Check for no headers? their_tips_c == 0

    // Handle the haves that the peer sent to us.
    let known = handle_received_have(state, &mut their_tips_remaining, &mut their_tips, &mut their_known, &mut send_queue, &response.sync.have);

    // Send the headers we have.
    let mut headers = Vec::with_capacity(MAX_DELIVER_HEADERS.into());
    prepare_headers(state, &mut send_queue, &mut their_known, &mut headers);

    // Propose headers we have.
    prepare_haves(state, &mut queue, &mut haves);

    // TODO: Check if we're done.

    let msg: MsgECGSync<HeaderId> = MsgECGSync {
        have: haves.iter().map(|x| x.0).collect(),
        known: known,
        headers: headers,
    };
    conn.send(msg).await;

    // TODO
    // Loop:
    // - Send sync msg
    // - Receive sync msg

    unimplemented!{}
}
