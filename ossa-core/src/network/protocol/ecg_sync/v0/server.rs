use crate::network::protocol::ecg_sync::v0::{
    ecg, handle_received_ecg_sync, handle_received_have, mark_as_known, prepare_haves,
    prepare_headers, ECGSyncError, ECGSyncMessage, HeaderBitmap, MsgECGSync, MsgECGSyncData,
    MsgECGSyncRequest, MsgECGSyncResponse, MAX_DELIVER_HEADERS, MAX_HAVE_HEADERS,
};
use crate::network::ConnectionManager;
use crate::store::ecg::ECGHeader;
use crate::util::Stream;
use ossa_crdt::CRDT;
use std::cmp::min;
use std::collections::{BTreeSet, BinaryHeap, VecDeque};
use std::fmt::Debug;
use std::marker::PhantomData;

pub(crate) async fn ecg_sync_server<S: Stream<MsgECGSync<Header, T>>, StoreId, Header, T: CRDT>(
    conn: &mut ConnectionManager<S>,
    store_id: &StoreId,
    state: &mut ecg::State<Header, T>,
) -> Result<(), ECGSyncError>
where
    Header: Clone + ECGHeader + Debug,
{
    let request: MsgECGSyncRequest<Header, T> = conn.receive().await;
    // JP: Set (and check) max value for tips?

    // Check if we're done.
    let mut done = request.is_done();

    let their_tips_c = request.tip_count;
    let mut their_tips: Vec<Header::HeaderId> = Vec::with_capacity(usize::from(their_tips_c));
    let mut their_tips_remaining = usize::from(their_tips_c);

    // Headers they know.
    let mut their_known = BTreeSet::new();

    // Priority queue of headers (by depth) to potentially send.
    let mut send_queue = BinaryHeap::new();

    // TODO: Check for no headers? their_tips_c == 0? or request.have.len() == 0?

    // Handle the haves that the peer sent to us.
    let mut known_bitmap = HeaderBitmap::new([0; 4]); // TODO: MAX_HAVE_HEADERS/8?
    handle_received_have(
        state,
        &mut their_tips_remaining,
        &mut their_tips,
        &mut their_known,
        // &mut send_queue,
        &request.have,
        &mut known_bitmap,
    );

    let mut headers = Vec::with_capacity(MAX_DELIVER_HEADERS.into());
    prepare_headers(state, &mut send_queue, &mut their_known, &mut headers);

    let our_tips = state.tips();
    let our_tips_c = u16::try_from(our_tips.len()).map_err(|e| ECGSyncError::TooManyTips(e))?;
    // JP: Set a max value for our_tips_c?

    // Initialize the priority queue with our tips, zipped with distance 0.
    let mut queue = BinaryHeap::new();
    queue.extend(our_tips.iter().map(|x| {
        if let Some(depth) = state.get_header_depth(x) {
            (true, depth, *x, 0)
        } else {
            // TODO XXX
            todo!("Do we need to do anything?")
        }
    }));

    let mut haves = Vec::with_capacity(MAX_HAVE_HEADERS.into());
    prepare_haves(state, &mut queue, &their_known, &mut haves);

    let response: MsgECGSyncResponse<Header, T> = MsgECGSyncResponse {
        tip_count: our_tips_c,
        sync: MsgECGSyncData {
            have: haves.clone(), // TODO: Skip this clone
            known: known_bitmap,
            headers: headers.clone(), // TODO: Skip this clone
            phantom: PhantomData,
        },
    };

    // Check if we're done.
    done = done && response.is_done();

    conn.send(response).await;

    while !done {
        // Receive sync msg
        let received_sync_msg: MsgECGSyncData<Header, T> = conn.receive().await;
        done = received_sync_msg.is_done();

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

        // Send sync msg
        let send_sync_msg: MsgECGSyncData<Header, T> = MsgECGSyncData {
            have: haves.clone(), // TODO: Avoid this clone.
            known: known_bitmap,
            headers: headers.clone(), // TODO: Skip this clone.
            phantom: PhantomData,
        };
        done = done && send_sync_msg.is_done();
        conn.send(send_sync_msg).await;

        // // TODO: Check if we're done.
        // done = false;
    }

    Ok(())
}
