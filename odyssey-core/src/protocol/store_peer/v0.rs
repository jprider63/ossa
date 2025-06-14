use std::future::Future;

use serde::{Deserialize, Serialize};

use crate::{network::protocol::MiniProtocol, util::Stream};


#[derive(Debug, Serialize, Deserialize)]
pub(crate) enum MsgStoreSync {
}

pub(crate) struct StoreSync {}
impl MiniProtocol for StoreSync {
    type Message = MsgStoreSync;

    // Has initiative
    fn run_server<S: Stream<Self::Message>>(self, stream: S) -> impl Future<Output = ()> + Send {
        async move {
            // ??
            // Send our store's status.
            // Wait for their status.
            //
            // Status:
            // | DownloadingHeader
            // | DownloadingInitialState {BlocksIds}
            // | Syncing {ecg_known: frontier, ecg_have: frontier}


            todo!();
        }
    }

    fn run_client<S: Stream<Self::Message>>(self, stream: S) -> impl Future<Output = ()> + Send {
        async move {
            todo!();
        }
    }
}
