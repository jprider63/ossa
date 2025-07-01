use itertools::Itertools;
use odyssey_crdt::CRDT;
use rand::{seq::SliceRandom as _, thread_rng};
use replace_with::replace_with_or_abort;
use serde::{Deserialize, Serialize};
use tokio::{sync::{mpsc::{UnboundedReceiver, UnboundedSender}, oneshot::{self, Sender}}, task::JoinHandle};
use tracing::{debug, error, warn};
use std::{collections::{BTreeMap, BTreeSet}, ops::Range};
use std::fmt::Debug;
use typeable::{TypeId, Typeable};

use crate::{auth::DeviceId, core::{OdysseyType, SharedState}, network::{multiplexer::{run_miniprotocol_async, SpawnMultiplexerTask}, protocol::MiniProtocol}, protocol::{manager::v0::PeerManagerCommand, store_peer::v0::{MsgStoreSyncRequest, StoreSync, StoreSyncCommand}}, store::{ecg::{ECGBody, ECGHeader}, v0::{MERKLE_REQUEST_LIMIT, PIECE_REQUEST_LIMIT, PIECE_SIZE}}, util::{self, compress_consecutive_into_ranges}};

pub mod ecg;
pub mod v0; // TODO: Move this to network::protocol

pub use v0::{MetadataBody, MetadataHeader, Nonce};

pub struct State<StoreId, Header: ecg::ECGHeader, T: CRDT, Hash> {
    // Peers that also have this store (that we are potentially connected to?).
    peers: BTreeMap<DeviceId, PeerInfo<Header::HeaderId, Header>>, // BTreeSet<DeviceId>,
    state_machine: StateMachine<StoreId, Header, T, Hash>,
    metadata_subscribers: BTreeMap<DeviceId, oneshot::Sender<Option<v0::MetadataHeader<Hash>>>>,
    merkle_subscribers: BTreeMap<DeviceId, (Vec<Range<u64>>, oneshot::Sender<Option<Vec<Hash>>>)>,
    piece_subscribers: BTreeMap<DeviceId, (Vec<Range<u64>>, oneshot::Sender<Option<Vec<Option<Vec<u8>>>>>)>,
    // listeners: Vec<UnboundedSender<StateUpdate<Header, T>>>,
}

// States are:
// - Initializing - Setting up the thread that owns the store (not defined here).
// - DownloadingMetadata - Don't have the header so we're downloading it.
// - Syncing - Have the header and syncing updates between peers.
pub enum StateMachine<StoreId, Header: ecg::ECGHeader, T: CRDT, Hash> {
    DownloadingMetadata {
        store_id: StoreId,
    },
    DownloadingMerkle {
        metadata: MetadataHeader<Hash>,
        piece_hashes: Vec<Option<Hash>>,
    },
    DownloadingInitialState {
        metadata: MetadataHeader<Hash>,
        piece_hashes: Vec<Hash>,
        initial_state: Vec<Option<Vec<u8>>>,
    },
    Syncing {
        metadata: MetadataHeader<Hash>,
        piece_hashes: Vec<Hash>,
        initial_state: Vec<u8>, // Or just T?
        ecg_state: ecg::State<Header, T>,
        decrypted_state: DecryptedState<Header, T>, // Temporary
        // decrypted_state: Option<DecryptedState<Header, T>>, // JP: Is this actually used?
                                                               // Does it make sense?
    },
}

pub struct DecryptedState<Header: ecg::ECGHeader, T: CRDT> {
    /// Latest ECG application state we've seen.
    latest_state: T,

    /// Headers corresponding to the latest ECG application state.
    latest_headers: BTreeSet<Header::HeaderId>,
}

/// Information about a peer.
#[derive(Debug)]
struct PeerInfo<HeaderId, Header> {
    /// Status of incoming sync status from peer.
    incoming_status: PeerStatus<()>,
    /// Status of outgoing sync status to peer.
    outgoing_status: PeerStatus<OutgoingPeerStatus<HeaderId, Header>>,
    ecg_status: ECGStatus<HeaderId>,
}

#[derive(Clone, Debug)]
/// Information about a peer's ECG status.
pub(crate) struct ECGStatus<HeaderId> {
    /// Greatest common ancestor between our ECG graphs.
    pub(crate) meet: Vec<HeaderId>,
    /// Whether we need to update the meet between us and this peer.
    pub(crate) meet_needs_update: bool,
    // JP: Track their_tip?
}

impl<Hash, Header> PeerInfo<Hash, Header> {
    /// Checks if the peer is ready for a sync request.
    /// This means the peer is syncing and does not have an outstanding request.
    fn is_ready_for_sync(&self) -> bool {
        if let PeerStatus::Syncing(s) = &self.outgoing_status {
            !s.is_outstanding
        } else {
            false
        }
    }
}

#[derive(Debug)]
/// Outgoing information about a syncing peer.
struct OutgoingPeerStatus<HeaderId, Header> {
    /// Sender channel for requests to the outgoing peer store.
    sender_peer: UnboundedSender<StoreSyncCommand<HeaderId, Header>>,
    /// Whether we have an outgoing peer request that is outstanding.
    is_outstanding: bool,
}

/// Status of peers who we are potentially syncing this store with.
#[derive(Debug)]
pub(crate) enum PeerStatus<T> {
    /// Peer is known and likely connected to, but is not syncing this store.
    Known, // JP: Inactive?
    /// Setting up the thread that syncs the store with the peer. It's possible that the peer will reject the sync request.
    Initializing,
    // {
    //     task: JoinHandle<()>,
    // },
    /// The thread that syncs the store with the peer is syncing.
    Syncing(T), // JP: Running instead?

//     /// Peers that we are connected to, but are not syncing this store. It's possible that these connections have dropped.
//     Known(), // TODO: Last known IP address, port, statistics (latency, bandwidth, ...). JP: Should some of this be stored globally?
//     /// Peers that we are connected to, but are not syncing this store. It's possible that these connections have dropped.
//     Connected(), 
//     /// Peers that we are connected to and are syncing this store. It's possible that these connections have dropped.
//     Active(), 
}

impl<T> PeerStatus<T> {
    fn is_known(&self) -> bool {
        if let PeerStatus::Known = self {
            true
        } else {
            false
        }
    }

    fn is_initializing(&self) -> bool {
        if let PeerStatus::Initializing = self {
            true
        } else {
            false
        }
    }
}

impl<StoreId: Copy + Eq, Header: ecg::ECGHeader + Clone + Debug, T: CRDT + Clone, Hash: util::Hash + Debug + Into<StoreId>> State<StoreId, Header, T, Hash> {
    /// Initialize a new store with the given state. This initializes the header, including
    /// generating a random nonce.
    pub fn new_syncing(initial_state: T) -> State<StoreId, Header, T, Hash>
    where
        T: Serialize + Typeable,
    {
        let init_body = MetadataBody::new(&initial_state);
        let store_header = MetadataHeader::generate::<T>(&init_body);
        let decrypted_state = DecryptedState {
            latest_state: initial_state,
            latest_headers: BTreeSet::new(),
        };

        let (piece_hashes, initial_state) = init_body.build();

        let state_machine = StateMachine::Syncing {
            metadata: store_header,
            piece_hashes,
            initial_state,
            ecg_state: ecg::State::new(),
            decrypted_state, // : Some(decrypted_state),
        };
        State {
            peers: BTreeMap::new(),
            state_machine,
            metadata_subscribers: BTreeMap::new(),
            merkle_subscribers: BTreeMap::new(),
            piece_subscribers: BTreeMap::new(),
        }
    }

    /// Create a new store with the given store id that is downloading the store's header.
    pub(crate) fn new_downloading(store_id: StoreId) -> Self {
        let state_machine = StateMachine::DownloadingMetadata {
            store_id
        };

        State {
            peers: BTreeMap::new(),
            state_machine,
            metadata_subscribers: BTreeMap::new(),
            merkle_subscribers: BTreeMap::new(),
            piece_subscribers: BTreeMap::new(),
        }
    }

    pub fn store_id(&self) -> StoreId {
        match &self.state_machine {
            StateMachine::DownloadingMetadata{store_id} => {
                *store_id
            }
            StateMachine::DownloadingMerkle { metadata, .. } => {
                metadata.store_id()
            }
            StateMachine::DownloadingInitialState { metadata, .. } => {
                metadata.store_id()
            }
            StateMachine::Syncing { metadata, .. } => {
                metadata.store_id()
            }
        }
    }

    /// Insert a peer as known if its status isn't already tracked by the store.
    fn insert_known_peer(&mut self, peer: DeviceId) {
        let ecg_status = ECGStatus { meet: vec![], meet_needs_update: true };
        self.peers
            .entry(peer)
            // .and_modify(|s| {
            //     match s {
            //         PeerStatus::Known => *s = PeerStatus::Known,
            //         PeerStatus::Initializing => (),
            //         PeerStatus::Syncing => (),
            //     }
            // })
            // .or_insert(PeerStatus::Known);
            .or_insert(PeerInfo { incoming_status: PeerStatus::Known, outgoing_status: PeerStatus::Known, ecg_status});
    }

    /// Helper to update a known peer to initializing.
    fn update_peer_to_initializing<A>(&mut self, peer: &DeviceId, direction_lambda: fn(&mut PeerInfo<Header::HeaderId, Header>) -> &mut PeerStatus<A>)
    where
        A: Debug
    {
        let Some(info) = self.peers.get_mut(peer) else {
            error!("Invariant violated. Attempted to initialize an unknown peer: {}", peer);
            panic!();
        };
        let status = direction_lambda(info);
        if status.is_known() {
            *status = PeerStatus::Initializing;
        } else {
            error!("Invariant violated. Attempted to initialize an already initialized peer: {} - {:?}", peer, status);
            panic!();
        }
    }

    /// Update a known peer's outgoing status to initializing.
    fn update_peer_to_initializing_outgoing(&mut self, peer: &DeviceId) {
        self.update_peer_to_initializing(peer, |info| &mut info.outgoing_status);
    }

    /// Update a known peer's incoming status to initializing.
    fn update_peer_to_initializing_incoming(&mut self, peer: &DeviceId) {
        self.update_peer_to_initializing(peer, |info| &mut info.incoming_status);
    }

    /// Helper to update a known peer to syncing.
    fn update_peer_to_syncing<A>(&mut self, peer: &DeviceId, direction_lambda: fn(&mut PeerInfo<Header::HeaderId, Header>) -> &mut PeerStatus<A>, sender_m: A)
    where
        A: Debug
    {
        let Some(info) = self.peers.get_mut(peer) else {
            error!("Invariant violated. Attempted to initialize an unknown peer: {}", peer);
            panic!();
        };
        let status = direction_lambda(info);
        match status {
            PeerStatus::Initializing => {
                *status = PeerStatus::Syncing(sender_m);
            }
            PeerStatus::Known => {
                error!("Invariant violated. Attempted to initialize an unknown peer: {} - {:?}", peer, status);
                panic!();
            }
            PeerStatus::Syncing(_) => {
                error!("Invariant violated. Attempted to sync an already syncing peer: {} - {:?}", peer, status);
                panic!();
            }
        }
    }

    fn update_peer_to_syncing_incoming(&mut self, peer: &DeviceId) {
        self.update_peer_to_syncing(peer, |info| &mut info.incoming_status, ());
    }

    fn update_peer_to_syncing_outgoing(&mut self, peer: &DeviceId, sender: OutgoingPeerStatus<Header::HeaderId, Header>) {
        self.update_peer_to_syncing(peer, |info| &mut info.outgoing_status, sender);
    }

    fn update_outgoing_peer_to_ready(&mut self, peer: &DeviceId) {
        let Some(info) = self.peers.get_mut(peer) else {
            error!("Invariant violated. Attempted to update an unknown peer: {}", peer);
            panic!();
        };
        match info.outgoing_status {
            PeerStatus::Initializing => {
                error!("Invariant violated. Attempted to update a peer that is initializing: {} - {:?}", peer, info.outgoing_status);
                panic!();
            }
            PeerStatus::Known => {
                error!("Invariant violated. Attempted to update a peer that is not syncing: {} - {:?}", peer, info.outgoing_status);
                panic!();
            }
            PeerStatus::Syncing(ref mut status) => {
                status.is_outstanding = false;
            }
        }
    }

    /// Send sync requests to peers.
    fn send_sync_requests(&mut self) {
        fn send_command<Hash, Header>(i: &mut PeerInfo<Hash, Header>, message: StoreSyncCommand<Hash, Header>) {
            let PeerStatus::Syncing(ref mut s) = i.outgoing_status else {
                unreachable!("Already checked that the peer is ready.");
            };

            // Mark as outstanding.
            s.is_outstanding = true;
            s.sender_peer.send(message).expect("TODO");               
        }

        // Get peers (of this store) without outstanding requests.
        let mut peers: Vec<_> = self.peers.iter_mut().filter(|(_,i)| i.is_ready_for_sync()).collect();
        // TODO: Rank and (weighted) randomize the peers.
        let mut rng = thread_rng();
        peers.shuffle(&mut rng);

        // Send requests to peers based on what we need.
        match &self.state_machine {
            StateMachine::DownloadingMetadata { .. } => {
                // Tell the first peer store task to request the metadata.
                peers.iter_mut().take(1).for_each(|(_, i)| {
                    let message = StoreSyncCommand::MetadataHeaderRequest;
                    send_command(i, message);
                });
            }
            StateMachine::DownloadingMerkle { piece_hashes, .. } => {
                // TODO: Keep track of (and filter out) which ones are currently requested.
                let needed_hashes = piece_hashes.iter().enumerate().filter_map(|h| if h.1.is_none() { Some(h.0 as u64) } else { None }).chunks(MERKLE_REQUEST_LIMIT as usize);
                let mut needed_hashes: Vec<_> = needed_hashes.into_iter().collect();

                // Randomize which peer to request the hashes from.
                needed_hashes.shuffle(&mut rng);

                peers.iter_mut().zip(needed_hashes).for_each(|(p, hashes)| {
                    let message = StoreSyncCommand::MerkleRequest(compress_consecutive_into_ranges(hashes).collect());
                    send_command(p.1, message);
                });
            }
            StateMachine::DownloadingInitialState { metadata, piece_hashes, initial_state } => {
                let needed_pieces = initial_state.iter().enumerate().filter_map(|h| if h.1.is_none() { Some(h.0 as u64) } else { None }).chunks(PIECE_REQUEST_LIMIT as usize);
                let mut needed_pieces: Vec<_> = needed_pieces.into_iter().collect();

                // Randomize which peer to request the pieces from.
                needed_pieces.shuffle(&mut rng);

                peers.iter_mut().zip(needed_pieces).for_each(|(p, pieces)| {
                    let message = StoreSyncCommand::InitialStatePieceRequest(compress_consecutive_into_ranges(pieces).collect());
                    send_command(p.1, message);
                });
            }
            StateMachine::Syncing { metadata, piece_hashes, initial_state, ecg_state, decrypted_state } => {
                // Request ECG updates from peers
                peers.iter_mut().for_each(|p| {
                    let ecg_status = p.1.ecg_status.clone();
                    let ecg_state = ecg_state.state().clone();
                    let message = StoreSyncCommand::ECGSyncRequest{ ecg_status, ecg_state };
                    send_command(p.1, message)
                });
            }
        }
    }

    // Handle a sync request for this store from a peer.
    fn handle_metadata_peer_request(&mut self, peer: DeviceId, response_chan: Sender<HandlePeerResponse<MetadataHeader<Hash>>>) {
        if let Some(metadata) = self.metadata() {
            // We have the metadata so share it with the peer.
            response_chan.send(Ok(*metadata)).expect("TODO");
        } else {
            // We don't have the metadata so tell them to wait.
            let (send_chan, recv_chan) = oneshot::channel();
            response_chan.send(Err(recv_chan)).expect("TODO");

            // Register the wait channel.
            self.metadata_subscribers.insert(peer, send_chan); // JP: Safe to drop old one?
        }
    }

    fn handle_merkle_peer_request(&mut self, peer: DeviceId, piece_ids: Vec<Range<u64>>, response_chan: Sender<HandlePeerResponse<Vec<Hash>>>) {

        let hashes = self.piece_hashes().map(|piece_hashes| {
            handle_merkle_peer_request_helper(piece_hashes, &piece_ids)
        });

        if let Some(piece_hashes) = hashes {
            // We have the hashes so share it with the peer.
            response_chan.send(Ok(piece_hashes)).expect("TODO");
        } else {
            // We don't have the hashes so tell them to wait.
            let (send_chan, recv_chan) = oneshot::channel();
            response_chan.send(Err(recv_chan)).expect("TODO");

            // Register the wait channel.
            self.merkle_subscribers.insert(peer, (piece_ids, send_chan)); // JP: Safe to drop old one?
        }
    }

    fn handle_piece_peer_request(&mut self, peer: DeviceId, piece_ids: Vec<Range<u64>>, response_chan: Sender<HandlePeerResponse<Vec<Option<Vec<u8>>>>>) {
        let pieces = handle_piece_peer_request_helper(&self.state_machine, &piece_ids);

        if let Some(pieces) = pieces {
            // We have the pieces so share it with the peer.
            response_chan.send(Ok(pieces)).expect("TODO");
        } else {
            // We don't have the pieces so tell them to wait.
            let (send_chan, recv_chan) = oneshot::channel();
            response_chan.send(Err(recv_chan)).expect("TODO");

            // Register the wait channel.
            self.piece_subscribers.insert(peer, (piece_ids, send_chan)); // JP: Safe to drop old one?
        }
    }

    fn handle_ecg_sync_request(&mut self, peer: DeviceId, piece_ids: (Vec<Header::HeaderId>, Vec<Header::HeaderId>), response_chan: Sender<HandlePeerResponse<Vec<()>>>) {
        todo!();
    }

    fn handle_received_metadata(&mut self, peer: DeviceId, metadata: MetadataHeader<Hash>) {
        debug!("Recieved metadata from peer ({peer}): {metadata:?}");

        // Mark peer as ready.
        self.update_outgoing_peer_to_ready(&peer);

        // Validate metadata.
        let is_valid = metadata.validate_store_id(self.store_id());
        warn!("TODO: Validate signature.");
        warn!("TODO: Validate type id.");

        if !is_valid {
            // TODO: Penalize/blacklist/disconnect peer?
            warn!("TODO: Peer provided invalid metadata.");
            return;
        }

        // Update state.
        self.state_machine = StateMachine::DownloadingMerkle {
            piece_hashes: vec![None; metadata.piece_count() as usize],
            metadata,
        };

        // Send metadata to any peers that are waiting.
        let subs = std::mem::take(&mut self.metadata_subscribers);
        for (sub_peer, sub) in subs {
            let peer_knows = sub_peer == peer;
            let msg = if peer_knows {
                None
            } else {
                Some(metadata)
            };
            sub.send(msg).expect("TODO");
        }

        // Sync with peer(s). Do this for all commands??
        self.send_sync_requests();
    }

    fn metadata(&self) -> Option<&MetadataHeader<Hash>> {
        match &self.state_machine {
            StateMachine::DownloadingMetadata { .. } => {
                None
            }
            StateMachine::DownloadingMerkle { metadata, .. } => {
                Some(metadata)
            }
            StateMachine::DownloadingInitialState { metadata, .. } => {
                Some(metadata)
            }
            StateMachine::Syncing { metadata, .. } => {
                Some(metadata)
            }
        }
    }

    fn piece_hashes(&self) -> Option<&Vec<Hash>> {
        match &self.state_machine {
            StateMachine::DownloadingMetadata { .. } => {
                None
            }
            StateMachine::DownloadingMerkle { .. } => {
                // We shouldn't share pieces until we've verified them.
                None
            }
            StateMachine::DownloadingInitialState { piece_hashes, .. } => {
                Some(piece_hashes)
            }
            StateMachine::Syncing { piece_hashes, .. } => {
                Some(piece_hashes)
            }
        }
    }

    fn handle_received_merkle_hashes(&mut self, peer: DeviceId, piece_ids: Vec<Range<u64>>, their_piece_hashes: Vec<Hash>) {
        warn!("TODO: Keep track if you received different hashes from different peers.");

        // Mark peer as ready.
        self.update_outgoing_peer_to_ready(&peer);

        let piece_ids: Vec<_> = piece_ids.into_iter().flatten().collect();
        if piece_ids.len() != their_piece_hashes.len() {
            warn!("TODO: Peer provided an invalid response");
            return;
        }

        // Update state.
        let piece_hashes = if let StateMachine::DownloadingMerkle { ref mut piece_hashes, .. } = &mut self.state_machine {
            piece_hashes
        } else {
            // We're not downloading anymore so we're done.
            return;
        };
        piece_ids.into_iter().zip(their_piece_hashes).for_each(|(i, hash)| {
            piece_hashes[i as usize] = Some(hash);
        });

        // Check if we're done.
        let piece_hashes_m: Option<Vec<_>> = piece_hashes.iter().cloned().collect();
        let Some(piece_hashes) = piece_hashes_m else {
            return;
        };

        // Validate piece hashes.
        let is_valid = self.metadata().unwrap().merkle_root == util::merkle_root(&piece_hashes);
        if !is_valid {
            // TODO: Penalize/blacklist/disconnect peer?
            warn!("TODO: Peer provided invalid piece hashes.");
            return;
        }

        // Update state.
        self.state_machine = match self.state_machine {
            StateMachine::DownloadingMerkle { metadata, ..} => {
                let initial_state = vec![None; metadata.piece_count() as usize];
                StateMachine::DownloadingInitialState { metadata, piece_hashes, initial_state }
            }
            _ => unreachable!("We already checked that we're downloading merkle"),
        };

        // Send piece hashes to any peers that are waiting.
        let subs = std::mem::take(&mut self.merkle_subscribers);
        let piece_hashes = self.piece_hashes().expect("Unreachable: We just set the piece hashes");
        for (sub_peer, (piece_ids, sub)) in subs {
            let peer_knows = sub_peer == peer;
            let msg = if peer_knows {
                None
            } else {
                Some(handle_merkle_peer_request_helper(piece_hashes, &piece_ids))
            };
            sub.send(msg).expect("TODO");
        }

        // Sync with peer(s). Do this for all commands??
        self.send_sync_requests();
    }

    fn handle_received_initial_state_pieces(&mut self, peer: DeviceId, piece_ids: Vec<Range<u64>>, their_pieces: Vec<Option<Vec<u8>>>, listeners: &[UnboundedSender<StateUpdate<Header, T>>])
    where
        T: for<'d> Deserialize<'d>,
    {
        // Mark peer as ready.
        self.update_outgoing_peer_to_ready(&peer);

        let piece_ids: Vec<_> = piece_ids.into_iter().flatten().collect();
        if piece_ids.len() != their_pieces.len() {
            warn!("TODO: Peer provided an invalid response");
            return;
        }

        // Update state.
        let (initial_state, piece_hashes) = if let StateMachine::DownloadingInitialState { ref mut initial_state, ref piece_hashes, .. } = &mut self.state_machine {
            (initial_state, piece_hashes)
        } else {
            return;
        };
        piece_ids.into_iter().zip(their_pieces).for_each(|(i, their_piece)| {
            if let Some(their_piece) = their_piece {
                let piece = &mut initial_state[i as usize];
                // Only set piece if it's currently None and if it validates.
                let expected_hash = piece_hashes[i as usize];
                if piece.is_none() && util::validate_piece(&their_piece, &expected_hash){
                    *piece = Some(their_piece);

                    // TODO: Credit peer with this piece.
                }
            }
        });

        // Check if we're done. Exit if we're not.
        if initial_state.iter().any(|o| o.is_none()) {
            return;
        }

        // Update state.
        replace_with_or_abort(&mut self.state_machine, |sm| {
            match sm {
                StateMachine::DownloadingInitialState { metadata, piece_hashes, initial_state } => {
                    let ecg_state = ecg::State::new();
                    let initial_state: Vec<u8> = initial_state.into_iter().flatten().flatten().collect::<Vec<u8>>();
                    let Ok(latest_state) = serde_cbor::de::from_slice::<T>(&initial_state) else {
                        todo!("TODO: The store is invalid. Initial state does not parse.");
                    };
                    let decrypted_state = DecryptedState {
                        latest_state,
                        latest_headers: BTreeSet::new(),
                    };
                    StateMachine::Syncing { metadata, piece_hashes, initial_state, ecg_state, decrypted_state, }
                }
                _ => unreachable!("We already checked that we're downloading the initial state"),
            }
        });

        // Update listeners.
        let StateMachine::Syncing { ecg_state, decrypted_state, .. } = &self.state_machine else {
            unreachable!("We just set our state to syncing")
        };
        update_listeners(&listeners, &decrypted_state.latest_state, &ecg_state);

        // Send pieces to any peers that are waiting.
        let subs = std::mem::take(&mut self.piece_subscribers);
        for (_sub_peer, (piece_ids, sub)) in subs {
            // JP: Do we need to keep sub_peer here?
            let msg = handle_piece_peer_request_helper(&self.state_machine, &piece_ids).expect("Unreachable: We just set our state to syncing");
            sub.send(Some(msg)).expect("TODO");
        }

        // Sync with peer(s). Do this for all commands??
        self.send_sync_requests();
    }

    fn handle_received_updated_meet(&mut self, peer: DeviceId, meet: Vec<Header::HeaderId>) {
        // Mark peer as ready.
        self.update_outgoing_peer_to_ready(&peer);

        let Some(info) = self.peers.get_mut(&peer) else {
            error!("Invariant violated. Attempted to update an unknown peer: {}", peer);
            panic!();
        };

        // JP: Join current meet with new meet?
        debug!("JP: Join current meet with new meet?");
        info.ecg_status.meet = meet;
        info.ecg_status.meet_needs_update = false;
    }
}

fn update_listeners<Header: ecg::ECGHeader + Clone, T: CRDT + Clone>(listeners: &[UnboundedSender<StateUpdate<Header, T>>], latest_state: &T, ecg_state: &ecg::State<Header, T>) {
    for l in listeners {
        let snapshot: StateUpdate<Header, T> = StateUpdate::Snapshot {
            snapshot: latest_state.clone(),
            ecg_state: ecg_state.clone(),
        };
        l.send(snapshot).expect("TODO");
    }
}

// JP: Or should Odyssey own this/peers?
/// Manage peers by ranking them, randomize, potentially connecting to some of them, etc.
async fn manage_peers<OT: OdysseyType, T: CRDT<Time = OT::Time> + Clone + Send + 'static>(
    store: &mut State<OT::StoreId, OT::ECGHeader, T, OT::Hash>,
    shared_state: &SharedState<OT::StoreId>,
    send_commands: &UnboundedSender<UntypedStoreCommand<OT::Hash, <OT::ECGHeader as ECGHeader>::HeaderId, OT::ECGHeader>>,
)
where
    T::Op: Serialize,
    OT::ECGHeader: Clone + Serialize + for<'d> Deserialize<'d>,
    <OT::ECGHeader as ECGHeader>::HeaderId: Serialize + for<'d> Deserialize<'d> + Send,
    //OT::ECGHeader<T>::HeaderId : Send,
    //T: Send,
{
    // For now, sync with all (connected?) peers.
    // Don't connect to peers we're already syncing with.
    let peers: Vec<_> = store.peers.iter().filter(|p| p.1.outgoing_status.is_known()).collect();
    let peers: Vec<_> = {
        // Acquire lock on shared state.
        let peer_states = shared_state.peer_state.read().await;
        peers.into_iter().filter_map(|(peer_id, _)| {
            let chan = peer_states.get(peer_id)?;
            Some((*peer_id, chan.clone()))
        }).collect()
    };
    for (peer_id, command_chan) in peers {
        // Mark task as initializing.
        store.update_peer_to_initializing_outgoing(&peer_id);

        // Create closure that spawns task to sync store with peer.
        let send_commands = send_commands.clone();
        let spawn_task = Box::new(move |_party, stream_id, sender, receiver| {
            // Create miniprotocol
            // Spawn task that syncs store with peer.
            // JP: Should run with initiative?
            tokio::spawn(async move {
                // Tell store we're running and send it our channel.
                let (send_peer, recv_peer) =
                    tokio::sync::mpsc::unbounded_channel::<StoreSyncCommand<<OT::ECGHeader as ECGHeader>::HeaderId, OT::ECGHeader>>();

                let register_cmd = UntypedStoreCommand::RegisterOutgoingPeerSyncing {
                    peer: peer_id,
                    send_peer,
                };
                send_commands.send(register_cmd).expect("TODO");

                // Start miniprotocol as server.
                let mp = StoreSync::<OT::Hash, _, _>::new_server(peer_id, recv_peer, send_commands);
                run_miniprotocol_async::<_, OT>(mp, false, stream_id, sender, receiver).await;

                debug!("Store sync with peer (with initiative) exited.")

                // JP: This requires StorePeer protocols in both directions (if both sides want updates from the other party).
                // This has the downside that we may run ECG sync in both directions.. Does it make sense to store StorePeer state in a shared Arc<RWLock>?
                // Separate thread for SCSync?
                // TODO: Setup in both directions, remove initialized check.
                //
                // In the store task, sync state:
                // Based on status,
                //   if we're still downloading the metadata, request the peer to send it 
                //   if we are downloading the initial state, send a download request for a piece(s) we need. How do we do timeouts here though? Perhaps in the StorePeer task?
                //   if we're syncing:
                //      run ECG sync to find the meet
                //      Request all operations after the meet
            })
        });

        // Send request to peer's manager for stream.
        let store_id = store.store_id();
        let cmd = PeerManagerCommand::RequestStoreSync {
            store_id,
            spawn_task,
        };
        command_chan.send(cmd).expect("TODO");
    }
}


/// Run the handler that owns this store and manages its state. This handler is typically run in
/// its own tokio thread.
pub(crate) async fn run_handler<OT: OdysseyType, T>(
    mut store: State<OT::StoreId, OT::ECGHeader, T, OT::Hash>,
    mut recv_commands: UnboundedReceiver<StoreCommand<OT::ECGHeader, OT::ECGBody, T>>,
    send_commands_untyped: UnboundedSender<UntypedStoreCommand<OT::Hash, <OT::ECGHeader as ECGHeader>::HeaderId, OT::ECGHeader>>,
    mut recv_commands_untyped: UnboundedReceiver<UntypedStoreCommand<OT::Hash, <OT::ECGHeader as ECGHeader>::HeaderId, OT::ECGHeader>>,
    shared_state: SharedState<OT::StoreId>,
)
where
    <OT as OdysseyType>::ECGHeader: Send + Clone + Serialize + for<'d> Deserialize<'d> + 'static,
    // <<OT as OdysseyType>::ECGHeader as ECGHeader>::Body: ECGBody<T> + Send,
    <OT as OdysseyType>::ECGBody: ECGBody<T, Header = OT::ECGHeader> + Send,
    <<OT as OdysseyType>::ECGHeader as ECGHeader>::HeaderId: Send + Serialize + for<'d> Deserialize<'d>,
    T::Op: Serialize,
    T: CRDT<Time = OT::Time> + Clone + Send + 'static + for<'d> Deserialize<'d>,
{
    let mut listeners: Vec<UnboundedSender<StateUpdate<OT::ECGHeader, T>>> = vec![];

    // TODO: Check when done
    loop {
        tokio::select! {
            cmd_m = recv_commands.recv() => {
                let Some(cmd) = cmd_m else {
                    error!("Failed to receive StoreCommand");
                    return;
                };

                // // Rank and connect to a few peers.
                // manage_peers::<OT,T>(&mut store, &shared_state).await;

                match cmd {
                    StoreCommand::Apply {
                        operation_header,
                        operation_body,
                    } => {
                        store.state_machine = match store.state_machine {
                            // StateMachine::DownloadingMetadata { store_id } => {
                            //     // JP: Should we ever apply an operation if we're still downloading the store??
                            //     warn!("Is this unreachable?");

                            //     // Rank and connect to a few peers.
                            //     manage_peers::<OT,T>(&mut store, &shared_state).await;

                            //     StateMachine::DownloadingMetadata { store_id }
                            // }
                            // StateMachine::DownloadingMerkle { metadata, piece_hashes } => {
                            //     // JP: Should we ever apply an operation if we're still downloading the store??
                            //     warn!("Is this unreachable?");

                            //     // Rank and connect to a few peers.
                            //     manage_peers::<OT,T>(&mut store, &shared_state).await;

                            //     StateMachine::DownloadingMerkle { metadata, piece_hashes }
                            // }
                            // StateMachine::DownloadingInitialState { metadata, piece_hashes, initial_state } => {
                            //     // JP: Should we ever apply an operation if we're still downloading the store??
                            //     warn!("Is this unreachable?");

                            //     // Rank and connect to a few peers.
                            //     manage_peers::<OT,T>(&mut store, &shared_state).await;

                            //     StateMachine::DownloadingInitialState { metadata, piece_hashes, initial_state }
                            // }
                            StateMachine::Syncing { metadata, piece_hashes, initial_state , ecg_state, decrypted_state } => {
                                let mut ecg_state = ecg_state;
                                let mut decrypted_state = decrypted_state;

                                // Update ECG state.
                                let success = ecg_state.insert_header(operation_header.clone());
                                if !success {
                                    todo!("Invalid header"); // : {:?}", operation_header);
                                }

                                // Update state.
                                // TODO: Get new time?.. Or take it as an argument
                                // Operation ID/time is function of tips, current operation, ...? How do we
                                // do batching? (HeaderId(h) | Self, Index(u8)) ? This requires having all
                                // the batched operations?
                                let causal_state = OT::to_causal_state(&ecg_state);
                                for (time, operation) in
                                    operation_body.zip_operations_with_time(&operation_header)
                                {
                                    decrypted_state.latest_state = decrypted_state.latest_state.apply(causal_state, time, operation);
                                }

                                // Send state to subscribers.
                                update_listeners(&listeners, &decrypted_state.latest_state, &ecg_state);

                                StateMachine::Syncing { metadata, piece_hashes, initial_state, ecg_state, decrypted_state }
                            }
                            _ => {
                                warn!("JP: Does this ever happen?");
                                store.state_machine
                            }
                        };
                    }
                    StoreCommand::SubscribeState { send_state } => {
                        // Send current state.
                        let snapshot = match &store.state_machine {
                            StateMachine::DownloadingMetadata { .. } => {
                                StateUpdate::Downloading { percent: 0 }
                            }
                            StateMachine::DownloadingMerkle { .. } => {
                                StateUpdate::Downloading { percent: 0 }
                            }
                            StateMachine::DownloadingInitialState { metadata, initial_state, .. } => {
                                let percent = if metadata.initial_state_size == 0 {
                                    0
                                } else {
                                    let downloaded = initial_state.iter().filter(|p| p.is_some()).count() as u64;
                                    100 * downloaded * PIECE_SIZE / metadata.initial_state_size
                                };
                                StateUpdate::Downloading { percent }
                            }
                            StateMachine::Syncing { ref ecg_state, ref decrypted_state, .. } => {
                                StateUpdate::Snapshot {
                                    snapshot: decrypted_state.latest_state.clone(),
                                    ecg_state: ecg_state.clone(),
                                }
                            }
                        };
                        send_state.send(snapshot).expect("TODO");

                        // Register this subscriber.
                        listeners.push(send_state);
                    }
                }
            }
            cmd_m = recv_commands_untyped.recv() => {
                let Some(cmd) = cmd_m else {
                    error!("Failed to receive UntypedStoreCommand");
                    return;
                };
                match cmd {
                    // Called when:
                    // - Peer manager threads have this thread as a mutual store.
                    UntypedStoreCommand::RegisterPeers { peers } => {
                        debug!("Received UntypedStoreCommand::RegisterPeers: {:?}", peers);

                        // Add peer to known peers.
                        for peer in peers {
                            store.insert_known_peer(peer);
                        }

                        debug!("Peer statuses: {:?}", store.peers);

                        // Spawn sync threads for each shared store.
                        // TODO: Only do this if server?
                        // Check if we already are syncing these.
                        manage_peers::<OT,T>(&mut store, &shared_state, &send_commands_untyped).await;
                    }
                    // Sets up task to respond to a request to sync this store from a peer (without initiative).
                    // Called when:
                    // - The peer requests we sync this store with them
                    UntypedStoreCommand::SyncWithPeer { peer, response_chan } => {
                        debug!("Received UntypedStoreCommand::SyncWithPeer: {:?}", peer);

                        // Insert peer as known if we don't know them (since they're requesting the store).
                        store.insert_known_peer(peer);

                        let response = {
                            // Check if already syncing with this peer. (JP: What if they're both already "Initializing"? Potential race condition where they don't sync)
                            if let Some(status) = store.peers.get(&peer) {
                                if status.incoming_status.is_known() {
                                    // Mark task as initializing.
                                    store.update_peer_to_initializing_incoming(&peer);

                                    // Create closure that spawns task to sync store with peer.
                                    let send_commands_untyped = send_commands_untyped.clone();
                                    let spawn_task: Box<SpawnMultiplexerTask> = Box::new(move |party, stream_id, sender, receiver| {
                                        // Create miniprotocol
                                        // Spawn task that syncs store with peer.
                                        // JP: Should run without initiative so that other peer can setup their handler?
                                        tokio::spawn(async move {
                                            debug!("Sync with peer (without initiative).");

                                            // Tell store we're running.
                                            let register_cmd = UntypedStoreCommand::RegisterIncomingPeerSyncing {
                                                peer,
                                            };
                                            send_commands_untyped.send(register_cmd).expect("TODO");

                                            // Start miniprotocol as client.
                                            let mp = StoreSync::<OT::Hash, _, _>::new_client(peer, send_commands_untyped);
                                            run_miniprotocol_async::<_, OT>(mp, true, stream_id, sender, receiver).await;
                                            debug!("Store sync with peer (without initiative) exited.")
                                        })
                                    });
                                    Some(spawn_task)
                                } else {
                                    debug!("Store is already running");
                                    None
                                }
                            } else {
                                unreachable!("Don't know this peer.");
                                // JP: This should be impossible now.
                                None
                            }
                        };

                        response_chan.send(response).or(Err(())).expect("TODO");
                    }
                    UntypedStoreCommand::RegisterOutgoingPeerSyncing{ peer, send_peer } => {
                        // JP: Maybe send_peer actually isn't needed??? We could construct oneshots???
                        // Update peer's state to syncing and register channel.
                        let outgoing_status = OutgoingPeerStatus {
                            sender_peer: send_peer,
                            is_outstanding: false,
                        };
                        store.update_peer_to_syncing_outgoing(&peer, outgoing_status);

                        // Sync with peer(s). Do this for all commands??
                        store.send_sync_requests();
                    }
                    UntypedStoreCommand::RegisterIncomingPeerSyncing{ peer } => {
                        // JP: Maybe this actually isn't needed??? We could construct oneshots for every request..

                        // Update peer's state to syncing and register channel.
                        store.update_peer_to_syncing_incoming(&peer);
                    }
                    UntypedStoreCommand::HandleMetadataPeerRequest(HandlePeerRequest { peer, request, response_chan }) => {
                        store.handle_metadata_peer_request(peer, response_chan);
                    }
                    UntypedStoreCommand::HandleMerklePeerRequest(HandlePeerRequest { peer, request, response_chan }) => {
                        store.handle_merkle_peer_request(peer, request, response_chan);
                    }
                    UntypedStoreCommand::HandlePiecePeerRequest(HandlePeerRequest { peer, request, response_chan }) => {
                        store.handle_piece_peer_request(peer, request, response_chan);
                    }
                    UntypedStoreCommand::HandleECGSyncRequest(HandlePeerRequest { peer, request, response_chan }) => {
                        store.handle_ecg_sync_request(peer, request, response_chan);
                    }
                    UntypedStoreCommand::ReceivedMetadata { peer, metadata } => {
                        store.handle_received_metadata(peer, metadata);
                    }
                    UntypedStoreCommand::ReceivedMerkleHashes { peer, ranges, pieces } => {
                        store.handle_received_merkle_hashes(peer, ranges, pieces);
                    }
                    UntypedStoreCommand::ReceivedInitialStatePieces { peer, ranges, pieces } => {
                        store.handle_received_initial_state_pieces(peer, ranges, pieces, &listeners);
                    }
                    UntypedStoreCommand::ReceivedUpdatedMeet { peer, meet } => {
                        store.handle_received_updated_meet(peer, meet);
                    }
                }
            }
        }
    }
    debug!("Store thread exiting.");
}

pub(crate) enum StoreCommand<Header: ECGHeader, Body, T> {
    Apply {
        operation_header: Header,     // <Hash, T>,
        operation_body: Body, // <Hash, T>,
    },
    // TODO: Support unsubscribe.
    SubscribeState {
        send_state: UnboundedSender<StateUpdate<Header, T>>,
    },
}

pub enum StateUpdate<Header: ECGHeader, T> {
    Downloading {
        // Percent of the state that we've downloaded (0 - 100).
        percent: u64,
    },
    Snapshot {
        snapshot: T,
        ecg_state: ecg::State<Header, T>,
        // TODO: ECG DAG
    },
}

// trait UntypedCRDT: CRDT<Op = dyn Any, Time = dyn Any> {} // Any + Sized + 'static +  
// trait UntypedECGHeader: ECGHeader<dyn Any> {} // Any + Sized + 'static + 
// 
// struct UntypedStoreCommand(StoreCommand<dyn UntypedECGHeader, dyn UntypedCRDT>);
// struct UntypedStoreCommand(StoreCommand<dyn UntypedECGHeader, dyn Any>);

type HandlePeerResponse<Response> = Result<
        Response,
        oneshot::Receiver<Option<Response>>,
    >;

/// Untyped variant of `StoreCommand` since existentials don't work.
// #[derive(Debug)]
pub(crate) enum UntypedStoreCommand<Hash, HeaderId, Header> {
    /// Register the discovered peers.
    RegisterPeers {
        peers: Vec<DeviceId>,
    },
    /// Request store to sync with peer. Store can refuse.
    SyncWithPeer {
        peer: DeviceId,
        response_chan: oneshot::Sender<Option<Box<SpawnMultiplexerTask>>>,
    },
    RegisterOutgoingPeerSyncing {
        peer: DeviceId,
        send_peer: UnboundedSender<StoreSyncCommand<HeaderId, Header>>,
    },
    HandleMetadataPeerRequest(HandlePeerRequest<(), v0::MetadataHeader<Hash>>),
    HandleMerklePeerRequest(HandlePeerRequest<Vec<Range<u64>>, Vec<Hash>>),
    HandlePiecePeerRequest(HandlePeerRequest<Vec<Range<u64>>, Vec<Option<Vec<u8>>>>),
    HandleECGSyncRequest(HandlePeerRequest<(Vec<HeaderId>, Vec<HeaderId>), Vec<()>>),
    RegisterIncomingPeerSyncing {
        peer: DeviceId,
    },
    ReceivedMetadata {
        peer: DeviceId,
        metadata: MetadataHeader<Hash>,
    },
    ReceivedMerkleHashes {
        peer: DeviceId,
        ranges: Vec<Range<u64>>,
        pieces: Vec<Hash>,
    },
    ReceivedInitialStatePieces {
        peer: DeviceId,
        ranges: Vec<Range<u64>>,
        pieces: Vec<Option<Vec<u8>>>,
    },
    ReceivedUpdatedMeet {
        peer: DeviceId,
        meet: Vec<HeaderId>,
    },
}

pub(crate) struct HandlePeerRequest<Request, Response> {
    pub(crate) peer: DeviceId,
    pub(crate) request: Request, // MsgStoreSyncRequest,
    /// Return either the result or a channel to wait on for the response.
    pub(crate) response_chan: oneshot::Sender<HandlePeerResponse<Response>>,
}

fn handle_merkle_peer_request_helper<H: Copy>(piece_hashes: &[H], piece_ids: &[Range<u64>]) -> Vec<H> {
    handle_peer_request_range_helper(piece_hashes, piece_ids)
}

fn handle_piece_peer_request_helper<StoreId, Header: ecg::ECGHeader, T: CRDT, Hash>(state_machine: &StateMachine<StoreId, Header, T, Hash>, piece_ids: &[Range<u64>]) -> Option<Vec<Option<Vec<u8>>>> {
    // TODO: Can we avoid these clones?
    match state_machine {
        StateMachine::DownloadingMetadata { .. } => None,
        StateMachine::DownloadingMerkle { .. } => None,
        StateMachine::DownloadingInitialState { initial_state, .. } => {
            Some(handle_peer_request_range_helper(&initial_state, piece_ids))
        }
        StateMachine::Syncing { initial_state, .. } => {
            let pieces: Vec<_> = piece_ids.iter().cloned().flatten().map(|i| {
                warn!("TODO: Properly handle invalid requests"); // Return None if i >= metadata.piece_count()?
                let start: usize = (i * PIECE_SIZE) as usize;
                let end = std::cmp::min(((i + 1) * PIECE_SIZE) as usize, initial_state.len());
                Some(initial_state[start..end].to_vec())
            }).collect();
            Some(pieces)
        }
    }
}

fn handle_peer_request_range_helper<T: Clone>(piece_hashes: &[T], piece_ids: &[Range<u64>]) -> Vec<T> {
    warn!("TODO: check ranges are in bounds or return error");
    let hashes: Vec<_> = piece_ids.iter().cloned().flatten().map(|i| piece_hashes.get(i as usize).expect("TODO: Properly handle invalid requests").clone()).collect();
    hashes
}

