use adsb_index_api_types::live::{
    ClientToServerMessage,
    ServerToClientMessage,
    SubscriptionEvent,
};
use axum::{
    extract::{
        State,
        WebSocketUpgrade,
        ws::{
            self,
        },
    },
    response::IntoResponse,
};
use serde::{
    Deserialize,
    Serialize,
    de::DeserializeOwned,
};
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::api::Api;

pub async fn get_live(State(api): State<Api>, upgrade: WebSocketUpgrade) -> impl IntoResponse {
    upgrade.on_upgrade(async move |websocket| {
        WebSocketHandler::new(api, websocket).run().await;
    })
}

#[derive(Debug)]
struct WebSocketHandler {
    api: Api,
    websocket: WebSocket,
    subscription_sender: mpsc::Sender<SubscriptionMessage>,
    subscription_receiver: mpsc::Receiver<SubscriptionMessage>,
}

impl WebSocketHandler {
    fn new(api: Api, websocket: ws::WebSocket) -> Self {
        let (subscription_sender, subscription_receiver) = mpsc::channel(128);

        Self {
            api,
            websocket: websocket.into(),
            subscription_sender,
            subscription_receiver,
        }
    }

    async fn run(mut self) {
        loop {
            tokio::select! {
                _ = self.api.shutdown.cancelled() => {
                    let _ = self.websocket.send_close(Some(CloseReason { code: CloseCode::GOING_AWAY, reason: Default::default() })).await;
                    break;
                }
                message = self.websocket.receive::<ClientToServerMessage>() => {
                    match message {
                        Err(error) => {
                            tracing::error!(?error, "websocket receive error");
                            let _ = self.websocket.send_close(error.as_close_reason()).await;
                            break;
                        }
                        Ok(None) => break,
                        Ok(Some(message)) => {
                            if let Err(error) = self.handle_websocket_message(message).await {
                                let _ = self.websocket.send_close(error.as_close_reason()).await;
                                break;
                            }
                        }
                    }
                }
                message = self.subscription_receiver.recv() => {
                    // the channel should never close, since we hold a sender to it.
                    let message = message.expect("subscription event channel closed unexpectedly");
                    if let Err(error) = self.handle_subscription_message(message).await {
                        let _ = self.websocket.send_close(error.as_close_reason()).await;
                        break;
                    }
                }
            }
        }
    }

    async fn handle_websocket_message(
        &mut self,
        message: ClientToServerMessage,
    ) -> Result<(), Error> {
        match message {
            ClientToServerMessage::Subscribe {
                subscription_id,
                filter,
            } => todo!(),
            ClientToServerMessage::Unsubcribe { subscription_id } => todo!(),
        }

        Ok(())
    }

    async fn handle_subscription_message(
        &mut self,
        message: SubscriptionMessage,
    ) -> Result<(), Error> {
        self.websocket
            .send(&ServerToClientMessage::Subscription {
                subscription_id: message.subscription_id,
                event: message.event,
                dropped_count: message.dropped_count,
            })
            .await?;

        Ok(())
    }
}

#[derive(Debug)]
pub struct SubscriptionMessage {
    subscription_id: Uuid,
    event: SubscriptionEvent,
    dropped_count: usize,
}

#[derive(Debug, thiserror::Error)]
#[error("websocket error")]
pub enum Error {
    Axum(#[from] axum::Error),
    Json(#[from] serde_json::Error),
}

impl Error {
    fn as_close_reason(&self) -> Option<CloseReason> {
        match self {
            Self::Json(error) => {
                Some(CloseReason {
                    code: CloseCode::PROTOCOL_ERROR,
                    reason: error.to_string(),
                })
            }
            _ => None,
        }
    }
}

#[derive(Debug)]
struct CloseReason {
    code: CloseCode,
    reason: String,
}

impl From<CloseReason> for ws::CloseFrame {
    fn from(value: CloseReason) -> Self {
        ws::CloseFrame {
            code: value.code.0,
            reason: value.reason.into(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct CloseCode(pub u16);

impl CloseCode {
    const INTERNAL_ERROR: Self = Self(1011);
    const GOING_AWAY: Self = Self(1001);
    const PROTOCOL_ERROR: Self = Self(1002);
}

#[derive(Debug)]
struct WebSocket {
    inner: ws::WebSocket,
}

impl From<ws::WebSocket> for WebSocket {
    fn from(value: ws::WebSocket) -> Self {
        Self { inner: value }
    }
}

impl WebSocket {
    async fn send<T: Serialize>(&mut self, message: T) -> Result<(), Error> {
        self.inner
            .send(ws::Message::Text(serde_json::to_string(&message)?.into()))
            .await?;
        Ok(())
    }

    async fn send_close(&mut self, reason: Option<CloseReason>) -> Result<(), Error> {
        self.inner
            .send(ws::Message::Close(reason.map(Into::into)))
            .await?;
        Ok(())
    }

    async fn receive<T: DeserializeOwned>(&mut self) -> Result<Option<T>, Error> {
        loop {
            match self.inner.recv().await {
                None => return Ok(None),
                Some(Err(error)) => return Err(error.into()),
                Some(Ok(ws::Message::Text(text))) => {
                    return Ok(Some(serde_json::from_str(&text)?));
                }
                Some(Ok(ws::Message::Binary(data))) => {
                    return Ok(Some(serde_json::from_slice(&data)?));
                }
                Some(Ok(ws::Message::Close(_))) => {
                    return Ok(None);
                }
                _ => {
                    // any other messages types are ignore and we wait for
                    // another frame
                }
            }
        }
    }
}
