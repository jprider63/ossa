use bitvec::{BitArr, prelude::Msb0};
use rand::{thread_rng, Rng};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::UnboundedSender;
use tracing::{debug, error};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Debug;
use std::future::Future;
use tokio::sync::watch;
use tokio::time::{sleep, Duration};

use crate::auth::DeviceId;
use crate::core::{OdysseyType, StoreStatus, StoreStatuses};
use crate::store::UntypedStoreCommand;
use crate::{
    network::{
        multiplexer::Party,
        protocol::{receive, send, MiniProtocol},
    },
    util::{Channel, Hash, Sha256Hash, Stream},
};

// MiniProtocol instance for stream/connection management.
pub(crate) struct Manager<StoreId> {
    party_with_initiative: Party,
    peer_id: DeviceId, // DeviceId of peer we're connected to.
    active_stores: watch::Receiver<StoreStatuses<StoreId>>,
}

impl<StoreId> Manager<StoreId> {
    pub(crate) fn new(initiative: Party, peer_id: DeviceId, active_stores: watch::Receiver<StoreStatuses<StoreId>>) -> Manager<StoreId> {
        Manager {
            party_with_initiative: initiative,
            peer_id,
            active_stores,
        }
    }

    fn server_has_initiative(&self) -> bool {
        match self.party_with_initiative {
            Party::Client => false,
            Party::Server => true,
        }
    }
}

impl<StoreId: Send + Sync + Copy + AsRef<[u8]> + Ord + Debug> MiniProtocol for Manager<StoreId> {
    type Message = MsgManager;

    fn run_server<S: Stream<Self::Message>>(self, stream: S) -> impl Future<Output = ()> + Send {
        async move {
            if self.server_has_initiative() {
                self.run_with_initiative(stream).await
            } else {
                self.run_without_initiative(stream).await
            }
            debug!("Manager server exiting");
        }
    }

    fn run_client<S: Stream<Self::Message>>(self, stream: S) -> impl Future<Output = ()> + Send {
    // fn run_client<S: Stream<MsgManager>>(self, mut stream: S, active_stores: watch::Receiver<BTreeSet<StoreId>>) -> impl Future<Output = ()> + Send {
        async move {
            if self.server_has_initiative() {
                self.run_without_initiative(stream).await
            } else {
                // Sleep for 5 seconds to not duplicate effort from the server.
                sleep(Duration::new(5, 0)).await;
                self.run_with_initiative(stream).await
            }
            debug!("Manager client exiting");
        }
    }
}

impl<StoreId: Send + Sync + Copy + AsRef<[u8]> + Ord + Debug> Manager<StoreId> {
    /// Manager run in mode that sends requests to peer.
    fn run_with_initiative<S: Stream<MsgManager>>(mut self, mut stream: S) -> impl Future<Output = ()> + Send {
        async move {
            debug!("Mux manager started with initiative!");
    
            // Advertise stores.
            let shared_stores = run_advertise_stores_server(&mut stream, &mut self.active_stores).await;
            handle_shared_stores(self.peer_id, shared_stores);
    
            loop {
                debug!("Mux manager looping with initiative!");
                tokio::select! {
                    changed_e = self.active_stores.changed() => {
                        changed_e.expect("TODO");
    
                        let shared_stores = run_advertise_stores_server(&mut stream, &mut self.active_stores).await;
                        handle_shared_stores(self.peer_id, shared_stores);
                    }
                }
            }
        }
    }

    /// Manager run in mode that responds to requests from peer.
    fn run_without_initiative<S: Stream<MsgManager>>(mut self, mut stream: S) -> impl Future<Output = ()> + Send {
        async move {
            debug!("Mux manager started without initiative!");
            
            loop {
                debug!("Mux manager looping without initiative!");
                // Receive requests from initiator.
                let response: MsgManagerRequest = receive(&mut stream).await.expect("TODO");
                match response {
                    MsgManagerRequest::AdvertiseStores { nonce, store_ids } => {
                        let shared_stores = run_advertise_stores_client(&mut stream, nonce, store_ids, &mut self.active_stores).await;
                        // TODO: Store and handle peers too?
                        debug!("Sent server store ids: {:?}", shared_stores);
                    }
                }
            }
        }
    }

}


// Server                         Client
//        ----- My StoreIds ---->
//        <---- My StoreIds -----
//
// Store (Peer, Vec<StoreId>) in peer thread? Probably in watch::Sender<Map<PeerId, Vec<StoreId>>>
// Or: watch::Sender<Map<StoreId, Vec<PeerId>>?
// Or: Map<StoreId, watch::Sender<Set<PeerId>>? ***
// Or: Spawn sync threads for each shared store.

fn handle_shared_stores<StoreId>(
    peer_id: DeviceId,
    shared_stores: Vec<(StoreId, UnboundedSender<UntypedStoreCommand>)>,
) {
    // Register the peer for this store.
    let peers = vec![peer_id];
    let cmd = UntypedStoreCommand::RegisterPeers { peers };

    for (_store_id, store_sender) in shared_stores {
        let res = store_sender.send(cmd.clone()); // .expect("TODO");
        if res.is_err() {
            error!("Failed to register peer: {:?}", res);
        }
    }
}

fn hash_store_id_with_nonce<StoreId: AsRef<[u8]>>(nonce: [u8; 4], store_id: &StoreId) -> Sha256Hash {
    let mut h = <Sha256Hash as Hash>::new();
    <Sha256Hash as Hash>::update(&mut h, nonce);
    <Sha256Hash as Hash>::update(&mut h, store_id);
    <Sha256Hash as Hash>::finalize(h)
}

async fn run_advertise_stores_server<S: Stream<MsgManager>, StoreId>(
    stream: &mut S,
    store_ids: &mut watch::Receiver<StoreStatuses<StoreId>>
) -> Vec<(StoreId, UnboundedSender<UntypedStoreCommand>)>
where
    StoreId: Copy + AsRef<[u8]>,
{
    // TODO: Prioritize and choose stores.
    // Truncate stores length to MAX_ADVERTISE_STORES.
    let store_ids:Vec<_> = store_ids.borrow_and_update().iter().filter_map(|e| e.1.command_channel().map(|c| (e.0, c))).take(MAX_ADVERTISE_STORES).map(|(&s, c)| (s, c.clone())).collect();

    // Send store advertising request.
    let nonce = thread_rng().gen();

    let hashed_store_ids = store_ids.iter().map(|(store_id, _)|
        hash_store_id_with_nonce(nonce, store_id)
    ).collect();
    let req = MsgManagerRequest::AdvertiseStores {
        nonce,
        store_ids: hashed_store_ids,
    };
    send(stream, req).await.expect("TODO");

    // Wait for response.
    let response: MsgManagerAdvertiseStoresResponse = receive(stream).await.expect("TODO");

    store_ids.into_iter().zip(response.have_stores).filter_map(|((store_id, chan), is_shared)| if is_shared { Some((store_id, chan)) } else { None }).collect()
}

async fn run_advertise_stores_client<S: Stream<MsgManager>, StoreId: Copy + Ord + AsRef<[u8]>>(stream: &mut S, nonce: [u8; 4], their_store_ids: Vec<Sha256Hash>, our_store_ids: &mut watch::Receiver<StoreStatuses<StoreId>>) -> BTreeSet<StoreId> {
    let our_store_ids: BTreeMap<Sha256Hash, StoreId> = our_store_ids.borrow_and_update().iter().filter(|e| e.1.is_initialized()).map(|(store_id, _)| {
        let h = hash_store_id_with_nonce(nonce, store_id);
        (h, *store_id)
    }).collect();

    let mut have_stores = StoreBitmap::ZERO;
    let mut mutual_store_ids = BTreeSet::new();
    their_store_ids.iter().enumerate().for_each(|(i, their_store_id)| {
        if let Some(v) = our_store_ids.get(their_store_id) {
            have_stores.set(i, true);
            mutual_store_ids.insert(*v);
        }
    });

    let response = MsgManagerAdvertiseStoresResponse {
        have_stores,
    };
    send(stream, response).await.expect("TODO");

    mutual_store_ids
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) enum MsgManager {
    Request(MsgManagerRequest),
    AdvertiseStoresResponse(MsgManagerAdvertiseStoresResponse),
}


#[derive(Debug, Serialize, Deserialize)]
pub(crate) enum MsgManagerRequest {
    // Advertise stores using a poor man's PSI.
    AdvertiseStores {
        nonce: [u8; 4],
        store_ids: Vec<Sha256Hash>,
        // Hash of store ids, using the nonce as a salt.
        // Maximum length is MAX_ADVERTISE_STORES.
        // TODO: Verify length while parsing.
    },
}

pub const MAX_ADVERTISE_STORES: usize = 256;
pub type StoreBitmap = BitArr!(for MAX_ADVERTISE_STORES, in u8, Msb0);
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct MsgManagerAdvertiseStoresResponse {
    have_stores: StoreBitmap,
}

impl Into<MsgManager> for MsgManagerRequest {
    fn into(self) -> MsgManager {
        MsgManager::Request(self)
    }
}

impl Into<MsgManager> for MsgManagerAdvertiseStoresResponse {
    fn into(self) -> MsgManager {
        MsgManager::AdvertiseStoresResponse(self)
    }
}

impl TryInto<MsgManagerRequest> for MsgManager {
    type Error = ();
    fn try_into(self) -> Result<MsgManagerRequest, ()> {
        match self {
            MsgManager::Request(r) => Ok(r),
            MsgManager::AdvertiseStoresResponse(_) => Err(()),
        }
    }
}

impl TryInto<MsgManagerAdvertiseStoresResponse> for MsgManager {
    type Error = ();
    fn try_into(self) -> Result<MsgManagerAdvertiseStoresResponse, ()> {
        match self {
            MsgManager::Request(_) => Err(()),
            MsgManager::AdvertiseStoresResponse(r) => Ok(r),
        }
    }
}
