use std::collections::HashMap;

use adsbee_api_types::{
    IcaoAddress,
    Squawk,
    live::{
        ServerToClientMessage,
        SubscriptionEvent,
        SubscriptionFilter,
    },
};
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::{
    api::live::ClientId,
    tracker::Error,
    util::sparse_list::SparseList,
};

#[derive(Debug, Default)]
pub struct Subscriptions {
    subscriptions: SparseList<Subscription>,

    // key is (client id, client-chosen subscription id)
    by_subscriber_id: HashMap<(ClientId, Uuid), usize>,

    by_icao_address: HashMap<IcaoAddress, SparseList<usize>>,
    by_callsign: HashMap<String, SparseList<usize>>,
    by_squawk: HashMap<Squawk, SparseList<usize>>,
    // todo: by location -> r*tree
}

impl Subscriptions {
    pub fn subscribe(
        &mut self,
        client_id: ClientId,
        id: Uuid,
        filter: SubscriptionFilter,
        message_sender: mpsc::Sender<ServerToClientMessage>,
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

    pub fn unsubscribe(&mut self, client_id: ClientId, id: Uuid) -> Result<(), Error> {
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
    client_id: ClientId,
    id: Uuid,
    message_sender: mpsc::Sender<ServerToClientMessage>,
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
