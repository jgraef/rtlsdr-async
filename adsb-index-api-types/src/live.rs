use serde::{
    Deserialize,
    Serialize,
};
use uuid::Uuid;

use crate::{
    Bbox,
    flights::AircraftQuery,
};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ClientToServerMessage {
    Subscribe {
        id: Uuid,
        #[serde(flatten)]
        filter: SubscriptionFilter, // todo: filters
    },
    Unsubscribe {
        id: Uuid,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SubscriptionFilter {
    pub aircraft: AircraftQuery,

    #[serde(default)]
    pub area: Vec<Bbox>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ServerToClientMessage {
    Subscription {
        id: Uuid,

        #[serde(flatten)]
        event: SubscriptionEvent,

        #[serde(skip_serializing_if = "is_zero")]
        dropped_count: usize,
    },
    Subscribed { id: Uuid },
    Unsubscribed { id: Uuid },
    Error {
        id: Option<Uuid>,
        message: Option<String>,
    },
}

fn is_zero(x: &usize) -> bool {
    *x == 0
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SubscriptionEvent {
    // todo
}
