use crate::network::protocol::ecg_sync::v0::{
    handle_received_ecg_sync, prepare_haves, ECGSyncError, ECGSyncMessage, HeaderBitmap,
    MsgECGSync, MsgECGSyncData, MsgECGSyncRequest, MsgECGSyncResponse, MAX_DELIVER_HEADERS,
    MAX_HAVE_HEADERS,
};
use crate::network::ConnectionManager;
use crate::store::dag::{self, ECGHeader};
use crate::util::Stream;
use ossa_crdt::CRDT;
use std::collections::{BTreeSet, BinaryHeap};
use std::fmt::Debug;
use std::marker::PhantomData;

// TODO: Have this utilize session types.
/// Sync the headers of the eventual consistency graph.
/// Attempts to find the greatest common ancestor (meet) of the graphs.
/// Then we share/receive all the known headers after that point (in batches of size 32).
pub(crate) async fn ecg_sync_client<S: Stream<MsgECGSync<Header, T>>, StoreId, Header, T: CRDT>(
    conn: &mut ConnectionManager<S>,
    store_id: &StoreId,
    state: &mut dag::State<Header, T>,
) -> Result<(), ECGSyncError>
where
    Header: Clone + ECGHeader + Debug,
{
    // TODO:
    // - Get cached peer state.
    // - Retrieve snapshot of store's state.

    let our_tips = state.tips();
    let our_tips_c = u16::try_from(our_tips.len()).map_err(|e| ECGSyncError::TooManyTips(e))?;
    // JP: Set a max value for our_tips_c?

    // Initialize the queue with our tips, zipped with distance 0.
    let mut queue = BinaryHeap::new();
    queue.extend(our_tips.iter().map(|x| {
        if let Some(depth) = state.get_header_depth(x) {
            (true, depth, *x, 0)
        } else {
            // TODO XXX
            todo!("Do we need to do anything?")
        }
    }));
    // JP: Use a priority queue based on descending depth instead?

    // Headers they know.
    let mut their_known = BTreeSet::new();

    let mut haves = Vec::with_capacity(MAX_HAVE_HEADERS.into());
    prepare_haves(state, &mut queue, &their_known, &mut haves);

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
    let request: MsgECGSyncRequest<Header, T> = MsgECGSyncRequest {
        tip_count: our_tips_c,
        have: haves.clone(), // TODO: Avoid this clone?
        phantom: PhantomData,
    };

    // Check if we're done.
    let mut done = request.is_done();

    conn.send(request).await;

    let response: MsgECGSyncResponse<Header, T> = conn.receive().await;
    // JP: Set (and check) max value for tips?

    // Check if we're done.
    done = done && response.is_done();

    let their_tips_c = response.tip_count;
    let mut their_tips: Vec<Header::HeaderId> = Vec::with_capacity(usize::from(their_tips_c));
    let mut their_tips_remaining = usize::from(their_tips_c);

    // Queue of headers to potentially send.
    // JP: Priority queue by depth?
    let mut send_queue = BinaryHeap::new();

    let mut headers = Vec::with_capacity(MAX_DELIVER_HEADERS.into());
    let mut known_bitmap = HeaderBitmap::new([0; 4]); // TODO: MAX_HAVE_HEADERS/8?

    handle_received_ecg_sync(
        response.sync,
        state,
        &mut their_tips_remaining,
        &mut their_tips,
        &mut their_known,
        &mut send_queue,
        &mut queue,
        &mut haves,
        &mut headers,
        &mut known_bitmap,
    );

    // // Receive (and verify) the headers they sent to us
    // let all_valid = handle_received_headers(state, response.sync.headers);
    // // TODO: Record and exit if they sent invalid headers? Or tit for tat?

    // // TODO: Check for no headers? their_tips_c == 0

    // // Handle the haves that the peer sent to us.
    // let known = handle_received_have(state, &mut their_tips_remaining, &mut their_tips, &mut their_known, &mut send_queue, &response.sync.have);

    // // Send the headers we have.
    // prepare_headers(state, &mut send_queue, &mut their_known, &mut headers);

    // // Propose headers we have.
    // prepare_haves(state, &mut queue, &their_known, &mut haves);

    while !done {
        // Send sync msg
        let send_sync_msg: MsgECGSyncData<Header, T> = MsgECGSyncData {
            have: haves.clone(), // TODO: Avoid this clone. // .iter().map(|x| x.0).collect(),
            known: known_bitmap,
            headers: headers.clone(), // TODO: Skip this clone.
            phantom: PhantomData,
        };
        done = send_sync_msg.is_done();
        conn.send(send_sync_msg).await;

        // Receive sync msg
        let received_sync_msg: MsgECGSyncData<Header, T> = conn.receive().await;
        done = done && received_sync_msg.is_done();

        handle_received_ecg_sync(
            received_sync_msg,
            state,
            &mut their_tips_remaining,
            &mut their_tips,
            &mut their_known,
            &mut send_queue,
            &mut queue,
            &mut haves,
            &mut headers,
            &mut known_bitmap,
        );
    }

    Ok(())
}
