use bitvec::{BitArr, prelude::Msb0};
use rand::{thread_rng, Rng};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;

use crate::{
    network::{
        multiplexer::Party,
        protocol::{receive, send, MiniProtocol},
    },
    util::{Channel, Hash, Sha256Hash, Stream},
};

// MiniProtocol instance for stream/connection management.
pub(crate) struct Manager {
    party_with_initiative: Party,
}

impl Manager {
    pub(crate) fn new(initiative: Party) -> Manager {
        Manager {
            party_with_initiative: initiative,
        }
    }

    fn server_has_initiative(&self) -> bool {
        match self.party_with_initiative {
            Party::Client => false,
            Party::Server => true,
        }
    }
}

impl MiniProtocol for Manager {
    type Message = MsgManager;

    fn run_server<S: Stream<MsgManager>>(self, mut stream: S) -> impl Future<Output = ()> + Send {
        async move {
            if self.server_has_initiative() {
                run_with_initiative(stream).await
            } else {
                run_without_initiative(stream).await
            }
        }
    }

    fn run_client<S: Stream<MsgManager>>(self, mut stream: S) -> impl Future<Output = ()> + Send {
        async move {
            if self.server_has_initiative() {
                run_without_initiative(stream).await
            } else {
                run_with_initiative(stream).await
            }
        }
    }
}

fn run_with_initiative<S: Stream<MsgManager>>(mut stream: S) -> impl Future<Output = ()> + Send {
    async move {
        println!("Mux manager started with initiative!");

        // Advertise stores.
        let stores = todo!();
        let shared_stores = run_advertise_stores_server(&mut stream, stores).await;

        loop {
            todo!()
        }
    }
}

type StoreId = Sha256Hash; // TODO
async fn run_advertise_stores_server<S: Stream<MsgManager>>(stream: &mut S, store_ids: Vec<StoreId>) -> Vec<StoreId> {
    // TODO: Prioritize and choose stores.


    // Send store advertising request.
    let nonce = thread_rng().gen();

    // Truncate stores length to MAX_ADVERTISE_STORES.
    let hashed_store_ids = store_ids.iter().take(MAX_ADVERTISE_STORES).map(|store_id| {
        let mut h = <Sha256Hash as Hash>::new();
        <Sha256Hash as Hash>::update(&mut h, nonce);
        <Sha256Hash as Hash>::update(&mut h, store_id);
        <Sha256Hash as Hash>::finalize(h)
    }).collect();
    let req = MsgManagerRequest::AdvertiseStores {
        nonce,
        store_ids: hashed_store_ids,
    };
    send(stream, req).await.expect("TODO");

    // Wait for response.
    let response: MsgManagerAdvertiseStoresResponse = receive(stream).await.expect("TODO");

    store_ids.into_iter().zip(response.have_stores).filter_map(|(store_id, is_shared)| if is_shared { Some(store_id) } else { None }).collect()
}

async fn run_advertise_stores_client<S: Stream<MsgManager>>(stream: &mut S, nonce: [u8; 4], their_store_ids: Vec<StoreId>, our_store_ids: Vec<StoreId>) -> BTreeSet<StoreId> {
    let our_store_ids: BTreeMap<Sha256Hash, StoreId> = our_store_ids.into_iter().map(|store_id| {
        let mut h = <Sha256Hash as Hash>::new();
        <Sha256Hash as Hash>::update(&mut h, nonce);
        <Sha256Hash as Hash>::update(&mut h, store_id);
        let h = <Sha256Hash as Hash>::finalize(h);
        (h, store_id)
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

fn run_without_initiative<S: Stream<MsgManager>>(mut stream: S) -> impl Future<Output = ()> + Send {
    async move {
        println!("Mux manager started without initiative!");
        
        loop {
        // Receive requests from initiator.
            let response: MsgManagerRequest = receive(&mut stream).await.expect("TODO");
            match response {
                MsgManagerRequest::AdvertiseStores { nonce, store_ids } => {
                    let stores = todo!();
                    let shared_stores = run_advertise_stores_client(&mut stream, nonce, store_ids, stores).await;
                    todo!()
                }
            }
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) enum MsgManager {
    Request(MsgManagerRequest),
    AdvertiseStoresResponse(MsgManagerAdvertiseStoresResponse),
}


#[derive(Debug, Serialize, Deserialize)]
enum MsgManagerRequest {
    // Advertise stores using a poor man's PSI.
    AdvertiseStores {
        nonce: [u8; 4],
        store_ids: Vec<Sha256Hash>, // Hash of store ids, using the nonce as a salt.
    },
}

pub const MAX_ADVERTISE_STORES: usize = 256;
pub type StoreBitmap = BitArr!(for MAX_ADVERTISE_STORES, in u8, Msb0);
#[derive(Debug, Serialize, Deserialize)]
struct MsgManagerAdvertiseStoresResponse {
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
