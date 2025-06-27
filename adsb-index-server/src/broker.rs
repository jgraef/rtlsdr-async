use std::collections::HashMap;

use adsb_index_api_types::{
    IcaoAddress,
    Squawk,
    live::{
        SubscriptionEvent,
        SubscriptionFilter,
    },
};
use tokio::sync::{
    mpsc,
    oneshot,
};
use uuid::Uuid;

const COMMAND_QUEUE_SIZE: usize = 32;

#[derive(Debug, thiserror::Error)]
#[error("broker error")]
pub enum Error {
    #[error("reactor dead")]
    ReactorDead,
}

#[derive(Clone, Debug)]
pub struct Broker {
    command_sender: mpsc::Sender<Command>,
}

impl Broker {
    pub fn new() -> Self {
        let (command_sender, command_receiver) = mpsc::channel(COMMAND_QUEUE_SIZE);

        tokio::spawn(async move {
            let reactor = Reactor::new(command_receiver);
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

    pub async fn subscribe(&self, client_id: usize, id: Uuid, filter: SubscriptionFilter, message_sender: mpsc::Sender<SubscriptionMessage>) -> Result<(), Error> {
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

    pub async fn unsubscribe(&self, id: Uuid) -> Result<(), Error> {
        let (result_sender, result_receiver) = oneshot::channel();

        self.send_command(Command::Unsubscribe { id, result_sender })
            .await;

        result_receiver.await.expect("broker result channel closed")
    }
}

#[derive(Debug)]
struct Reactor {
    command_receiver: mpsc::Receiver<Command>,
    subscriptions: Subscriptions,
    // todo: receive updates from beast stream, but these need to be processed first
}

impl Reactor {
    fn new(command_receiver: mpsc::Receiver<Command>) -> Self {
        Self {
            command_receiver,
            subscriptions: Default::default(),
        }
    }

    async fn run(mut self) -> Result<(), Error> {
        loop {
            tokio::select! {
                command = self.command_receiver.recv() => {
                    if let Some(command) = command {
                        self.handle_command(command).await?;
                    }
                    else {
                        break;
                    }
                }
            }
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
                let result = self.subscriptions.subscribe(client_id, id, filter, event_sender);
                let _ = result_sender.send(result);
            }
            Command::Unsubscribe { id, result_sender } => {
                let result = self.subscriptions.unsubscribe(id);
                let _ = result_sender.send(result);
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
        id: Uuid,
        result_sender: oneshot::Sender<Result<(), Error>>,
    },
}

#[derive(Debug, Default)]
struct Subscriptions {
    subscriptions: Vec<Option<Subscription>>,
    free_list: Vec<usize>,

    // key is (client id, client-chosen subscription id)
    by_subscriber_id: HashMap<(usize, Uuid), usize>,

    by_icao_address: HashMap<IcaoAddress, Vec<usize>>,
    by_callsign: HashMap<String, Vec<usize>>,
    by_squawk: HashMap<Squawk, Vec<usize>>,

    // todo: by location -> r*tree
}

impl Subscriptions {
    pub fn subscribe(&mut self, client_id: usize, id: Uuid, filter: SubscriptionFilter, message_sender: mpsc::Sender<SubscriptionMessage>) -> Result<(), Error> {
        let subscription = Subscription { message_sender };

        let index = if let Some(index) = self.free_list.pop() {
            assert!(self.subscriptions[index].is_none());
            self.subscriptions[index] = Some(subscription);
            index
        }
        else {
            let index = self.subscriptions.len();
            self.subscriptions.push(Some(subscription));
            index
        };

        self.by_subscriber_id.insert((client_id, id), index);

        if filter.area.is_empty() {
            for icao_address in &filter.aircraft.icao {
                self.by_icao_address.entry(*icao_address).or_default().push(index);
            }
            for callsign in &filter.aircraft.callsign {
                self.by_callsign.entry(callsign.clone()).or_default().push(index);
            }
            for squawk in &filter.aircraft.squawk {
                self.by_squawk.entry(*squawk).or_default().push(index);
            }
        }
        else {
            todo!();
        }
        
        Ok(())
    }

    pub fn unsubscribe(&mut self, id: Uuid) -> Result<(), Error> {
        todo!();
    }
}

#[derive(Debug)]
struct Subscription {
    message_sender: mpsc::Sender<SubscriptionMessage>,
    // todo: we need to store information with which we can remove references from the `by_*` maps.
    // todo: secondary filter
}

#[derive(Debug)]
pub struct SubscriptionMessage {
    pub id: Uuid,
    pub event: SubscriptionEvent,
    pub dropped_count: usize,
}
