use serde::{Deserialize, Serialize};
use std::future::Future;

use crate::{
    network::{
        multiplexer::Party,
        protocol::{receive, send, MiniProtocol},
    },
    util::{Channel, Stream},
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
                run_with_initiative().await
            } else {
                run_without_initiative().await
            }
        }
    }

    fn run_client<S: Stream<MsgManager>>(self, mut stream: S) -> impl Future<Output = ()> + Send {
        async move {
            if self.server_has_initiative() {
                run_without_initiative().await
            } else {
                run_with_initiative().await
            }
        }
    }
}

fn run_with_initiative() -> impl Future<Output = ()> + Send {
    async move {
        println!("Mux manager started with initiative!");

        loop {
            todo!()
        }
    }
}

fn run_without_initiative() -> impl Future<Output = ()> + Send {
    async move {
        println!("Mux manager started without initiative!");
        todo!()
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) enum MsgManager {}
