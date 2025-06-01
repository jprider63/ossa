use odyssey_crdt::CRDT;
use serde::Serialize;
use std::collections::BTreeSet;
use typeable::{TypeId, Typeable};

use crate::{core::OdysseyType, store::ecg::{ECGBody, ECGHeader}, util};

pub mod ecg;
pub mod v0; // TODO: Move this to network::protocol

pub use v0::{MetadataBody, MetadataHeader, Nonce};


// States are:
// - Initializing - Setting up the thread that owns the store (not defined here).
// - Downloading - Don't have the header so we're downloading it.
// - Syncing - Have the header and syncing updates between peers.
pub enum State<Header: ecg::ECGHeader<T>, T: CRDT, Hash> {
    Downloading {
        store_id: Hash,
    },
    Syncing {
        store_header: MetadataHeader<Hash>,
        ecg_state: ecg::State<Header, T>,
        decrypted_state: Option<DecryptedState<Header, T>>, // JP: Is this actually used?
                                                                       // Does it make sense?
    }
}

pub struct DecryptedState<Header: ecg::ECGHeader<T>, T: CRDT> {
    /// Latest ECG application state we've seen.
    latest_state: T,

    /// Headers corresponding to the latest ECG application state.
    latest_headers: BTreeSet<Header::HeaderId>,
}

impl<Header: ecg::ECGHeader<T>, T: CRDT, Hash: util::Hash> State<Header, T, Hash> {
    pub fn new(initial_state: T) -> State<Header, T, Hash>
    where
        T: Serialize + Typeable,
    {
        let init_body = MetadataBody::new(&initial_state);
        let store_header = MetadataHeader::generate::<T>(&init_body);
        let decrypted_state = DecryptedState {
            latest_state: initial_state,
            latest_headers: BTreeSet::new(),
        };

        State::Syncing {
            store_header,
            ecg_state: ecg::State::new(),
            decrypted_state: Some(decrypted_state),
        }
    }

    pub fn store_id(&self) -> Hash {
        match self {
            State::Downloading{store_id} => {
                *store_id
            }
            State::Syncing { store_header, .. } => {
                store_header.store_id()
            }
        }
    }
}

/// Run the handler that owns this store and manages its state. This handler is typically run in
/// its own tokio thread.
pub(crate) async fn run_handler<OT: OdysseyType, T: CRDT<Time = OT::Time> + Clone + Send + 'static>(
    mut store: State<OT::ECGHeader<T>, T, OT::StoreId>,
)
where
    <OT as OdysseyType>::ECGHeader<T>: Send + Clone + 'static,
    <<OT as OdysseyType>::ECGHeader<T> as ECGHeader<T>>::Body: ECGBody<T> + Send,
    <<OT as OdysseyType>::ECGHeader<T> as ECGHeader<T>>::HeaderId: Send,
    T::Op: Serialize,
{
    match store {
        State::Downloading {..} => {
            // Get peers that have this store too. ? Or the other thread will do this
            // automatically?

            // Wait until the store is downloaded.
            todo!("Wait until the store is downloaded")
            // Update state to be syncing.
        },
        State::Syncing {..} => {
            todo!()
        },
    }
}
