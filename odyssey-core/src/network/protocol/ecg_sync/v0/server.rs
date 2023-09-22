
use crate::network::{ConnectionManager};
use crate::network::protocol::ecg_sync::v0::{ECGSyncError, HeaderBitmap, MAX_DELIVER_HEADERS, MAX_HAVE_HEADERS, MsgECGSync, MsgECGSyncRequest, MsgECGSyncResponse, handle_received_ecg_sync, handle_received_have, mark_as_known, prepare_haves, prepare_headers, ecg};
use std::cmp::min;
use std::collections::{BinaryHeap, BTreeSet, VecDeque};

pub(crate) async fn ecg_sync_server<StoreId, HeaderId>(conn: &ConnectionManager, store_id: &StoreId, state: &ecg::State<HeaderId>) -> Result<(), ECGSyncError>
where HeaderId:Copy + Ord
{
    let request: MsgECGSyncRequest<HeaderId> = conn.receive().await;
    // JP: Set (and check) max value for tips?

    let their_tips_c = request.tip_count;
    let mut their_tips:Vec<HeaderId> = Vec::with_capacity(usize::from(their_tips_c));
    let mut their_tips_remaining = usize::from(their_tips_c);

    // Headers they know.
    let mut their_known = BTreeSet::new();

    // Queue of headers to potentially send.
    // JP: Priority queue by depth?
    let mut send_queue = BinaryHeap::new();

    // TODO: Check for no headers? their_tips_c == 0? or request.have.len() == 0?

    // Handle the haves that the peer sent to us.
    let mut known_bitmap = HeaderBitmap::new([0;4]); // TODO: MAX_HAVE_HEADERS/8?
    handle_received_have(state, &mut their_tips_remaining, &mut their_tips, &mut their_known, &mut send_queue, &request.have, &mut known_bitmap);

    let mut headers = Vec::with_capacity(MAX_DELIVER_HEADERS.into());
    prepare_headers(state, &mut send_queue, &mut their_known, &mut headers);

    let our_tips = state.tips();
    let our_tips_c = u16::try_from(our_tips.len()).map_err(|e| ECGSyncError::TooManyTips(e))?;
    // JP: Set a max value for our_tips_c?

    // Initialize the priority queue with our tips, zipped with distance 0.
    let mut queue = BinaryHeap::new();
    queue.extend(our_tips.iter().map(|x| (true, state.get_header_depth(x), *x, 0)));

    let mut haves = Vec::with_capacity(MAX_HAVE_HEADERS.into());
    prepare_haves(state, &mut queue, &their_known, &mut haves);

    // TODO: Check if we're done.
    let mut done = false;

    let response: MsgECGSyncResponse<HeaderId> = MsgECGSyncResponse {
        tip_count: our_tips_c,
        sync: MsgECGSync {
            have: haves.clone(), // TODO: Skip this clone
            known: known_bitmap,
            headers: headers.clone(), // TODO: Skip this clone
        },
    };

    conn.send(response).await;

    while !done {
        // Receive sync msg
        let received_sync_msg: MsgECGSync<HeaderId> = conn.receive().await;

        done = handle_received_ecg_sync(received_sync_msg, state, &mut their_tips_remaining, &mut their_tips, &mut their_known, &mut send_queue, &mut queue, &mut haves, &mut headers, &mut known_bitmap);
        unimplemented!("TODO: Fix this `done` termination handling");

        // Send sync msg
        let send_sync_msg: MsgECGSync<HeaderId> = MsgECGSync {
            have: haves.clone(), // TODO: Avoid this clone.
            known: known_bitmap,
            headers: headers.clone(), // TODO: Skip this clone.
        };
        conn.send(send_sync_msg).await;

        // // TODO: Check if we're done.
        // done = false;
    }

    Ok(())
}
