use std::collections::HashMap;

use adsb_index_api_types::{
    IcaoAddress,
    Squawk,
    live::{
        SubscriptionEvent,
        SubscriptionFilter,
    },
};
use serde::{
    Deserialize,
    Serialize,
};
use tokio::sync::{
    mpsc,
    oneshot,
};
use uuid::Uuid;

use crate::util::sparse_list::SparseList;

const COMMAND_QUEUE_SIZE: usize = 32;

#[derive(Debug, thiserror::Error)]
#[error("broker error")]
pub enum Error {
    #[error("reactor dead")]
    ReactorDead,
    #[error("invalid subscription: {id}")]
    InvalidSubscriptionId { client_id: usize, id: Uuid },
}

#[derive(Debug, Serialize, Deserialize)]
pub enum Event {}

#[derive(Clone, Debug)]
pub struct Broker {
    command_sender: mpsc::Sender<Command>,
}

impl Broker {
    pub fn new() -> Self {
        let (command_sender, command_receiver) = mpsc::channel(COMMAND_QUEUE_SIZE);

        tokio::spawn(async move {
            let reactor = Reactor {
                command_receiver,
                subscriptions: Default::default(),
            };
            reactor.run().await.expect("broker reactor error");
        });

        Self { command_sender }
    }

    async fn send_command(&self, command: Command) {
        self.command_sender
            .send(command)
            .await
            .expect("broker command channel closed");
    }

    pub async fn subscribe(
        &self,
        client_id: usize,
        id: Uuid,
        filter: SubscriptionFilter,
        message_sender: mpsc::Sender<SubscriptionMessage>,
    ) -> Result<(), Error> {
        let (result_sender, result_receiver) = oneshot::channel();

        self.send_command(Command::Subscribe {
            client_id,
            id,
            filter,
            message_sender,
            result_sender,
        })
        .await;

        result_receiver.await.expect("broker result channel closed")
    }

    pub async fn unsubscribe(&self, client_id: usize, id: Uuid) -> Result<(), Error> {
        let (result_sender, result_receiver) = oneshot::channel();

        self.send_command(Command::Unsubscribe {
            client_id,
            id,
            result_sender,
        })
        .await;

        result_receiver.await.expect("broker result channel closed")
    }
}

#[derive(Debug)]
struct Reactor {
    subscriptions: Subscriptions,

    command_receiver: mpsc::Receiver<Command>,
}

impl Reactor {
    async fn run(mut self) -> Result<(), Error> {
        while let Some(command) = self.command_receiver.recv().await {
            self.handle_command(command).await?;
        }

        Ok(())
    }

    async fn handle_command(&mut self, command: Command) -> Result<(), Error> {
        match command {
            Command::Subscribe {
                client_id,
                id,
                filter,
                message_sender: event_sender,
                result_sender,
            } => {
                let result = self
                    .subscriptions
                    .subscribe(client_id, id, filter, event_sender);
                let _ = result_sender.send(result);
            }
            Command::Unsubscribe {
                client_id,
                id,
                result_sender,
            } => {
                let result = self.subscriptions.unsubscribe(client_id, id);
                let _ = result_sender.send(result);
            }
            Command::Publish {} => {
                todo!();
            }
        }

        Ok(())
    }
}

#[derive(Debug)]
enum Command {
    Subscribe {
        client_id: usize,
        id: Uuid,
        filter: SubscriptionFilter,
        message_sender: mpsc::Sender<SubscriptionMessage>,
        result_sender: oneshot::Sender<Result<(), Error>>,
    },
    Unsubscribe {
        client_id: usize,
        id: Uuid,
        result_sender: oneshot::Sender<Result<(), Error>>,
    },
    Publish {
        // todo: an actual message to distribute to clients
    },
}

#[derive(Debug, Default)]
struct Subscriptions {
    subscriptions: SparseList<Subscription>,

    // key is (client id, client-chosen subscription id)
    by_subscriber_id: HashMap<(usize, Uuid), usize>,

    by_icao_address: HashMap<IcaoAddress, SparseList<usize>>,
    by_callsign: HashMap<String, SparseList<usize>>,
    by_squawk: HashMap<Squawk, SparseList<usize>>,
    // todo: by location -> r*tree
}

impl Subscriptions {
    pub fn subscribe(
        &mut self,
        client_id: usize,
        id: Uuid,
        filter: SubscriptionFilter,
        message_sender: mpsc::Sender<SubscriptionMessage>,
    ) -> Result<(), Error> {
        if let Some(_index) = self.by_subscriber_id.get(&(client_id, id)) {
            // todo: make this change the subscription
            self.unsubscribe(client_id, id)?;
        }

        let (index, subscription) = self.subscriptions.insert_and_get_mut(Subscription {
            client_id,
            id,
            message_sender,
            by_icao_address: Vec::with_capacity(filter.aircraft.icao.len()),
            by_callsign: Vec::with_capacity(filter.aircraft.callsign.len()),
            by_squawk: Vec::with_capacity(filter.aircraft.squawk.len()),
        });

        if filter.area.is_empty() {
            for icao_address in filter.aircraft.icao {
                let filter_index = self
                    .by_icao_address
                    .entry(icao_address)
                    .or_default()
                    .insert(index);
                subscription
                    .by_icao_address
                    .push((icao_address, filter_index));
            }
            for callsign in filter.aircraft.callsign {
                let filter_index = self
                    .by_callsign
                    .entry(callsign.clone())
                    .or_default()
                    .insert(index);
                subscription.by_callsign.push((callsign, filter_index));
            }
            for squawk in filter.aircraft.squawk {
                let filter_index = self.by_squawk.entry(squawk).or_default().insert(index);
                subscription.by_squawk.push((squawk, filter_index))
            }
        }
        else {
            todo!();
        }

        Ok(())
    }

    pub fn unsubscribe(&mut self, client_id: usize, id: Uuid) -> Result<(), Error> {
        let index = self
            .by_subscriber_id
            .remove(&(client_id, id))
            .ok_or_else(|| Error::InvalidSubscriptionId { client_id, id })?;

        let subscription = self
            .subscriptions
            .remove(index)
            .expect("subscription index exists, but subscription doesn't");

        for (icao_address, filter_index) in subscription.by_icao_address {
            self.by_icao_address
                .get_mut(&icao_address)
                .expect("invalid backref")
                .remove(filter_index);
        }
        for (callsign, filter_index) in subscription.by_callsign {
            self.by_callsign
                .get_mut(&callsign)
                .expect("invalid backref")
                .remove(filter_index);
        }
        for (squawk, filter_index) in subscription.by_squawk {
            self.by_squawk
                .get_mut(&squawk)
                .expect("invalid backref")
                .remove(filter_index);
        }

        todo!();
    }
}

#[derive(Debug)]
struct Subscription {
    client_id: usize,
    id: Uuid,
    message_sender: mpsc::Sender<SubscriptionMessage>,
    by_icao_address: Vec<(IcaoAddress, usize)>,
    by_callsign: Vec<(String, usize)>,
    by_squawk: Vec<(Squawk, usize)>,
    // todo: secondary filter
}

#[derive(Debug)]
pub struct SubscriptionMessage {
    pub id: Uuid,
    pub event: SubscriptionEvent,
    pub dropped_count: usize,
}
