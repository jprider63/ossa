use std::{fmt::Debug, future::Future, marker::PhantomData, ops::Range};

use bitvec::{order::Msb0, BitArr};
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc::{UnboundedReceiver, UnboundedSender}, oneshot};
use tracing::{debug, warn};

use crate::{auth::DeviceId, network::protocol::{receive, send, MiniProtocol}, protocol::store_peer::ecg_sync::ECGSyncServer, store::{self, ecg, ECGStatus, HandlePeerRequest, UntypedStoreCommand}, util::Stream};


#[derive(Debug, Serialize, Deserialize)]
pub(crate) enum MsgStoreSync<Hash, HeaderId, Header> {
    Request(MsgStoreSyncRequest<HeaderId>),
    MetadataHeaderResponse(MsgStoreSyncMetadataResponse<Hash>),
    MerkleResponse(MsgStoreSyncMerkleResponse<Hash>),
    PiecesResponse(MsgStoreSyncPieceResponse),
    ECGResponse(MsgStoreECGSyncResponse<HeaderId, Header>),
}

pub const MAX_HAVE_HEADERS: u16 = 32;
pub type HeaderBitmap = BitArr!(for MAX_HAVE_HEADERS as usize, in u8, Msb0);
#[derive(Debug, Serialize, Deserialize)]
pub(crate) enum MsgStoreSyncRequest<HeaderId> {
    MetadataHeader,
    MerkleHashes {
        ranges: Vec<Range<u64>>,
    },
    InitialStatePieces {
        ranges: Vec<Range<u64>>,
    },
    ECGInitialSync {
        tips: Vec<HeaderId>,
    },
    ECGSync {
        tips: Vec<HeaderId>,
        known: HeaderBitmap,
    },
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct MsgStoreSyncMetadataResponse<Hash> (StoreSyncResponse<store::v0::MetadataHeader<Hash>>);

#[derive(Debug, Serialize, Deserialize)]
pub(crate) enum StoreSyncResponse<A> {
    Response(A),
    // Don't currently know, wait until we share it with you.
    Wait,
    // They may or may not have the metadata, but they rejected the request.
    Reject,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct MsgStoreSyncMerkleResponse<Hash> (StoreSyncResponse<Vec<Hash>>);

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct MsgStoreSyncPieceResponse (StoreSyncResponse<Vec<Option<Vec<u8>>>>);

pub(crate) type RawECGBody = Vec<u8>; // Serialized ECG body
#[derive(Debug, Serialize, Deserialize)]
pub(crate) enum MsgStoreECGSyncResponse<HeaderId, Header> {
    Response {
        have: Vec<HeaderId>,
        operations: Vec<(Header, RawECGBody)>, // ECG headers and serialized ECG body.
    },
    Wait, // JP: Use StoreSyncResponse?
}

impl<Hash, HeaderId, Header> Into<MsgStoreSync<Hash, HeaderId, Header>> for MsgStoreSyncRequest<HeaderId> {
    fn into(self) -> MsgStoreSync<Hash, HeaderId, Header> {
        MsgStoreSync::Request(self)
    }
}

impl<Hash, HeaderId, Header> TryInto<MsgStoreSyncRequest<HeaderId>> for MsgStoreSync<Hash, HeaderId, Header> {
    type Error = ();
    fn try_into(self) -> Result<MsgStoreSyncRequest<HeaderId>, ()> {
        match self {
            MsgStoreSync::Request(r) => Ok(r),
            MsgStoreSync::MetadataHeaderResponse(_) => Err(()),
            MsgStoreSync::MerkleResponse(_) => Err(()),
            MsgStoreSync::PiecesResponse(_) => Err(()),
            MsgStoreSync::ECGResponse(_) => Err(()),
        }
    }
}

impl<Hash, HeaderId, Header> Into<MsgStoreSync<Hash, HeaderId, Header>> for MsgStoreSyncMetadataResponse<Hash> {
    fn into(self) -> MsgStoreSync<Hash, HeaderId, Header> {
        MsgStoreSync::MetadataHeaderResponse(self)
    }
}

impl<Hash, HeaderId, Header> TryInto<MsgStoreSyncMetadataResponse<Hash>> for MsgStoreSync<Hash, HeaderId, Header> {
    type Error = ();
    fn try_into(self) -> Result<MsgStoreSyncMetadataResponse<Hash>, ()> {
        match self {
            MsgStoreSync::Request(_) => Err(()),
            MsgStoreSync::MetadataHeaderResponse(r) => Ok(r),
            MsgStoreSync::MerkleResponse(_) => Err(()),
            MsgStoreSync::PiecesResponse(_) => Err(()),
            MsgStoreSync::ECGResponse(_) => Err(()),
        }
    }
}

impl<Hash, HeaderId, Header> Into<MsgStoreSync<Hash, HeaderId, Header>> for MsgStoreSyncMerkleResponse<Hash> {
    fn into(self) -> MsgStoreSync<Hash, HeaderId, Header> {
        MsgStoreSync::MerkleResponse(self)
    }
}

impl<Hash, HeaderId, Header> TryInto<MsgStoreSyncMerkleResponse<Hash>> for MsgStoreSync<Hash, HeaderId, Header> {
    type Error = ();
    fn try_into(self) -> Result<MsgStoreSyncMerkleResponse<Hash>, ()> {
        match self {
            MsgStoreSync::Request(_) => Err(()),
            MsgStoreSync::MetadataHeaderResponse(_) => Err(()),
            MsgStoreSync::MerkleResponse(r) => Ok(r),
            MsgStoreSync::PiecesResponse(_) => Err(()),
            MsgStoreSync::ECGResponse(_) => Err(()),
        }
    }
}

impl<Hash, HeaderId, Header> Into<MsgStoreSync<Hash, HeaderId, Header>> for MsgStoreSyncPieceResponse {
    fn into(self) -> MsgStoreSync<Hash, HeaderId, Header> {
        MsgStoreSync::PiecesResponse(self)
    }
}

impl<Hash, HeaderId, Header> TryInto<MsgStoreSyncPieceResponse> for MsgStoreSync<Hash, HeaderId, Header> {
    type Error = ();
    fn try_into(self) -> Result<MsgStoreSyncPieceResponse, ()> {
        match self {
            MsgStoreSync::Request(_) => Err(()),
            MsgStoreSync::MetadataHeaderResponse(_) => Err(()),
            MsgStoreSync::MerkleResponse(_) => Err(()),
            MsgStoreSync::PiecesResponse(r) => Ok(r),
            MsgStoreSync::ECGResponse(_) => Err(()),
        }
    }
}

impl<Hash, HeaderId, Header> Into<MsgStoreSync<Hash, HeaderId, Header>> for MsgStoreECGSyncResponse<HeaderId, Header> {
    fn into(self) -> MsgStoreSync<Hash, HeaderId, Header> {
        MsgStoreSync::ECGResponse(self)
    }
}

impl<Hash, HeaderId, Header> TryInto<MsgStoreECGSyncResponse<HeaderId, Header>> for MsgStoreSync<Hash, HeaderId, Header> {
    type Error = ();
    fn try_into(self) -> Result<MsgStoreECGSyncResponse<HeaderId, Header>, ()> {
        match self {
            MsgStoreSync::Request(_) => Err(()),
            MsgStoreSync::MetadataHeaderResponse(_) => Err(()),
            MsgStoreSync::MerkleResponse(_) => Err(()),
            MsgStoreSync::PiecesResponse(_) => Err(()),
            MsgStoreSync::ECGResponse(r) => Ok(r),
        }
    }
}

pub(crate) struct StoreSync<Hash, HeaderId, Header> {
    peer: DeviceId,
    // Receive commands from store if we have initiatives or send commands to store if we're the responder.
    recv_chan: Option<UnboundedReceiver<StoreSyncCommand<HeaderId, Header>>>,
    // Send commands to store if we're the responder and send results back to store if we're the initiator.
    send_chan: UnboundedSender<UntypedStoreCommand<Hash, HeaderId, Header>>, // JP: Make this a stream?
}

impl<Hash, HeaderId, Header> StoreSync<Hash, HeaderId, Header> {
    pub(crate) fn new_server(peer: DeviceId, recv_chan: UnboundedReceiver<StoreSyncCommand<HeaderId, Header>>, send_chan: UnboundedSender<UntypedStoreCommand<Hash, HeaderId, Header>>) -> Self {
        let recv_chan = Some(recv_chan);
        Self { peer, recv_chan, send_chan }
    }

    pub(crate) fn new_client(peer: DeviceId, send_chan: UnboundedSender<UntypedStoreCommand<Hash, HeaderId, Header>>) -> Self {
        Self { peer, recv_chan: None, send_chan }
    }



    #[inline(always)]
    async fn run_client_helper<S: Stream<MsgStoreSync<Hash, HeaderId, Header>>, Req, Resp, SResp>(&self, stream: &mut S, request: Req, build_command: fn(HandlePeerRequest<Req, Resp>) -> UntypedStoreCommand<Hash, HeaderId, Header>, build_response: fn(StoreSyncResponse<Resp>) -> SResp)
    where
        Hash: Debug,
        SResp: Into<MsgStoreSync<Hash, HeaderId, Header>> + Debug,
        Resp: Debug,
    {
        // Send request to store.
        let (response_chan, recv_chan) = oneshot::channel();
        let cmd = build_command(HandlePeerRequest {
            peer: self.peer,
            request,
            response_chan,
        });
        self.send_chan.send(cmd).expect("TODO");

        // Wait for response.
        match recv_chan.await.expect("TODO") {
            Ok(response) => {
                // Send response to peer.
                let response = build_response(StoreSyncResponse::Response(response));
                debug!("Sending response to peer ({}): {response:?}", self.peer);
                send(stream, response).await.expect("TODO");
            }
            Err(chan) => {
                // If waiting, tell peer.
                debug!("Telling peer to wait for response ({})", self.peer);
                send(stream, MsgStoreSyncMetadataResponse(StoreSyncResponse::Wait)).await.expect("TODO");

                // Wait for response and send to peer.
                let response = if let Some(res) = chan.await.expect("TODO") {
                    debug!("Sending response to peer ({}): {res:?}", self.peer);
                    StoreSyncResponse::Response(res)
                } else {
                    debug!("Not sending response to peer ({})", self.peer);
                    StoreSyncResponse::Reject
                };

                send(stream, build_response(response)).await.expect("TODO");
            }
        }
    }

}

pub(crate) enum StoreSyncCommand<HeaderId, Header> {
    MetadataHeaderRequest,
    MerkleRequest(Vec<Range<u64>>),
    InitialStatePieceRequest(Vec<Range<u64>>),
    ECGSyncRequest{
        ecg_status: ECGStatus<HeaderId>,
        ecg_state: ecg::UntypedState<HeaderId, Header>,
    },
}

impl<Hash: Debug + Serialize + for<'a> Deserialize<'a> + Send + Sync + Clone, HeaderId: Debug + Ord + Serialize + for<'a> Deserialize<'a> + Send + Sync + Clone, Header: Debug + Send + Sync + Serialize + for<'a> Deserialize<'a>> MiniProtocol for StoreSync<Hash, HeaderId, Header> {
    type Message = MsgStoreSync<Hash, HeaderId, Header>;

    // Has initiative
    fn run_server<S: Stream<Self::Message>>(self, mut stream: S) -> impl Future<Output = ()> + Send {
        async move {
            let mut ecg_sync: Option<ECGSyncServer<Hash, HeaderId, Header>> = None;

            // Wait for command from store.
            let mut recv_chan = self.recv_chan.expect("Unreachable. Server must be given a receive channel.");
            while let Some(cmd) = recv_chan.recv().await {
                match cmd {
                    StoreSyncCommand::MetadataHeaderRequest => {
                        send(&mut stream, MsgStoreSyncRequest::MetadataHeader).await.expect("TODO");

                        let MsgStoreSyncMetadataResponse(result) = receive(&mut stream).await.expect("TODO");
                        match result {
                            StoreSyncResponse::Response(metadata) => {
                                // Send store the metadata and tell store we're ready.
                                let msg = UntypedStoreCommand::ReceivedMetadata { peer: self.peer, metadata };
                                self.send_chan.send(msg).expect("TODO");
                            }
                            StoreSyncResponse::Wait => {
                                // Wait for response (Must be Cancel or MetadataHeader).
                                todo!();
                            }
                            StoreSyncResponse::Reject => {
                                // Send store they canceled and tell store we're ready.
                                todo!();
                            }
                        }
                    }
                    StoreSyncCommand::MerkleRequest(ranges) => {
                        send(&mut stream, MsgStoreSyncRequest::MerkleHashes{ ranges: ranges.clone() }).await.expect("TODO");

                        let MsgStoreSyncMerkleResponse(result) = receive(&mut stream).await.expect("TODO");
                        match result {
                            StoreSyncResponse::Response(pieces) => {
                                // Send store the piece hashes and tell store we're ready.
                                let msg = UntypedStoreCommand::ReceivedMerkleHashes {peer: self.peer, ranges, pieces};
                                self.send_chan.send(msg).expect("TODO");
                            }
                            StoreSyncResponse::Wait => {
                                // Wait for response (Must be Cancel or MerklePieces).
                                todo!();
                            }
                            StoreSyncResponse::Reject => {
                                // Send store they canceled and tell store we're ready.
                                todo!();
                            }
                        }
                    }
                    StoreSyncCommand::InitialStatePieceRequest(ranges) => {
                        send(&mut stream, MsgStoreSyncRequest::InitialStatePieces { ranges: ranges.clone() }).await.expect("TODO");
                        let MsgStoreSyncPieceResponse(result) = receive(&mut stream).await.expect("TODO");
                        match result {
                            StoreSyncResponse::Response(pieces) => {
                                // Send store the pieces and tell store we're ready.
                                let msg = UntypedStoreCommand::ReceivedInitialStatePieces {peer: self.peer, ranges, pieces};
                                self.send_chan.send(msg).expect("TODO");
                            }
                            StoreSyncResponse::Wait =>
                                // Wait for response (Must be Reject or Repsonse).
                                todo!(),
                            StoreSyncResponse::Reject =>
                                // Send store they rejected and tell store we're ready.
                                todo!(),
                        }
                    }
                    StoreSyncCommand::ECGSyncRequest { ecg_status, ecg_state } => {
                        // if ecg_status.meet_needs_update {
                        //     // TODO: Actually figure out meet.
                        //     warn!("TODO: Actually figure out meet");
                        //     let meet = ecg_status.meet;
                        //     let msg = UntypedStoreCommand::ReceivedUpdatedMeet { peer: self.peer, meet};
                        //     self.send_chan.send(msg).expect("TODO");
                        // } else {
                        //     // TODO: Only request updates if they have a tip we don't know or if they're caught up to us?
                        //     warn!("TODO: Only request updates if they have a tip we don't know or if they're caught up to us?");

                        //     let tip = ecg_state.tips().iter().cloned().collect();
                        //     let msg = MsgStoreSyncRequest::ECGSync { meet: ecg_status.meet, tip };
                        //     send(&mut stream, msg).await.expect("TODO");
                        //     let MsgStoreECGSyncResponse(result) = receive(&mut stream).await.expect("TODO");
                        //     match result {
                        //         StoreSyncResponse::Response(operations) => {
                        //             todo!();
                        //         }
                        //         StoreSyncResponse::Wait =>
                        //             // Wait for response (Must be Reject or Repsonse).
                        //             todo!(),
                        //         StoreSyncResponse::Reject =>
                        //             // Send store they rejected and tell store we're ready.
                        //             todo!(),
                        //     }
                        // }

                        let operations = match ecg_sync {
                            None => {
                                // First round of ECG sync, so create and run first round.

                                // JP: Eventually switch ecg_state to an Arc<RWLock>?
                                let (new_ecg_sync, operations) = ECGSyncServer::run_new(&mut stream, ecg_state).await;
                                ecg_sync = Some(new_ecg_sync);
                                operations
                            }
                            Some(ref mut ecg_sync) => {
                                // Subsequent rounds of ECG sync.
                                ecg_sync.run_round(&mut stream, &ecg_state).await
                            }
                        };

                        let msg = UntypedStoreCommand::ReceivedECGOperations { peer: self.peer, operations};
                        self.send_chan.send(msg).expect("TODO");
                    }
                }
            }

            debug!("StoreSyncCommand receiver channel closed");



            // ??
            // Send our store's status.
            // Wait for their status.
            //
            // Status:
            // | DownloadingHeader
            // | DownloadingInitialState {BlocksIds}
            // | Syncing {ecg_known: frontier, ecg_have: frontier}

        }
    }

    fn run_client<S: Stream<Self::Message>>(self, mut stream: S) -> impl Future<Output = ()> + Send {
        async move {
            // TODO: Check when done.
            loop {
                // Receive request.
                let request = receive(&mut stream).await.expect("TODO");
                match request {
                    MsgStoreSyncRequest::MetadataHeader => {
                        const fn build_command<Hash, HeaderId, Header>(req: HandlePeerRequest<(), store::v0::MetadataHeader<Hash>>) -> UntypedStoreCommand<Hash, HeaderId, Header> {
                            UntypedStoreCommand::HandleMetadataPeerRequest(req)
                        }
                        const fn build_response<Hash, HeaderId, Header>(h: StoreSyncResponse<store::v0::MetadataHeader<Hash>>) -> MsgStoreSyncMetadataResponse<Hash> {
                            MsgStoreSyncMetadataResponse(h)
                        }

                        self.run_client_helper::<_, (), store::v0::MetadataHeader<Hash>, MsgStoreSyncMetadataResponse<Hash>>(&mut stream, (), build_command, build_response::<_, HeaderId, Header>).await;
                    }
                    MsgStoreSyncRequest::MerkleHashes { ranges } => {
                        const fn build_command<Hash, HeaderId, Header>(req: HandlePeerRequest<Vec<Range<u64>>, Vec<Hash>>) -> UntypedStoreCommand<Hash, HeaderId, Header> {
                            UntypedStoreCommand::HandleMerklePeerRequest(req)
                        }
                        const fn build_response<H>(h: StoreSyncResponse<Vec<H>>) -> MsgStoreSyncMerkleResponse<H> {
                            MsgStoreSyncMerkleResponse(h)
                        }

                        self.run_client_helper::<_, Vec<Range<u64>>, Vec<Hash>, MsgStoreSyncMerkleResponse<Hash>>(&mut stream, ranges, build_command, build_response).await;
                    }
                    MsgStoreSyncRequest::InitialStatePieces { ranges } => {
                        const fn build_command<Hash, HeaderId, Header>(req: HandlePeerRequest<Vec<Range<u64>>, Vec<Option<Vec<u8>>>>) -> UntypedStoreCommand<Hash, HeaderId, Header> {
                            UntypedStoreCommand::HandlePiecePeerRequest(req)
                        }
                        const fn build_response(h: StoreSyncResponse<Vec<Option<Vec<u8>>>>) -> MsgStoreSyncPieceResponse {
                            MsgStoreSyncPieceResponse(h)
                        }

                        self.run_client_helper::<_, Vec<Range<u64>>, Vec<Option<Vec<u8>>>, MsgStoreSyncPieceResponse>(&mut stream, ranges, build_command, build_response).await;
                    }
                    MsgStoreSyncRequest::ECGInitialSync { tips } => {
                        // const fn build_command<Hash, HeaderId, Header>(req: HandlePeerRequest<(Vec<HeaderId>, Vec<HeaderId>), Vec<(Header, RawECGBody)>>) -> UntypedStoreCommand<Hash, HeaderId, Header> {
                        //     UntypedStoreCommand::HandleECGSyncRequest(req)
                        // }
                        // const fn build_response<Header>(h: StoreSyncResponse<Vec<(Header, RawECGBody)>>) -> MsgStoreECGSyncResponse<Header> {
                        //     MsgStoreECGSyncResponse(h)
                        // }
                        // self.run_client_helper::<_, _, _, _>(&mut stream, (meet, tip), build_command, build_response).await;
                        todo!();
                    }
                    MsgStoreSyncRequest::ECGSync { tips, known } => {
                        todo!();
                    }
                }
            }
        }
    }
}
