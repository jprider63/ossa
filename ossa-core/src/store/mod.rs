use itertools::Itertools;
use ossa_crdt::CRDT;
use ossa_typeable::Typeable;
use rand::{seq::SliceRandom as _, thread_rng};
use replace_with::replace_with_or_abort;
use serde::{Deserialize, Serialize};
use tokio::sync::watch;
use std::fmt::{Debug, Display};
use std::marker::PhantomData;
use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Range,
};
use tokio::sync::{
    mpsc::{UnboundedReceiver, UnboundedSender},
    oneshot::{self, Sender},
};
use tracing::{debug, error, warn};

use crate::protocol::store_bft_sync::v0::{BFTSyncResponse, StoreBFTSync, StoreBFTSyncCommand};
use crate::store::bft::{BFTState, SCDT};
use crate::store::v0::BLOCK_SIZE;
use crate::time::ConcretizeTime;
use crate::util::merkle_tree::{MerkleTree, Potential};
use crate::{
    auth::DeviceId,
    core::{OssaType, SharedState},
    network::multiplexer::{run_miniprotocol_async, SpawnMultiplexerTask},
    protocol::{
        manager::v0::PeerManagerCommand,
        store_sc_dag::v0::{StoreDAGSync, StoreSCGSyncCommand},
        store_peer::v0::{StoreSync, StoreSyncCommand},
    },
    store::{
        dag::{DAGBody, DAGHeader, RawDAGBody},
        v0::{BLOCK_REQUEST_LIMIT, MERKLE_REQUEST_LIMIT},
    },
    util::{self, compress_consecutive_into_ranges},
};

pub mod bft;
pub mod dag;
pub mod v0; // TODO: Move this to network::protocol

pub use v0::{MetadataBody, MetadataHeader, Nonce};

// TODO: Concretize StoreId to Sha256Hash.
/// A typed reference to another store.
#[derive(Serialize, Deserialize, Typeable, Debug)] // , PartialEq, Eq, PartialOrd, Ord)]
pub struct StoreRef<StoreId, S, C> {
    store_id: StoreId,
    phantom: PhantomData<fn(S, C)>,
}

impl<StoreId: Display, S, C> Display for StoreRef<StoreId, S, C> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.store_id.fmt(f)
    }
}

impl<StoreId, S, C> StoreRef<StoreId, S, C> {
    pub fn new(store_id: StoreId) -> Self {
        Self { store_id, phantom: PhantomData }
    }
}

impl<StoreId: Clone, S, C> Clone for StoreRef<StoreId, S, C> {
    fn clone(&self) -> Self {
        Self { store_id: self.store_id.clone(), phantom: PhantomData }
    }
}

impl<StoreId: Ord, S, C> PartialOrd for StoreRef<StoreId, S, C> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl<StoreId: Ord, S, C> Ord for StoreRef<StoreId, S, C> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.store_id.cmp(&other.store_id)
    }
}

impl<StoreId: PartialEq, S, C> PartialEq for StoreRef<StoreId, S, C> {
    fn eq(&self, other: &Self) -> bool {
        self.store_id == other.store_id
    }
}

impl<StoreId: Eq, S, C> Eq for StoreRef<StoreId, S, C> {}

pub struct State<StoreId, SHeader: dag::DAGHeader, THeader: dag::DAGHeader, S, T: CRDT, Hash> {
    // Peers that also have this store (that we are potentially connected to?).
    peers: BTreeMap<DeviceId, PeerInfo<SHeader::HeaderId, SHeader, THeader::HeaderId, THeader>>, // BTreeSet<DeviceId>,
    state_machine: StateMachine<StoreId, SHeader, THeader, S, T, Hash>,
    metadata_subscribers: BTreeMap<DeviceId, oneshot::Sender<Option<v0::MetadataHeader<Hash>>>>,
    merkle_subscribers: BTreeMap<DeviceId, (Vec<Range<u64>>, oneshot::Sender<Option<Vec<Hash>>>)>,
    block_subscribers: BTreeMap<
        DeviceId,
        (
            Vec<Range<u64>>,
            oneshot::Sender<Option<Vec<Option<Vec<u8>>>>>,
        ),
    >,
    ecg_subscribers:
        BTreeMap<DeviceId, oneshot::Sender<dag::UntypedState<THeader::HeaderId, THeader>>>,
    scg_subscribers:
        BTreeMap<DeviceId, oneshot::Sender<dag::UntypedState<SHeader::HeaderId, SHeader>>>,
    // listeners: Vec<UnboundedSender<StateUpdate<Header, T>>>,
    /// State for BFT sync / consensus.
    pub(crate) bft_state: watch::Sender<BFTState<StoreId, SHeader::HeaderId>>,
}

// States are:
// - Initializing - Setting up the thread that owns the store (not defined here).
// - DownloadingMetadata - Don't have the header so we're downloading it.
// - Syncing - Have the header and syncing updates between peers.
pub(crate) enum StateMachine<
    StoreId,
    SHeader: dag::DAGHeader,
    THeader: dag::DAGHeader,
    S,
    T: CRDT,
    Hash,
> {
    DownloadingMetadata {
        store_id: StoreId,
    },
    /// Invariant: At least one partial_merkle_tree node is None.
    DownloadingMerkle {
        metadata: MetadataHeader<Hash>,
        partial_merkle_tree: MerkleTree<Potential<Hash>>,
    },
    /// Invariant: At least one initial_state block is None.
    DownloadingInitialState {
        metadata: MetadataHeader<Hash>,
        merkle_tree: MerkleTree<Hash>,
        initial_state: Vec<Option<Vec<u8>>>,
    },
    Syncing {
        metadata: MetadataHeader<Hash>,
        merkle_tree: MerkleTree<Hash>,
        initial_state: Vec<u8>, // Or just T?
        ecg_state: dag::State<THeader, T>,
        sc_state: bft::State<SHeader, S>,
        // TODO: S::Op
        decrypted_state: DecryptedState<SHeader, THeader, T>, // Temporary
                                                              // decrypted_state: Option<DecryptedState<Header, T>>, // JP: Is this actually used?
                                                              // Does it make sense?
    },
}

pub struct DecryptedState<SHeader: dag::DAGHeader, THeader: dag::DAGHeader, T: CRDT> {
    /// Latest ECG application state we've seen.
    latest_ec_state: T,

    /// Headers corresponding to the latest ECG application state.
    // TODO: Remove this.
    latest_headers: BTreeSet<THeader::HeaderId>,

    sc_operations: BTreeMap<SHeader::HeaderId, ()>, // TODO: SHeader, S::Op
}

/// Information about a peer.
#[derive(Debug)]
struct PeerInfo<SHeaderId, SHeader, THeaderId, THeader> {
    ecg_status: PeerProtocolStatus<StoreSyncCommand<THeaderId, THeader>>, // ECGHeader?
    scg_status: PeerProtocolStatus<StoreSCGSyncCommand<SHeaderId, SHeader>>, // SCGHeader?
                                                                          // TODO: sc_status: PeerProtocolStatus<HeaderId, Header>,
    bft_status: PeerProtocolStatus<StoreBFTSyncCommand>,
}

#[derive(Debug)]
struct PeerProtocolStatus<CommandType> {
    /// Status of incoming sync status from peer.
    incoming_status: PeerStatus<()>,
    /// Status of outgoing sync status to peer.
    outgoing_status: PeerStatus<OutgoingPeerStatus<CommandType>>,
    // ecg_status: ECGStatus<HeaderId>,
}

// #[derive(Clone, Debug)]
// /// Information about a peer's ECG status.
// pub(crate) struct ECGStatus<HeaderId> {
//     /// Greatest common ancestor between our ECG graphs.
//     pub(crate) meet: Vec<HeaderId>,
//     /// Whether we need to update the meet between us and this peer.
//     pub(crate) meet_needs_update: bool,
//     // JP: Track their_tip?
// }

impl<SHeaderId, SHeader, THeaderId, THeader> PeerInfo<SHeaderId, SHeader, THeaderId, THeader> {
    /// Checks if the peer is ready for an ECG sync request.
    /// This means the peer is syncing and does not have an outstanding request.
    fn is_ready_for_sync<CommandType>(
        &self,
        f: fn(
            &PeerInfo<SHeaderId, SHeader, THeaderId, THeader>,
        ) -> &PeerProtocolStatus<CommandType>,
    ) -> bool {
        if let PeerStatus::Syncing(s) = &f(self).outgoing_status {
            !s.is_outstanding
        } else {
            false
        }
    }
}

#[derive(Debug)]
/// Outgoing information about a syncing peer.
struct OutgoingPeerStatus<CommandType> {
    /// Sender channel for requests to the outgoing peer store.
    sender_peer: UnboundedSender<CommandType>,
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

impl<
        StoreId: Copy + Eq,
        SHeader: dag::DAGHeader + Clone + Debug,
        THeader: dag::DAGHeader + Clone + Debug,
        S: Clone,
        T: CRDT + Clone,
        Hash: util::Hash + Debug + Into<StoreId>,
    > State<StoreId, SHeader, THeader, S, T, Hash>
{
    /// Initialize a new store with the given state. This initializes the header, including
    /// generating a random nonce.
    pub fn new_syncing(
        initial_sc_state: S,
        initial_ec_state: T,
    ) -> State<StoreId, SHeader, THeader, S, T, Hash>
    where
        S: Serialize + Typeable,
        T: Serialize + Typeable,
    {
        let init_body = MetadataBody::new(&initial_sc_state, &initial_ec_state);
        debug!("Initialized body: {:?}", init_body);
        let store_header = MetadataHeader::generate::<S, T>(&init_body);
        let store_id = store_header.store_id();
        let decrypted_state = DecryptedState {
            latest_ec_state: initial_ec_state,
            latest_headers: BTreeSet::new(),
            sc_operations: BTreeMap::new(),
        };

        let (merkle_tree, initial_state) = init_body.build();

        let state_machine = StateMachine::Syncing {
            metadata: store_header,
            merkle_tree,
            initial_state,
            ecg_state: dag::State::new(),
            sc_state: bft::State::new(initial_sc_state),
            decrypted_state, // : Some(decrypted_state),
        };
        State {
            peers: BTreeMap::new(),
            state_machine,
            metadata_subscribers: BTreeMap::new(),
            merkle_subscribers: BTreeMap::new(),
            block_subscribers: BTreeMap::new(),
            ecg_subscribers: BTreeMap::new(),
            scg_subscribers: BTreeMap::new(),
            bft_state: watch::Sender::new(BFTState::new(store_id)),
        }
    }

    /// Create a new store with the given store id that is downloading the store's header.
    pub(crate) fn new_downloading(store_id: StoreId) -> Self {
        let state_machine = StateMachine::DownloadingMetadata { store_id };

        State {
            peers: BTreeMap::new(),
            state_machine,
            metadata_subscribers: BTreeMap::new(),
            merkle_subscribers: BTreeMap::new(),
            block_subscribers: BTreeMap::new(),
            ecg_subscribers: BTreeMap::new(),
            scg_subscribers: BTreeMap::new(),
            bft_state: watch::Sender::new(BFTState::new(store_id)),
        }
    }

    pub fn store_id(&self) -> StoreId {
        match &self.state_machine {
            StateMachine::DownloadingMetadata { store_id } => *store_id,
            StateMachine::DownloadingMerkle { metadata, .. } => metadata.store_id(),
            StateMachine::DownloadingInitialState { metadata, .. } => metadata.store_id(),
            StateMachine::Syncing { metadata, .. } => metadata.store_id(),
        }
    }

    /// Insert a peer as known if its status isn't already tracked by the store.
    fn insert_known_peer(&mut self, peer: DeviceId) {
        // let ecg_status = ECGStatus { meet: vec![], meet_needs_update: true };
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
            .or_insert(PeerInfo {
                ecg_status: PeerProtocolStatus {
                    incoming_status: PeerStatus::Known,
                    outgoing_status: PeerStatus::Known,
                },
                scg_status: PeerProtocolStatus {
                    incoming_status: PeerStatus::Known,
                    outgoing_status: PeerStatus::Known,
                },
                bft_status: PeerProtocolStatus {
                    incoming_status: PeerStatus::Known,
                    outgoing_status: PeerStatus::Known,
                }
            }); // , ecg_status});
    }

    /// Helper to update a known peer to initializing.
    fn update_peer_to_initializing<A>(
        &mut self,
        peer: &DeviceId,
        direction_lambda: fn(
            &mut PeerInfo<SHeader::HeaderId, SHeader, THeader::HeaderId, THeader>,
        ) -> &mut PeerStatus<A>,
    ) where
        A: Debug,
    {
        let Some(info) = self.peers.get_mut(peer) else {
            error!(
                "Invariant violated. Attempted to initialize an unknown peer: {}",
                peer
            );
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

    /// Update a known peer's outgoing (ECG + SCG + BFT) status to initializing.
    fn update_peer_to_initializing_outgoing(&mut self, peer: &DeviceId) {
        self.update_peer_to_initializing(peer, |info| &mut info.ecg_status.outgoing_status);
        self.update_peer_to_initializing(peer, |info| &mut info.scg_status.outgoing_status);
        self.update_peer_to_initializing(peer, |info| &mut info.bft_status.outgoing_status);
    }

    /// Update a known peer's incoming (ECG + SCG + BFT) status to initializing.
    fn update_peer_to_initializing_incoming(&mut self, peer: &DeviceId) {
        self.update_peer_to_initializing(peer, |info| &mut info.ecg_status.incoming_status);
        self.update_peer_to_initializing(peer, |info| &mut info.scg_status.incoming_status);
        self.update_peer_to_initializing(peer, |info| &mut info.bft_status.incoming_status);
    }

    /// Helper to update a known peer to syncing.
    fn update_peer_to_syncing<A>(
        &mut self,
        peer: &DeviceId,
        direction_lambda: fn(
            &mut PeerInfo<SHeader::HeaderId, SHeader, THeader::HeaderId, THeader>,
        ) -> &mut PeerStatus<A>,
        sender_m: A,
    ) where
        A: Debug,
    {
        let Some(info) = self.peers.get_mut(peer) else {
            error!(
                "Invariant violated. Attempted to initialize an unknown peer: {}",
                peer
            );
            panic!();
        };
        let status = direction_lambda(info);
        match status {
            PeerStatus::Initializing => {
                *status = PeerStatus::Syncing(sender_m);
            }
            PeerStatus::Known => {
                error!(
                    "Invariant violated. Attempted to initialize a known peer: {} - {:?}",
                    peer, status
                );
                panic!();
            }
            PeerStatus::Syncing(_) => {
                error!(
                    "Invariant violated. Attempted to sync an already syncing peer: {} - {:?}",
                    peer, status
                );
                panic!();
            }
        }
    }

    fn update_peer_ecg_to_syncing_incoming(&mut self, peer: &DeviceId) {
        self.update_peer_to_syncing(peer, |info| &mut info.ecg_status.incoming_status, ());
    }

    fn update_peer_scg_to_syncing_incoming(&mut self, peer: &DeviceId) {
        self.update_peer_to_syncing(peer, |info| &mut info.scg_status.incoming_status, ());
    }

    fn update_peer_ecg_to_syncing_outgoing(
        &mut self,
        peer: &DeviceId,
        sender: OutgoingPeerStatus<StoreSyncCommand<THeader::HeaderId, THeader>>,
    ) {
        self.update_peer_to_syncing(peer, |info| &mut info.ecg_status.outgoing_status, sender);
    }

    fn update_peer_scg_to_syncing_outgoing(
        &mut self,
        peer: &DeviceId,
        sender: OutgoingPeerStatus<StoreSCGSyncCommand<SHeader::HeaderId, SHeader>>,
    ) {
        self.update_peer_to_syncing(peer, |info| &mut info.scg_status.outgoing_status, sender);
    }

    fn update_peer_bft_to_syncing_outgoing(
        &mut self,
        peer: &DeviceId,
        sender: OutgoingPeerStatus<StoreBFTSyncCommand>,
    ) {
        self.update_peer_to_syncing(peer, |info| &mut info.bft_status.outgoing_status, sender);
    }

    fn update_outgoing_peer_to_ready_helper<CommandType>(
        &mut self,
        peer: &DeviceId,
        f: fn(
            &mut PeerInfo<SHeader::HeaderId, SHeader, THeader::HeaderId, THeader>,
        ) -> &mut PeerProtocolStatus<CommandType>,
    ) where
        CommandType: Debug,
    {
        let Some(info) = self.peers.get_mut(peer) else {
            error!(
                "Invariant violated. Attempted to update an unknown peer: {}",
                peer
            );
            panic!();
        };
        let status = f(info);
        match status.outgoing_status {
            PeerStatus::Initializing => {
                error!("Invariant violated. Attempted to update a peer that is initializing: {} - {:?}", peer, status.outgoing_status);
                panic!();
            }
            PeerStatus::Known => {
                error!(
                    "Invariant violated. Attempted to update a peer that is not syncing: {} - {:?}",
                    peer, status.outgoing_status
                );
                panic!();
            }
            PeerStatus::Syncing(ref mut status) => {
                status.is_outstanding = false;
            }
        }
    }

    fn update_outgoing_peer_ecg_to_ready(&mut self, peer: &DeviceId) {
        self.update_outgoing_peer_to_ready_helper(peer, |info| &mut info.ecg_status)
    }

    fn update_outgoing_peer_scg_to_ready(&mut self, peer: &DeviceId) {
        self.update_outgoing_peer_to_ready_helper(peer, |info| &mut info.scg_status)
    }

    fn update_outgoing_peer_bft_to_ready(&mut self, peer: &DeviceId) {
        self.update_outgoing_peer_to_ready_helper(peer, |info| &mut info.bft_status)
    }

    /// Send sync requests to peers.
    fn send_sync_requests(&mut self) {
        fn send_command<T: Debug>(i: &mut PeerProtocolStatus<T>, message: T) {
            let PeerStatus::Syncing(ref mut s) = i.outgoing_status else {
                unreachable!("Already checked that the peer is ready. Status: {:?}", i.outgoing_status);
            };

            // Mark as outstanding.
            s.is_outstanding = true;
            s.sender_peer.send(message).expect("TODO");
        }

        // Get and randomize peers (of this store) without outstanding requests.
        fn get_outstanding_peers<SHeader: DAGHeader, THeader: DAGHeader, CommandType>(
            peers: &mut BTreeMap<
                DeviceId,
                PeerInfo<SHeader::HeaderId, SHeader, THeader::HeaderId, THeader>,
            >,
            protocol_f: fn(
                &PeerInfo<SHeader::HeaderId, SHeader, THeader::HeaderId, THeader>,
            ) -> &PeerProtocolStatus<CommandType>,
        ) -> Vec<(
            &DeviceId,
            &mut PeerInfo<SHeader::HeaderId, SHeader, THeader::HeaderId, THeader>,
        )> {
            let mut peers: Vec<_> = peers
                .iter_mut()
                .filter(|(_, i)| i.is_ready_for_sync(protocol_f))
                .collect();
            // TODO: Rank and (weighted) randomize the peers.
            let mut rng = rand::rng();
            peers.shuffle(&mut rng);
            peers
        }

        // Send ECG requests to peers based on what we need.
        let mut ecg_peers = get_outstanding_peers(&mut self.peers, |i| &i.ecg_status);
        match &self.state_machine {
            StateMachine::DownloadingMetadata { .. } => {
                // Tell the first peer store task to request the metadata.
                ecg_peers.iter_mut().take(1).for_each(|(_, i)| {
                    let message = StoreSyncCommand::MetadataHeaderRequest;
                    send_command(&mut i.ecg_status, message);
                });
            }
            StateMachine::DownloadingMerkle {
                partial_merkle_tree,
                ..
            } => {
                debug!(
                    "send_sync_requests: DownloadingMerkle: {:?}",
                    partial_merkle_tree
                );

                // TODO: Keep track of (and filter out) which ones are currently requested.
                let needed_hashes = partial_merkle_tree
                    .missing_indices()
                    .chunks(MERKLE_REQUEST_LIMIT as usize);
                let mut needed_hashes: Vec<_> = needed_hashes.into_iter().collect();
                if needed_hashes.is_empty() {
                    panic!("Invariant violated: We have all merkle nodes but are in state DownloadingMerkle.");
                }

                // Randomize which peer to request the hashes from.
                let mut rng = rand::rng();
                needed_hashes.shuffle(&mut rng);

                ecg_peers
                    .iter_mut()
                    .zip(needed_hashes)
                    .for_each(|(p, hashes)| {
                        let message = StoreSyncCommand::MerkleRequest(
                            compress_consecutive_into_ranges(hashes).collect(),
                        );
                        send_command(&mut p.1.ecg_status, message);
                    });
            }
            StateMachine::DownloadingInitialState { initial_state, .. } => {
                let needed_blocks = initial_state
                    .iter()
                    .enumerate()
                    .filter_map(|h| {
                        if h.1.is_none() {
                            Some(h.0 as u64)
                        } else {
                            None
                        }
                    })
                    .chunks(BLOCK_REQUEST_LIMIT as usize);
                let mut needed_blocks: Vec<_> = needed_blocks.into_iter().collect();
                if needed_blocks.is_empty() {
                    panic!("Invariant violated: We have all blocks but are in state DownloadingInitialState.");
                }

                // Randomize which peer to request the blocks from.
                let mut rng = rand::rng();
                needed_blocks.shuffle(&mut rng);

                ecg_peers
                    .iter_mut()
                    .zip(needed_blocks)
                    .for_each(|(p, blocks)| {
                        let message = StoreSyncCommand::InitialStateBlockRequest(
                            compress_consecutive_into_ranges(blocks).collect(),
                        );
                        send_command(&mut p.1.ecg_status, message);
                    });
            }
            StateMachine::Syncing { ecg_state, .. } => {
                debug!("Sending ECG sync requests to peers.");
                // Request ECG updates from peers
                ecg_peers.iter_mut().for_each(|p| {
                    // let ecg_status = p.1.ecg_status.clone();
                    let ecg_state = ecg_state.state().clone();
                    let message = StoreSyncCommand::ECGSyncRequest { ecg_state };
                    debug!("Sending ECG sync request to peer ({})", p.0);
                    send_command(&mut p.1.ecg_status, message)
                });
            }
        }

        // Send SCG requests
        match &self.state_machine {
            StateMachine::Syncing { sc_state, .. } => {
                debug!("Sending SCG sync requests to peers.");
                let mut scg_peers = get_outstanding_peers(&mut self.peers, |i| &i.scg_status);
                scg_peers.iter_mut().for_each(|p| {
                    let dag_state = sc_state.dag_state.state().clone();
                    let message = StoreSCGSyncCommand::SCGSyncRequest { dag_state };
                    debug!("Sending SCG sync request to peer ({})", p.0);
                    send_command(&mut p.1.scg_status, message);

                    let message = StoreBFTSyncCommand::BFTSyncRequest;
                    debug!("Sending BFT sync request to peer ({})", p.0);
                    send_command(&mut p.1.bft_status, message);
                });
            }
            _ => {}
        }
    }

    // Handle a sync request for this store from a peer.
    fn handle_metadata_peer_request(
        &mut self,
        peer: DeviceId,
        response_chan: Sender<HandlePeerResponse<MetadataHeader<Hash>>>,
    ) {
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

    fn handle_merkle_peer_request(
        &mut self,
        peer: DeviceId,
        node_ids: Vec<Range<u64>>,
        response_chan: Sender<HandlePeerResponse<Vec<Hash>>>,
    ) {
        debug!("Received merkle peer request for node_ids: {node_ids:?}");
        let hashes = self
            .merkle_tree()
            .map(|merkle_tree| handle_merkle_peer_request_helper(merkle_tree, &node_ids));

        if let Some(node_hashes) = hashes {
            // We have the hashes so share it with the peer.
            response_chan.send(Ok(node_hashes)).expect("TODO");
        } else {
            // We don't have the hashes so tell them to wait.
            let (send_chan, recv_chan) = oneshot::channel();
            response_chan.send(Err(recv_chan)).expect("TODO");

            // Register the wait channel.
            self.merkle_subscribers.insert(peer, (node_ids, send_chan)); // JP: Safe to drop old one?
        }
    }

    fn handle_block_peer_request(
        &mut self,
        peer: DeviceId,
        block_ids: Vec<Range<u64>>,
        response_chan: Sender<HandlePeerResponse<Vec<Option<Vec<u8>>>>>,
    ) {
        let blocks = handle_block_peer_request_helper(&self.state_machine, &block_ids);

        if let Some(blocks) = blocks {
            // We have the blocks so share it with the peer.
            response_chan.send(Ok(blocks)).expect("TODO");
        } else {
            // We don't have the blocks so tell them to wait.
            let (send_chan, recv_chan) = oneshot::channel();
            response_chan.send(Err(recv_chan)).expect("TODO");

            // Register the wait channel.
            self.block_subscribers.insert(peer, (block_ids, send_chan)); // JP: Safe to drop old one?
        }
    }

    fn handle_ecg_subscribe(
        &mut self,
        peer: DeviceId,
        tips: Option<BTreeSet<THeader::HeaderId>>,
        response_chan: oneshot::Sender<dag::UntypedState<THeader::HeaderId, THeader>>,
    ) {
        // Respond immediately if peer thread is stale (or they requested it immediately with None).
        if let StateMachine::Syncing { ecg_state, .. } = &self.state_machine {
            let respond_immediately = if let Some(tips) = tips {
                debug!("our_tips: {:?}", ecg_state.tips());
                debug!("their_tips: {:?}", tips);
                !ecg_state.tips().eq(&tips) // JP: Only respond now if our tips exceed theirs??
            } else {
                true
            };

            if respond_immediately {
                debug!("Responding immediately with ECG state.");
                response_chan.send(ecg_state.state.clone()).expect("TODO");

                return;
            }
        };

        // Register subscriber.
        debug!("Registering subscriber for ECG state for peer: {peer}");
        self.ecg_subscribers.insert(peer, response_chan);
    }

    fn handle_scg_subscribe(
        &mut self,
        peer: DeviceId,
        tips: Option<BTreeSet<SHeader::HeaderId>>,
        response_chan: oneshot::Sender<dag::UntypedState<SHeader::HeaderId, SHeader>>,
    ) {
        // Respond immediately if peer thread is stale (or they requested it immediately with None).
        if let StateMachine::Syncing { sc_state, .. } = &self.state_machine {
            let respond_immediately = if let Some(tips) = tips {
                debug!("our_tips: {:?}", sc_state.dag_state.tips());
                debug!("their_tips: {:?}", tips);
                !sc_state.dag_state.tips().eq(&tips) // JP: Only respond now if our tips exceed theirs??
            } else {
                true
            };

            if respond_immediately {
                debug!("Responding immediately with SCG state.");
                response_chan.send(sc_state.dag_state.state.clone()).expect("TODO");

                return;
            }
        };

        // Register subscriber.
        debug!("Registering subscriber for SCG state for peer: {peer}");
        self.scg_subscribers.insert(peer, response_chan);
    }

    // fn handle_ecg_sync_request(&mut self, peer: DeviceId, request: (Vec<Header::HeaderId>, Vec<Header::HeaderId>), response_chan: Sender<HandlePeerResponse<Vec<(Header, RawECGBody)>>>) {
    //     // JP: Instead of receiving the meet, receive the frontier of what they need? Or have an
    //     // enum where that's one option (but what if a new branch from the root is added)??
    //     let (meet, tips) = request;
    //     // Send everything after the meet (that they don't have). Update meet?

    //     // Traverse backwards from our tip + stop once we get to something they have
    //     todo!();
    // }

    fn handle_received_metadata(
        &mut self,
        peer: DeviceId,
        metadata: MetadataHeader<Hash>,
        listeners: &[UnboundedSender<StateUpdate<SHeader, S, THeader, T>>],
    ) where
        S: for<'d> Deserialize<'d>,
        T: for<'d> Deserialize<'d>,
    {
        debug!("Recieved metadata from peer ({peer}): {metadata:?}");

        // Mark peer as ready.
        self.update_outgoing_peer_ecg_to_ready(&peer);

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
            partial_merkle_tree: MerkleTree::new_with_capacity(
                metadata.merkle_root,
                metadata.block_count(),
            ),
            metadata,
        };
        debug!("Updated state machine"); // : {:?}", self.state_machine);

        // Send metadata to any peers that are waiting.
        let subs = std::mem::take(&mut self.metadata_subscribers);
        for (sub_peer, sub) in subs {
            let peer_knows = sub_peer == peer;
            let msg = if peer_knows { None } else { Some(metadata) };
            sub.send(msg).expect("TODO");
        }

        // If there's only one block, we already know the entire merkle tree so move onto downloading blocks.
        if metadata.block_count() <= 1 {
            self.update_state_to_downloading_initial_state(peer, listeners);
        }
    }

    fn metadata(&self) -> Option<&MetadataHeader<Hash>> {
        match &self.state_machine {
            StateMachine::DownloadingMetadata { .. } => None,
            StateMachine::DownloadingMerkle { metadata, .. } => Some(metadata),
            StateMachine::DownloadingInitialState { metadata, .. } => Some(metadata),
            StateMachine::Syncing { metadata, .. } => Some(metadata),
        }
    }

    fn merkle_tree(&self) -> Option<&MerkleTree<Hash>> {
        match &self.state_machine {
            StateMachine::DownloadingMetadata { .. } => None,
            StateMachine::DownloadingMerkle { .. } => {
                // We shouldn't share hashes until we've verified them.
                // TODO: If we've verified some of the hashes, we can send those back.
                None
            }
            StateMachine::DownloadingInitialState { merkle_tree, .. } => Some(merkle_tree),
            StateMachine::Syncing { merkle_tree, .. } => Some(merkle_tree),
        }
    }

    fn handle_received_merkle_hashes(
        &mut self,
        peer: DeviceId,
        node_ids: Vec<Range<u64>>,
        their_node_hashes: Vec<Hash>,
        listeners: &[UnboundedSender<StateUpdate<SHeader, S, THeader, T>>],
    ) where
        S: for<'d> Deserialize<'d>,
        T: for<'d> Deserialize<'d>,
    {
        warn!("TODO: Keep track if you received different hashes from different peers.");

        // Mark peer as ready.
        self.update_outgoing_peer_ecg_to_ready(&peer);

        let node_ids: Vec<_> = node_ids.into_iter().flatten().collect();
        if node_ids.len() != their_node_hashes.len() {
            warn!("TODO: Peer provided an invalid response");
            return;
        }

        // Update state.
        let partial_merkle_tree = if let StateMachine::DownloadingMerkle {
            ref mut partial_merkle_tree,
            ..
        } = &mut self.state_machine
        {
            partial_merkle_tree
        } else {
            // We're not downloading anymore so we're done.
            return;
        };
        node_ids
            .into_iter()
            .zip(their_node_hashes)
            .for_each(|(i, hash)| {
                let valid = partial_merkle_tree.set(i, hash);
                if !valid {
                    warn!("TODO: Peer sent us an invalid merkle hash.");
                } // TODO: Else credit peer with sending us these merkle nodes.
            });

        // Update state.
        self.update_state_to_downloading_initial_state(peer, listeners);
    }

    fn handle_received_initial_state_blocks(
        &mut self,
        peer: DeviceId,
        block_ids: Vec<Range<u64>>,
        their_blocks: Vec<Option<Vec<u8>>>,
        listeners: &[UnboundedSender<StateUpdate<SHeader, S, THeader, T>>],
    ) where
        S: for<'d> Deserialize<'d>,
        T: for<'d> Deserialize<'d>,
    {
        // Mark peer as ready.
        self.update_outgoing_peer_ecg_to_ready(&peer);

        let block_ids: Vec<_> = block_ids.into_iter().flatten().collect();
        if block_ids.len() != their_blocks.len() {
            warn!("TODO: Peer provided an invalid response. Expected length {}. Received length {}", block_ids.len(), their_blocks.len());
            return;
        }

        // Update state.
        let (initial_state, merkle_tree) = if let StateMachine::DownloadingInitialState {
            ref mut initial_state,
            ref merkle_tree,
            ..
        } = &mut self.state_machine
        {
            (initial_state, merkle_tree)
        } else {
            return;
        };
        block_ids
            .into_iter()
            .zip(their_blocks)
            .for_each(|(i, their_block)| {
                if let Some(their_block) = their_block {
                    let block = &mut initial_state[i as usize];
                    // Only set block if it's currently None and if it validates.
                    if block.is_none() {
                        // TODO: Credit peer with this block. If not valid, penalize peer.
                        if merkle_tree.validate_chunk(i, &their_block) {
                            *block = Some(their_block);
                        } else {
                            warn!("TODO: Peer sent us an invalid block ({})", i);
                            debug!("{:?}", their_block);
                        }
                    }
                }
            });

        // Check if we're done. Exit if we're not.
        if initial_state.iter().any(|o| o.is_none()) {
            return;
        }

        // Update state.
        self.update_state_to_syncing(peer, listeners);
    }

    fn handle_received_scg_operations<OT>(
        &mut self,
        peer: DeviceId,
        operations: Vec<(SHeader, RawDAGBody)>,
        listeners: &[UnboundedSender<StateUpdate<SHeader, S, THeader, T>>],
    ) where
        OT: OssaType<SCGHeader = SHeader, ECGHeader = THeader>,
        S: SCDT,
        OT::SCGBody<S>: for<'d> Deserialize<'d>,
        S::Op: ConcretizeTime<<OT::SCGHeader as DAGHeader>::HeaderId>,
    {
        // Mark peer as ready.
        self.update_outgoing_peer_scg_to_ready(&peer);

        warn!("TODO: Validate operations from peer");

        let StateMachine::Syncing {
            ref mut sc_state,
            ref mut decrypted_state,
            ..
        } = &mut self.state_machine
        else {
            unreachable!("We must be syncing");
        };

        // Parse all operations.
        operations.into_iter().for_each(|(header, raw_operations)| {
            let operations: OT::SCGBody<S> = serde_cbor::from_slice(&raw_operations)
                .expect("TODO: Peer gave us improperly serialized operations");

            // TODO: Get rid of this clone.
            let success = sc_state
                .dag_state
                .insert_header(header.clone(), raw_operations);
            if !success {
                warn!("TODO: Failed to insert operations from peer.");
            } else {
                register_scg_operations::<OT, _, _>(
                    decrypted_state,
                    &sc_state.dag_state,
                    operations,
                );
            }
        });

        // Update listeners (except peer).
        update_sc_listeners(
            &mut self.scg_subscribers,
            listeners,
            &sc_state.initial_state,
            &sc_state.dag_state,
            Some(peer),
        );
    }

    fn handle_received_bft_operations<OT>(
        &mut self,
        peer: DeviceId,
        updates: Vec<BFTSyncResponse<SHeader::HeaderId>>,
        // listeners: &[UnboundedSender<StateUpdate<SHeader, S, THeader, T>>],
    )
    {
        // Mark peer as ready.
        self.update_outgoing_peer_bft_to_ready(&peer);

        if updates.is_empty() {
            return;
        }

        warn!("TODO: Validate operations from peer");

        self.bft_state.send_modify(|bft_state| {
            for update in updates {
                let _success = bft_state.handle_update(update);

                warn!("TODO: record whether successful or not for peer.");
            }
        });

        warn!("TODO: Do we need to update listeners?");
    }

    fn handle_received_ecg_operations<OT>(
        &mut self,
        peer: DeviceId,
        operations: Vec<(THeader, RawDAGBody)>,
        listeners: &[UnboundedSender<StateUpdate<SHeader, S, THeader, T>>],
    ) where
        OT: OssaType<ECGHeader = THeader, SCGHeader = SHeader>,
        T: CRDT<Time = OT::Time> + Debug,
        OT::ECGBody<T>: for<'d> Deserialize<'d>
            + Debug
            + DAGBody<
                T::Op,
                <T::Op as ConcretizeTime<<OT::ECGHeader as DAGHeader>::HeaderId>>::Serialized,
                Header = OT::ECGHeader,
            >, // ECGBody<T, Header = OT::ECGHeader> +
        // T::Op: ConcretizeTime<<OT::ECGHeader as ECGHeader>::HeaderId>,
        T::Op: ConcretizeTime<<THeader as DAGHeader>::HeaderId>,
    {
        // Mark peer as ready.
        self.update_outgoing_peer_ecg_to_ready(&peer);

        warn!("TODO: Validate operations from peer");

        let StateMachine::Syncing {
            ref mut ecg_state,
            ref mut decrypted_state,
            ..
        } = &mut self.state_machine
        else {
            unreachable!("We must be syncing");
        };

        // Parse and apply all operations.
        operations.into_iter().for_each(|(header, raw_operations)| {
            let operations = serde_cbor::from_slice(&raw_operations)
                .expect("TODO: Peer gave us improperly serialized operations");
            debug!("Applying operations {operations:?}");

            // TODO: Get rid of this clone.
            let success = ecg_state.insert_header(header.clone(), raw_operations);
            if !success {
                debug!("Failed to insert operations from peer.");
            } else {
                apply_operations::<OT, _>(decrypted_state, ecg_state, &header, operations);
            }
        });
        debug!("New decrypted state {:?}", decrypted_state.latest_ec_state);

        // Update listeners (except peer).
        update_ec_listeners(
            &mut self.ecg_subscribers,
            listeners,
            &decrypted_state.latest_ec_state,
            ecg_state,
            Some(peer),
        );
    }

    // Precondition: State is StateMachine::DownloadingMerkle.
    fn update_state_to_downloading_initial_state(
        &mut self,
        peer: DeviceId,
        listeners: &[UnboundedSender<StateUpdate<SHeader, S, THeader, T>>],
    ) where
        S: for<'d> Deserialize<'d>,
        T: for<'d> Deserialize<'d>,
    {
        let StateMachine::DownloadingMerkle {
            ref partial_merkle_tree,
            ..
        } = &self.state_machine
        else {
            panic!("Precondition violated. State mush be StateMachine::DownloadingMerkle");
        };
        // Check if we're done.
        let merkle_tree_m = partial_merkle_tree.try_complete();
        let Some(merkle_tree) = merkle_tree_m else {
            return;
        };

        // // Validate piece hashes.
        // let is_valid = self.metadata().unwrap().merkle_root == util::merkle_root(&piece_hashes);
        // if !is_valid {
        //     // TODO: Penalize/blacklist/disconnect peer?
        //     warn!("TODO: Peer provided invalid piece hashes.");
        //     return;
        // }

        self.state_machine = match self.state_machine {
            StateMachine::DownloadingMerkle { metadata, .. } => {
                let initial_state = vec![None; metadata.block_count() as usize];
                StateMachine::DownloadingInitialState {
                    metadata,
                    merkle_tree,
                    initial_state,
                }
            }
            _ => unreachable!("We already checked that we're downloading merkle"),
        };

        // Send node hashes to any peers that are waiting.
        let subs = std::mem::take(&mut self.merkle_subscribers);
        let merkle_tree = self
            .merkle_tree()
            .expect("Unreachable: We just set the merkle tree");
        for (sub_peer, (node_ids, sub)) in subs {
            let peer_knows = sub_peer == peer;
            let msg = if peer_knows {
                None
            } else {
                Some(handle_merkle_peer_request_helper(merkle_tree, &node_ids))
            };
            sub.send(msg).expect("TODO");
        }

        // If there's no initial state, we already know all the blocks so move onto syncing.
        if self.metadata().unwrap().merkle_size() == 0 {
            self.update_state_to_syncing(peer, listeners);
        }
    }

    fn update_state_to_syncing(
        &mut self,
        peer: DeviceId,
        listeners: &[UnboundedSender<StateUpdate<SHeader, S, THeader, T>>],
    ) where
        S: for<'d> Deserialize<'d>,
        T: for<'d> Deserialize<'d>,
    {
        replace_with_or_abort(&mut self.state_machine, |sm| match sm {
            StateMachine::DownloadingInitialState {
                metadata,
                merkle_tree,
                initial_state,
            } => {
                let ecg_state = dag::State::new();
                let initial_state: Vec<u8> = initial_state
                    .into_iter()
                    .flatten()
                    .flatten()
                    .collect::<Vec<u8>>();
                debug_assert_eq!(initial_state.len(), metadata.merkle_size() as usize);
                let Ok(latest_sc_state) = serde_cbor::de::from_slice::<S>(
                    &initial_state[..metadata.initial_sc_state_size as usize],
                ) else {
                    todo!("TODO: The store is invalid. Initial SC state does not parse.");
                };
                let Ok(latest_ec_state) = serde_cbor::de::from_slice::<T>(
                    &initial_state[metadata.initial_sc_state_size as usize..],
                ) else {
                    todo!("TODO: The store is invalid. Initial EC state does not parse.");
                };
                let decrypted_state = DecryptedState {
                    latest_ec_state,
                    latest_headers: BTreeSet::new(),
                    sc_operations: BTreeMap::new(),
                };
                let sc_state = bft::State::new(latest_sc_state);
                StateMachine::Syncing {
                    metadata,
                    merkle_tree,
                    initial_state,
                    ecg_state,
                    sc_state,
                    decrypted_state,
                }
            }
            _ => unreachable!("We already checked that we're downloading the initial state"),
        });

        // Update listeners.
        let StateMachine::Syncing {
            ecg_state,
            decrypted_state,
            sc_state,
            ..
        } = &self.state_machine
        else {
            unreachable!("We just set our state to syncing")
        };
        update_sc_listeners(
            &mut self.scg_subscribers,
            listeners,
            &sc_state.initial_state,
            &sc_state.dag_state,
            Some(peer),
        );
        update_ec_listeners(
            &mut self.ecg_subscribers,
            listeners,
            &decrypted_state.latest_ec_state,
            ecg_state,
            Some(peer),
        );

        // Send blocks to any peers that are waiting.
        let subs = std::mem::take(&mut self.block_subscribers);
        for (_sub_peer, (block_ids, sub)) in subs {
            // JP: Do we need to keep sub_peer here?
            let msg = handle_block_peer_request_helper(&self.state_machine, &block_ids)
                .expect("Unreachable: We just set our state to syncing");
            sub.send(Some(msg)).expect("TODO");
        }
    }
}

fn register_scg_operations<OT: OssaType, S: SCDT, T: CRDT>(
    decrypted_state: &mut DecryptedState<OT::SCGHeader, OT::ECGHeader, T>,
    scg_state: &dag::State<OT::SCGHeader, S>,
    operation_body: OT::SCGBody<S>,
) -> ()
where
    S::Op: ConcretizeTime<<OT::SCGHeader as DAGHeader>::HeaderId>,
{
    warn!("TODO: Is there anything we need to do here? Some sort of validation of operations? Cache pending operations?");
}

fn update_sc_listeners<SHeader: dag::DAGHeader + Clone + Debug, S: Clone, THeader: dag::DAGHeader, T>(
    scg_subscribers: &mut BTreeMap<
        DeviceId,
        oneshot::Sender<dag::UntypedState<SHeader::HeaderId, SHeader>>,
    >,
    listeners: &[UnboundedSender<StateUpdate<SHeader, S, THeader, T>>],
    latest_state: &S,
    dag_state: &dag::State<SHeader, S>,
    from_peer: Option<DeviceId>,
) {
    for l in listeners {
        let snapshot: StateUpdate<SHeader, S, THeader, T> = StateUpdate::SnapshotSC {
            snapshot: latest_state.clone(),
            dag_state: dag_state.clone(),
        };
        l.send(snapshot).expect("TODO");
    }

    // Send updated state to one-time subscribers.
    // warn!("TODO: Do we always want to update SCG subscribers here? Ex: We may not want to when transitioning from downloading to syncing"); JP: Maybe this is ok since our peer_store won't have anything to share and will resubscribe.
    let subs = std::mem::take(scg_subscribers);
    for (sub_peer, sub) in subs {
        // Skip notifying subscriber if they told us about this update.
        if Some(sub_peer) != from_peer {
            sub.send(dag_state.state.clone()).expect("TODO");
        } else {
            warn!("TODO: Add headers that they sent us to their_known.");
            // Need to add back subscriber.
            scg_subscribers.insert(sub_peer, sub);
        }
    }
}

fn update_ec_listeners<SHeader: dag::DAGHeader, S, THeader: dag::DAGHeader + Clone + Debug, T: CRDT + Clone>(
    ecg_subscribers: &mut BTreeMap<
        DeviceId,
        oneshot::Sender<dag::UntypedState<THeader::HeaderId, THeader>>,
    >,
    listeners: &[UnboundedSender<StateUpdate<SHeader, S, THeader, T>>],
    latest_state: &T,
    ecg_state: &dag::State<THeader, T>,
    from_peer: Option<DeviceId>,
) {
    for l in listeners {
        let snapshot: StateUpdate<SHeader, S, THeader, T> = StateUpdate::SnapshotEC {
            snapshot: latest_state.clone(),
            dag_state: ecg_state.clone(),
        };
        l.send(snapshot).expect("TODO");
    }

    // Send updated state to one-time subscribers.
    // warn!("TODO: Do we always want to update ECG subscribers here? Ex: We may not want to when transitioning from downloading to syncing"); JP: Maybe this is ok since our peer_store won't have anything to share and will resubscribe.
    let subs = std::mem::take(ecg_subscribers);
    for (sub_peer, sub) in subs {
        // Skip notifying subscriber if they told us about this update.
        if Some(sub_peer) != from_peer {
            sub.send(ecg_state.state.clone()).expect("TODO");
        } else {
            warn!("TODO: Add headers that they sent us to their_known.");
            // Need to add back subscriber.
            ecg_subscribers.insert(sub_peer, sub);
        }
    }
}

// JP: Or should Ossa own this/peers?
/// Manage peers by ranking them, randomize, potentially connecting to some of them, etc.
async fn manage_peers<OT: OssaType, S: Clone, T: CRDT<Time = OT::Time> + Clone + Send + 'static>(
    store: &mut State<OT::StoreId, OT::SCGHeader, OT::ECGHeader, S, T, OT::Hash>,
    shared_state: &SharedState<OT::StoreId>,
    send_commands: &UnboundedSender<
        UntypedStoreCommand<
            OT::Hash,
            <OT::SCGHeader as DAGHeader>::HeaderId,
            OT::SCGHeader,
            <OT::ECGHeader as DAGHeader>::HeaderId,
            OT::ECGHeader,
        >,
    >,
) where
    // T::Op<CausalTime<T::Time>>: Serialize,
    OT::ECGHeader: Clone + Serialize + for<'d> Deserialize<'d> + Send + Sync,
    <OT::ECGHeader as DAGHeader>::HeaderId: Serialize + for<'d> Deserialize<'d> + Send,
    OT::SCGHeader: Clone + Debug + Serialize + for<'d> Deserialize<'d> + Sync,
    <OT::SCGHeader as DAGHeader>::HeaderId: Serialize + for<'d> Deserialize<'d> + Send + Sync,
    //OT::ECGHeader<T>::HeaderId : Send,
    //T: Send,
{
    // For now, sync with all (connected?) peers.
    // Don't connect to peers we're already syncing with.
    let peers: Vec<_> = store
        .peers
        .iter()
        .filter(|p| p.1.ecg_status.outgoing_status.is_known())
        .collect();
    let peers: Vec<_> = {
        // Acquire lock on shared state.
        let peer_states = shared_state.peer_state.read().await;
        peers
            .into_iter()
            .filter_map(|(peer_id, _)| {
                let chan = peer_states.get(peer_id)?;
                Some((*peer_id, chan.clone()))
            })
            .collect()
    };
    for (peer_id, command_chan) in peers {
        // Mark task as initializing.
        store.update_peer_to_initializing_outgoing(&peer_id);

        // Create closure that spawns task to sync store with peer.
        let send_commands_ = send_commands.clone();
        let spawn_task_ec = Box::new(move |_party, stream_id, sender, receiver| {
            // Create miniprotocol
            // Spawn task that syncs store with peer.
            // JP: Should run with initiative?
            tokio::spawn(async move {
                // Tell store we're running and send it our channel.
                let (send_peer, recv_peer) = tokio::sync::mpsc::unbounded_channel::<
                    StoreSyncCommand<<OT::ECGHeader as DAGHeader>::HeaderId, OT::ECGHeader>,
                >();

                let register_cmd = UntypedStoreCommand::RegisterOutgoingPeerECGSyncing {
                    peer: peer_id,
                    send_peer,
                };
                send_commands_.send(register_cmd).expect("TODO");

                // Start miniprotocol as server.
                let mp = StoreSync::<OT::Hash, _, _, _, _>::new_server(
                    peer_id,
                    recv_peer,
                    send_commands_,
                );
                run_miniprotocol_async(mp, false, stream_id, sender, receiver).await;

                debug!("Store ECG sync with peer (with initiative) exited.")

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

        let send_commands_ = send_commands.clone();
        let spawn_task_sc = Box::new(move |_party, stream_id, sender, receiver| {
            tokio::spawn(async move {
                // Tell store we're running and send it our channel.
                let (send_peer, recv_peer) = tokio::sync::mpsc::unbounded_channel();
                let register_cmd = UntypedStoreCommand::RegisterOutgoingPeerSCGSyncing {
                    peer: peer_id,
                    send_peer,
                };
                send_commands_.send(register_cmd).expect("TODO");

                // Run SC miniprotocol as server
                let mp = StoreDAGSync::<OT::Hash, _, _, _, _>::new_server(
                    peer_id,
                    recv_peer,
                    send_commands_,
                );
                run_miniprotocol_async(mp, false, stream_id, sender, receiver).await;

                debug!("Store SCG sync with peer (with initiative) exited.")
            })
        });

        let send_commands_ = send_commands.clone();
        let bft_state = store.bft_state.subscribe();
        let spawn_task_bft = Box::new(move |_party, stream_id, sender, receiver| {
            tokio::spawn(async move {
                // Tell store we're running and send it our channel.
                let (send_peer, recv_peer) = tokio::sync::mpsc::unbounded_channel();
                let register_cmd = UntypedStoreCommand::RegisterOutgoingPeerBFTSyncing {
                    peer: peer_id,
                    send_peer,
                };
                send_commands_.send(register_cmd).expect("TODO");

                // Start miniprotocol as server.
                let mp = StoreBFTSync::new_server(peer_id, recv_peer, send_commands_, bft_state);
                run_miniprotocol_async(mp, false, stream_id, sender, receiver).await;

                debug!("Store BFT sync with peer (with initiative) exited.")
            })
        });

        // Send request to peer's manager for stream.
        let store_id = store.store_id();
        let cmd = PeerManagerCommand::RequestStoreSync {
            store_id,
            spawn_task_ec,
            spawn_task_sc,
            spawn_task_bft,
        };
        command_chan.send(cmd).expect("TODO");
    }
}

fn apply_operations<OT: OssaType, T>(
    decrypted_state: &mut DecryptedState<OT::SCGHeader, OT::ECGHeader, T>,
    ecg_state: &dag::State<OT::ECGHeader, T>,
    operation_header: &OT::ECGHeader,
    operation_body: OT::ECGBody<T>,
) where
    T: CRDT<Time = OT::Time>,
    // T::Op<CausalTime<T::Time>>: Serialize,
    T::Op: ConcretizeTime<<OT::ECGHeader as DAGHeader>::HeaderId>,
    OT::ECGBody<T>: DAGBody<
        T::Op,
        <T::Op as ConcretizeTime<<OT::ECGHeader as DAGHeader>::HeaderId>>::Serialized,
        Header = OT::ECGHeader,
    >,
{
    let causal_state = OT::to_causal_state(ecg_state);
    for operation in operation_body.operations(operation_header.get_header_id()) {
        replace_with_or_abort(&mut decrypted_state.latest_ec_state, |s| {
            s.apply(causal_state, operation)
        });
    }
}

/// Run the handler that owns this store and manages its state. This handler is typically run in
/// its own tokio thread.
pub(crate) async fn run_handler<OT: OssaType, S, T>(
    mut store: State<OT::StoreId, OT::SCGHeader, OT::ECGHeader, S, T, OT::Hash>,
    mut recv_commands: UnboundedReceiver<StoreCommand<OT::SCGHeader, OT::SCGBody<S>, S, OT::ECGHeader, OT::ECGBody<T>, T>>,
    send_commands_untyped: UnboundedSender<
        UntypedStoreCommand<
            OT::Hash,
            <OT::SCGHeader as DAGHeader>::HeaderId,
            OT::SCGHeader,
            <OT::ECGHeader as DAGHeader>::HeaderId,
            OT::ECGHeader,
        >,
    >,
    mut recv_commands_untyped: UnboundedReceiver<
        UntypedStoreCommand<
            OT::Hash,
            <OT::SCGHeader as DAGHeader>::HeaderId,
            OT::SCGHeader,
            <OT::ECGHeader as DAGHeader>::HeaderId,
            OT::ECGHeader,
        >,
    >,
    shared_state: SharedState<OT::StoreId>,
) where
    // <<OT as OssaType>::ECGHeader as ECGHeader>::Body: ECGBody<T> + Send,
    S::Op: ConcretizeTime<<OT::SCGHeader as DAGHeader>::HeaderId>,
    T::Op: ConcretizeTime<<OT::ECGHeader as DAGHeader>::HeaderId>,
    OT::SCGBody<S>: Serialize + for<'d> Deserialize<'d>,
    OT::ECGBody<T>: Serialize
        + for<'d> Deserialize<'d>
        + Debug
        + DAGBody<
            T::Op,
            <T::Op as ConcretizeTime<<OT::ECGHeader as DAGHeader>::HeaderId>>::Serialized,
            Header = OT::ECGHeader,
        >,
    //     ECGBody<T, Header = OT::ECGHeader> + Send + Serialize + for<'d> Deserialize<'d> + Debug,
    OT::ECGHeader: Send + Sync + Clone + Serialize + for<'d> Deserialize<'d> + 'static,
    <OT::ECGHeader as DAGHeader>::HeaderId: Send + Serialize + for<'d> Deserialize<'d>,
    // T::Op<CausalTime<T::Time>>: Serialize,
    OT::SCGHeader: Debug + Clone + Sync + Serialize + for<'d> Deserialize<'d>,
    <OT::SCGHeader as DAGHeader>::HeaderId: Sync + Serialize + for<'d> Deserialize<'d>,
    S: SCDT + Clone + for<'d> Deserialize<'d>,
    T: CRDT<Time = OT::Time> + Debug + Clone + Send + 'static + for<'d> Deserialize<'d>,
{
    let mut listeners: Vec<UnboundedSender<StateUpdate<OT::SCGHeader, S, OT::ECGHeader, T>>> = vec![];

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
                            StateMachine::Syncing { metadata, merkle_tree, initial_state , ecg_state, decrypted_state, sc_state } => {
                                let mut ecg_state = ecg_state;
                                let mut decrypted_state = decrypted_state;

                                // Update ECG state.
                                let serialized_operations = serde_cbor::to_vec(&operation_body).expect("TODO");
                                let success = ecg_state.insert_header(operation_header.clone(), serialized_operations);
                                if !success {
                                    todo!("Invalid header"); // : {:?}", operation_header);
                                }

                                // Update state.
                                // TODO: Get new time?.. Or take it as an argument
                                // Operation ID/time is function of tips, current operation, ...? How do we
                                // do batching? (HeaderId(h) | Self, Index(u8)) ? This requires having all
                                // the batched operations?
                                apply_operations::<OT, _>(&mut decrypted_state, &ecg_state, &operation_header, operation_body);

                                // Send state to subscribers.
                                update_ec_listeners(&mut store.ecg_subscribers, &listeners, &decrypted_state.latest_ec_state, &ecg_state, None);

                                StateMachine::Syncing { metadata, merkle_tree, initial_state, ecg_state, decrypted_state, sc_state }
                            }
                            _ => {
                                warn!("JP: Does this ever happen?");
                                store.state_machine
                            }
                        };
                    }
                    StoreCommand::SubscribeState { send_state } => {
                        // Send current state.
                        let snapshots = match &store.state_machine {
                            StateMachine::DownloadingMetadata { .. } => {
                                vec![StateUpdate::Downloading { percent: 0 }]
                            }
                            StateMachine::DownloadingMerkle { .. } => {
                                vec![StateUpdate::Downloading { percent: 0 }]
                            }
                            StateMachine::DownloadingInitialState { metadata, initial_state, .. } => {
                                let percent = if metadata.merkle_size() == 0 {
                                    0
                                } else {
                                    let downloaded = initial_state.iter().filter(|p| p.is_some()).count() as u64;
                                    100 * downloaded * BLOCK_SIZE / (metadata.merkle_size() as u64)
                                };
                                vec![StateUpdate::Downloading { percent }]
                            }
                            StateMachine::Syncing { ref ecg_state, ref decrypted_state, ref sc_state, .. } => {
                                let ec = StateUpdate::SnapshotEC {
                                    snapshot: decrypted_state.latest_ec_state.clone(),
                                    dag_state: ecg_state.clone(),
                                };
                                let sc = StateUpdate::SnapshotSC {
                                    snapshot: sc_state.initial_state.clone(),
                                    dag_state: sc_state.dag_state.clone(),
                                };
                                vec![sc, ec]
                            }
                        };
                        snapshots.into_iter().for_each(|snapshot| send_state.send(snapshot).expect("TODO"));

                        // Register this subscriber.
                        listeners.push(send_state);
                    }
                    StoreCommand::Propose {
                        operation_header,
                        operation_body,
                    } => {
                        store = handle_propose::<OT, S, T>(store, &mut listeners, operation_header, operation_body).await;
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
                        manage_peers::<OT, S, T>(&mut store, &shared_state, &send_commands_untyped).await;
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
                                if status.ecg_status.incoming_status.is_known() {
                                    // Mark task as initializing.
                                    store.update_peer_to_initializing_incoming(&peer);

                                    // Create closure that spawns task to sync store with peer.
                                    let send_commands_untyped_ = send_commands_untyped.clone();
                                    let spawn_task_ec: Box<SpawnMultiplexerTask> = Box::new(move |party, stream_id, sender, receiver| {
                                        // Create miniprotocol
                                        // Spawn task that syncs store with peer.
                                        // JP: Should run without initiative so that other peer can setup their handler?
                                        tokio::spawn(async move {
                                            debug!("Sync with peer (without initiative).");

                                            // Tell store we're running.
                                            let register_cmd = UntypedStoreCommand::RegisterIncomingPeerECGSyncing {
                                                peer,
                                            };
                                            send_commands_untyped_.send(register_cmd).expect("TODO");

                                            // Start miniprotocol as client.
                                            let mp = StoreSync::<OT::Hash, _, _, _, _>::new_client(peer, send_commands_untyped_);
                                            run_miniprotocol_async(mp, true, stream_id, sender, receiver).await;
                                            debug!("Store ECG sync with peer (without initiative) exited.")
                                        })
                                    });

                                    let send_commands_untyped_ = send_commands_untyped.clone();
                                    let spawn_task_sc: Box<SpawnMultiplexerTask> = Box::new(move |party, stream_id, sender, receiver| {
                                        tokio::spawn(async move {
                                            // JP: Maybe this isn't needed???
                                            // Tell store we're running.
                                            let register_cmd = UntypedStoreCommand::RegisterIncomingPeerSCGSyncing {
                                                peer,
                                            };
                                            send_commands_untyped_.send(register_cmd).expect("TODO");

                                            // Start miniprotocol as client.
                                            let mp = StoreDAGSync::new_client(peer, send_commands_untyped_);
                                            run_miniprotocol_async(mp, true, stream_id, sender, receiver).await;
                                            debug!("Store SCG sync with peer (without initiative) exited.")
                                        })
                                    });

                                    let send_commands_untyped_ = send_commands_untyped.clone();
                                    let bft_state = store.bft_state.subscribe();
                                    let spawn_task_bft: Box<SpawnMultiplexerTask> = Box::new(move |party, stream_id, sender, receiver| {
                                        tokio::spawn(async move {
                                            warn!("TODO: Should we tell store we're running?");

                                            // Start miniprotocol as client.
                                            let mp = StoreBFTSync::new_client(peer, send_commands_untyped_, bft_state);
                                            run_miniprotocol_async(mp, true, stream_id, sender, receiver).await;
                                            debug!("Store BFT sync with peer (without initiative) exited.")
                                        })
                                    });
                                    Some((spawn_task_ec, spawn_task_sc, spawn_task_bft))
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

                        warn!("TODO: Create SC miniprotocol too?");

                        response_chan.send(response).or(Err(())).expect("TODO");
                    }
                    UntypedStoreCommand::RegisterOutgoingPeerECGSyncing{ peer, send_peer } => {
                        // JP: Maybe send_peer actually isn't needed??? We could construct oneshots???
                        // Update peer's state to syncing and register channel.
                        let outgoing_status = OutgoingPeerStatus {
                            sender_peer: send_peer,
                            is_outstanding: false,
                        };
                        store.update_peer_ecg_to_syncing_outgoing(&peer, outgoing_status);

                        // Sync with peer(s). Do this for all commands??
                        store.send_sync_requests();
                    }
                    UntypedStoreCommand::RegisterIncomingPeerECGSyncing{ peer } => {
                        // JP: Maybe this actually isn't needed??? We could construct oneshots for every request..

                        // Update peer's state to syncing and register channel.
                        store.update_peer_ecg_to_syncing_incoming(&peer);
                    }
                    UntypedStoreCommand::RegisterIncomingPeerSCGSyncing{ peer } => {
                        // JP: Maybe this actually isn't needed??? We could construct oneshots for every request..

                        // Update peer's state to syncing and register channel.
                        store.update_peer_scg_to_syncing_incoming(&peer);
                    }
                    UntypedStoreCommand::HandleMetadataPeerRequest(HandlePeerRequest { peer, request, response_chan }) => {
                        store.handle_metadata_peer_request(peer, response_chan);
                    }
                    UntypedStoreCommand::HandleMerklePeerRequest(HandlePeerRequest { peer, request, response_chan }) => {
                        store.handle_merkle_peer_request(peer, request, response_chan);
                    }
                    UntypedStoreCommand::HandleBlockPeerRequest(HandlePeerRequest { peer, request, response_chan }) => {
                        store.handle_block_peer_request(peer, request, response_chan);
                    }
                    UntypedStoreCommand::ReceivedMetadata { peer, metadata } => {
                        store.handle_received_metadata(peer, metadata, &listeners);
                        store.send_sync_requests();
                    }
                    UntypedStoreCommand::ReceivedMerkleHashes { peer, ranges, nodes } => {
                        store.handle_received_merkle_hashes(peer, ranges, nodes, &listeners);
                        store.send_sync_requests();
                    }
                    UntypedStoreCommand::ReceivedInitialStateBlocks { peer, ranges, blocks } => {
                        store.handle_received_initial_state_blocks(peer, ranges, blocks, &listeners);
                        store.send_sync_requests();
                    }
                    UntypedStoreCommand::ReceivedECGOperations { peer, operations } => {
                        store.handle_received_ecg_operations::<OT>(peer, operations, &listeners);
                        store.send_sync_requests();
                    }
                    UntypedStoreCommand::SubscribeECG { peer, tips, response_chan } => {
                        store.handle_ecg_subscribe(peer, tips, response_chan);
                    }
                    UntypedStoreCommand::ReceivedSCGOperations { peer, operations } => {
                        store.handle_received_scg_operations::<OT>(peer, operations, &listeners);
                        store.send_sync_requests();
                    }
                    UntypedStoreCommand::ReceivedBFTOperations { peer, updates } => {
                        store.handle_received_bft_operations::<OT>(peer, updates); // , &listeners);
                        store.send_sync_requests();
                    }
                    UntypedStoreCommand::SubscribeSCG { peer, tips, response_chan } => {
                        store.handle_scg_subscribe(peer, tips, response_chan);
                    }
                    UntypedStoreCommand::RegisterOutgoingPeerSCGSyncing { peer, send_peer } => {
                        // Update peer's state to syncing and register channel.
                        let outgoing_status = OutgoingPeerStatus {
                            sender_peer: send_peer,
                            is_outstanding: false,
                        };
                        store.update_peer_scg_to_syncing_outgoing(&peer, outgoing_status);

                        // Sync with peer(s). Do this for all commands??
                        store.send_sync_requests();
                    }
                    UntypedStoreCommand::RegisterOutgoingPeerBFTSyncing { peer, send_peer } => {
                        // Update peer's state to syncing and register channel.
                        let outgoing_status = OutgoingPeerStatus {
                            sender_peer: send_peer,
                            is_outstanding: false,
                        };
                        store.update_peer_bft_to_syncing_outgoing(&peer, outgoing_status);

                        // Sync with peer(s). Do this for all commands??
                        store.send_sync_requests();
                    }
                }
            }
        }
    }
    debug!("Store thread exiting.");
}

pub(crate) async fn handle_propose<OT: OssaType, S, T>(
    mut store: State<OT::StoreId, OT::SCGHeader, OT::ECGHeader, S, T, OT::Hash>,
    listeners: &mut [UnboundedSender<StateUpdate<OT::SCGHeader, S, OT::ECGHeader, T>>],
    operation_header: OT::SCGHeader,
    operation_body: OT::SCGBody<S>,
) -> State<OT::StoreId, OT::SCGHeader, OT::ECGHeader, S, T, OT::Hash>
where
    S: SCDT<Op: ConcretizeTime<<OT::SCGHeader as DAGHeader>::HeaderId>> + Clone,
    T: CRDT,
    OT::SCGHeader: Clone,
    OT::SCGBody<S>: Serialize,
{
    store.state_machine = match store.state_machine {
        StateMachine::Syncing { metadata, merkle_tree, initial_state, ecg_state, mut sc_state, decrypted_state } => {
            // Update SCG state.
            let serialized_operations = serde_cbor::to_vec(&operation_body).expect("TODO");
            let success = sc_state.dag_state.insert_header(operation_header, serialized_operations);
            if !success {
                todo!("Invalid header");
            }

            warn!("TODO: Vote for our own updates");

            // Send state to subscribers.
            update_sc_listeners(
                &mut store.scg_subscribers,
                listeners,
                &sc_state.initial_state,
                &sc_state.dag_state,
                None,
            );

            StateMachine::Syncing { metadata, merkle_tree, initial_state, ecg_state, sc_state, decrypted_state }
        }
        _ => {
            warn!("JP: Does this ever happen?");
            store.state_machine
        }
    };
    store
}

pub(crate) enum StoreCommand<SHeader: DAGHeader, SBody, S, THeader: DAGHeader, TBody, T> {
    Apply {
        operation_header: THeader, // <Hash, T>,
        operation_body: TBody,     // <Hash, T>,
    },
    // TODO: Support unsubscribe.
    SubscribeState {
        send_state: UnboundedSender<StateUpdate<SHeader, S, THeader, T>>,
    },
    /// Propose a SC operation.
    Propose {
        operation_header: SHeader, // <Hash, T>,
        operation_body: SBody,     // <Hash, T>,
    },
}

pub enum StateUpdate<SHeader: DAGHeader, S, THeader: DAGHeader, T> {
    Downloading {
        // Percent of the state that we've downloaded (0 - 100).
        percent: u64,
    },
    SnapshotEC {
        snapshot: T,
        dag_state: dag::State<THeader, T>,
        // TODO: ECG DAG
    },
    SnapshotSC {
        snapshot: S,
        dag_state: dag::State<SHeader, S>,
    },
}

// trait UntypedCRDT: CRDT<Op = dyn Any, Time = dyn Any> {} // Any + Sized + 'static +
// trait UntypedECGHeader: ECGHeader<dyn Any> {} // Any + Sized + 'static +
//
// struct UntypedStoreCommand(StoreCommand<dyn UntypedECGHeader, dyn UntypedCRDT>);
// struct UntypedStoreCommand(StoreCommand<dyn UntypedECGHeader, dyn Any>);

type HandlePeerResponse<Response> = Result<Response, oneshot::Receiver<Option<Response>>>;

/// Untyped variant of `StoreCommand` since existentials don't work.
// #[derive(Debug)]
pub(crate) enum UntypedStoreCommand<Hash, SHeaderId, SHeader, THeaderId, THeader> {
    /// Register the discovered peers.
    RegisterPeers {
        peers: Vec<DeviceId>,
    },
    /// Request store to sync with peer. Store can refuse.
    SyncWithPeer {
        peer: DeviceId,
        response_chan:
            oneshot::Sender<Option<(Box<SpawnMultiplexerTask>, Box<SpawnMultiplexerTask>, Box<SpawnMultiplexerTask>)>>,
    },
    RegisterOutgoingPeerECGSyncing {
        peer: DeviceId,
        send_peer: UnboundedSender<StoreSyncCommand<THeaderId, THeader>>,
    },
    HandleMetadataPeerRequest(HandlePeerRequest<(), v0::MetadataHeader<Hash>>),
    HandleMerklePeerRequest(HandlePeerRequest<Vec<Range<u64>>, Vec<Hash>>),
    HandleBlockPeerRequest(HandlePeerRequest<Vec<Range<u64>>, Vec<Option<Vec<u8>>>>),
    // HandleECGSyncRequest(HandlePeerRequest<(Vec<HeaderId>, Vec<HeaderId>), Vec<(Header, RawECGBody)>>), // (Meet, Tips)
    ReceivedMetadata {
        peer: DeviceId,
        metadata: MetadataHeader<Hash>,
    },
    ReceivedMerkleHashes {
        peer: DeviceId,
        ranges: Vec<Range<u64>>,
        nodes: Vec<Hash>,
    },
    ReceivedInitialStateBlocks {
        peer: DeviceId,
        ranges: Vec<Range<u64>>,
        blocks: Vec<Option<Vec<u8>>>,
    },
    RegisterIncomingPeerECGSyncing {
        peer: DeviceId,
    },
    ReceivedECGOperations {
        peer: DeviceId,
        operations: Vec<(THeader, RawDAGBody)>,
    },
    SubscribeECG {
        peer: DeviceId,
        tips: Option<BTreeSet<THeaderId>>,
        response_chan: oneshot::Sender<dag::UntypedState<THeaderId, THeader>>,
    },
    ReceivedSCGOperations {
        peer: DeviceId,
        operations: Vec<(SHeader, RawDAGBody)>,
    },
    ReceivedBFTOperations {
        peer: DeviceId,
        updates: Vec<BFTSyncResponse<SHeaderId>>,
    },
    SubscribeSCG {
        peer: DeviceId,
        tips: Option<BTreeSet<SHeaderId>>,
        response_chan: oneshot::Sender<dag::UntypedState<SHeaderId, SHeader>>,
    },
    RegisterOutgoingPeerSCGSyncing {
        peer: DeviceId,
        send_peer: UnboundedSender<StoreSCGSyncCommand<SHeaderId, SHeader>>,
    },
    RegisterIncomingPeerSCGSyncing {
        peer: DeviceId,
    },
    RegisterOutgoingPeerBFTSyncing {
        peer: DeviceId,
        send_peer: UnboundedSender<StoreBFTSyncCommand>,
    },
}

pub(crate) struct HandlePeerRequest<Request, Response> {
    pub(crate) peer: DeviceId,
    pub(crate) request: Request, // MsgStoreSyncRequest,
    /// Return either the result or a channel to wait on for the response.
    pub(crate) response_chan: oneshot::Sender<HandlePeerResponse<Response>>,
}

fn handle_merkle_peer_request_helper<H: Copy>(
    merkle_tree: &MerkleTree<H>,
    node_ids: &[Range<u64>],
) -> Vec<H> {
    warn!("TODO: check ranges are in bounds or return error");
    let hashes: Vec<_> = node_ids
        .iter()
        .cloned()
        .flatten()
        .map(|i| {
            merkle_tree
                .get(i)
                .expect("TODO: Properly handle invalid requests")
                .clone()
        })
        .collect();
    hashes
}

fn handle_block_peer_request_helper<
    StoreId,
    SHeader: dag::DAGHeader,
    THeader: dag::DAGHeader,
    S,
    T: CRDT,
    Hash,
>(
    state_machine: &StateMachine<StoreId, SHeader, THeader, S, T, Hash>,
    block_ids: &[Range<u64>],
) -> Option<Vec<Option<Vec<u8>>>> {
    // TODO: Can we avoid these clones?
    match state_machine {
        StateMachine::DownloadingMetadata { .. } => None,
        StateMachine::DownloadingMerkle { .. } => None,
        StateMachine::DownloadingInitialState { initial_state, .. } => {
            Some(handle_peer_request_range_helper(initial_state, block_ids))
        }
        StateMachine::Syncing { initial_state, .. } => {
            let blocks: Vec<_> = block_ids
                .iter()
                .cloned()
                .flatten()
                .map(|i| {
                    warn!("TODO: Properly handle invalid requests"); // Return None if i >= metadata.block_count()?
                    let start: usize = (i * BLOCK_SIZE) as usize;
                    let end = std::cmp::min(((i + 1) * BLOCK_SIZE) as usize, initial_state.len());
                    Some(initial_state[start..end].to_vec())
                })
                .collect();
            Some(blocks)
        }
    }
}

fn handle_peer_request_range_helper<T: Clone>(slice: &[T], ids: &[Range<u64>]) -> Vec<T> {
    warn!("TODO: check ranges are in bounds or return error");
    let hashes: Vec<_> = ids
        .iter()
        .cloned()
        .flatten()
        .map(|i| {
            slice
                .get(i as usize)
                .expect("TODO: Properly handle invalid requests")
                .clone()
        })
        .collect();
    hashes
}
