use adsb_index_api_types::live::{
    ClientToServerMessage,
    ServerToClientMessage,
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
    Serialize,
    de::DeserializeOwned,
};
use tokio::sync::mpsc;

use crate::{
    api::Api,
    tracker::SubscriptionMessage,
};

pub async fn get_live(State(api): State<Api>, upgrade: WebSocketUpgrade) -> impl IntoResponse {
    upgrade.on_upgrade(async move |websocket| {
        WebSocketHandler::new(api, websocket).run().await;
    })
}

#[derive(Debug)]
struct WebSocketHandler {
    client_id: usize,
    api: Api,
    websocket: WebSocket,
    subscription_sender: mpsc::Sender<SubscriptionMessage>,
    subscription_receiver: mpsc::Receiver<SubscriptionMessage>,
}

impl WebSocketHandler {
    fn new(api: Api, websocket: ws::WebSocket) -> Self {
        let (subscription_sender, subscription_receiver) = mpsc::channel(128);

        Self {
            client_id: api.next_client_id(),
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
                    let _ = self.websocket.send_close(CloseReason { code: CloseCode::GOING_AWAY, reason: Default::default() }).await;
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
                id,
                filter,
                start_keyframe,
            } => {
                match self
                    .api
                    .tracker
                    .subscribe(
                        self.client_id,
                        id,
                        filter,
                        start_keyframe,
                        self.subscription_sender.clone(),
                    )
                    .await
                {
                    Ok(()) => {
                        self.websocket
                            .send(ServerToClientMessage::Subscribed { id })
                            .await?;
                    }
                    Err(error) => {
                        self.websocket
                            .send(ServerToClientMessage::Error {
                                id: Some(id),
                                message: Some(error.to_string()),
                            })
                            .await?;
                    }
                }
            }
            ClientToServerMessage::Unsubscribe { id } => {
                match self.api.tracker.unsubscribe(self.client_id, id).await {
                    Ok(()) => {
                        self.websocket
                            .send(ServerToClientMessage::Unsubscribed { id })
                            .await?;
                    }
                    Err(error) => {
                        self.websocket
                            .send(ServerToClientMessage::Error {
                                id: Some(id),
                                message: Some(error.to_string()),
                            })
                            .await?;
                    }
                }
            }
        }

        Ok(())
    }

    async fn handle_subscription_message(
        &mut self,
        message: SubscriptionMessage,
    ) -> Result<(), Error> {
        self.websocket
            .send(&ServerToClientMessage::Subscription {
                id: message.id,
                event: message.event,
                dropped_count: message.dropped_count,
            })
            .await?;

        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
#[error("websocket error")]
pub enum Error {
    Axum(#[from] axum::Error),
    Json(#[from] serde_json::Error),
}

impl Error {
    fn as_close_reason(&self) -> CloseReason {
        match self {
            Self::Json(error) => {
                CloseReason {
                    code: CloseCode::PROTOCOL_ERROR,
                    reason: error.to_string(),
                }
            }
            _ => {
                CloseReason {
                    code: CloseCode::INTERNAL_ERROR,
                    reason: "internal error".to_owned(),
                }
            }
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

    async fn send_close(&mut self, reason: CloseReason) -> Result<(), Error> {
        self.inner
            .send(ws::Message::Close(Some(reason.into())))
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
