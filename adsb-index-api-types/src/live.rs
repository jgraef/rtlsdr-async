use serde::{
    Deserialize,
    Serialize,
};
use uuid::Uuid;

use crate::{
    Bbox,
    flights::AircraftQuery,
    util::{
        is_false,
        is_zero,
    },
};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ClientToServerMessage {
    Subscribe {
        id: Uuid,
        #[serde(flatten)]
        filter: SubscriptionFilter,
        #[serde(skip_serializing_if = "is_false")]
        start_keyframe: bool,
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

        #[serde(default, skip_serializing_if = "is_zero")]
        dropped_count: usize,
    },
    Subscribed {
        id: Uuid,
    },
    Unsubscribed {
        id: Uuid,
    },
    Error {
        id: Option<Uuid>,
        message: Option<String>,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SubscriptionEvent {
    // todo
}
