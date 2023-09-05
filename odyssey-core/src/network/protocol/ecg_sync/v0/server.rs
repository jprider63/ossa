
use crate::network::{ConnectionManager};
use crate::network::protocol::ecg_sync::v0::{ECGSyncError, HeaderBitmap, MAX_DELIVER_HEADERS, MAX_HAVE_HEADERS, MsgECGSync, MsgECGSyncRequest, MsgECGSyncResponse, mark_as_known, prepare_haves, prepare_headers, ecg};
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
    let mut send_queue = VecDeque::new();

    // TODO: Check for no headers? their_tips_c == 0

    // TODO:
    // Accumulate their_tips.
    let provided_tip_c = min(their_tips_remaining, request.have.len());
    their_tips.extend(&request.have[0..provided_tip_c]);
    their_tips_remaining = their_tips_remaining - provided_tip_c;
    // TODO: if their_tips is done, update peer_state.

    let mut known = HeaderBitmap::new([0;4]);
    for (i, header_id) in request.have.iter().enumerate() {
        if state.contains(header_id) {
            // Record their known headers.
            mark_as_known(state, &mut their_known, *header_id);

            // Respond with which headers we know.
            known.set(i, true);

            // If we know the header, potentially send the children of that header.
            send_queue.extend(state.get_children(&header_id));
        }
    }

    let mut headers = Vec::with_capacity(MAX_DELIVER_HEADERS.into());
    prepare_headers(state, &mut send_queue, &mut their_known, &mut headers);

    let our_tips = state.tips();
    let our_tips_c = u16::try_from(our_tips.len()).map_err(|e| ECGSyncError::TooManyTips(e))?;

    // Initialize the priority queue with our tips, zipped with distance 0.
    let mut queue = BinaryHeap::new();
    queue.extend(our_tips.iter().map(|x| (true, state.get_header_depth(x), *x, 0)));
    // JP: Use a priority queue based on descending depth instead?

    let mut haves = Vec::with_capacity(MAX_HAVE_HEADERS.into());
    prepare_haves(state, &mut queue, &mut haves);

    // TODO: Check if we're done.

    let response: MsgECGSyncResponse<HeaderId> = MsgECGSyncResponse {
        tip_count: our_tips_c,
        sync: MsgECGSync {
            have: haves.iter().map(|x| x.0).collect(),
            known: known,
            headers: headers,
        },
    };

    conn.send(response).await;

    unimplemented!{}
}
