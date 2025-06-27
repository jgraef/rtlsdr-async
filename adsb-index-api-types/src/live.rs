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
        subscription_id: Uuid,
        #[serde(flatten)]
        filter: SubscriptionFilter, // todo: filters
    },
    Unsubcribe {
        subscription_id: Uuid,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SubscriptionFilter {
    aircraft: AircraftQuery,

    #[serde(default)]
    area: Vec<Bbox>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ServerToClientMessage {
    Subscription {
        subscription_id: Uuid,

        #[serde(flatten)]
        event: SubscriptionEvent,

        #[serde(skip_serializing_if = "is_zero")]
        dropped_count: usize,
    },
    Error {
        subscription_id: Option<Uuid>,
        // todo
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
