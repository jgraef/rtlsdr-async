use axum::{
    extract::{
        State,
        WebSocketUpgrade,
        ws,
    },
    response::IntoResponse,
};
use serde::{
    Serialize,
    de::DeserializeOwned,
};

use crate::api::Api;

pub async fn get_live(State(api): State<Api>, upgrade: WebSocketUpgrade) -> impl IntoResponse {
    upgrade.on_upgrade(async move |websocket| {
        handle_live_socket(api, websocket.into()).await;
    })
}

async fn handle_live_socket(api: Api, websocket: WebSocket) {
    todo!();
}

#[derive(Debug, thiserror::Error)]
#[error("websocket error")]
pub enum Error {
    Axum(#[from] axum::Error),
    Json(#[from] serde_json::Error),
}

#[derive(Debug)]
pub struct WebSocket {
    inner: ws::WebSocket,
}

impl From<ws::WebSocket> for WebSocket {
    fn from(value: ws::WebSocket) -> Self {
        Self { inner: value }
    }
}

impl WebSocket {
    pub async fn send<T: Serialize>(&mut self, message: &T) -> Result<(), Error> {
        self.inner
            .send(ws::Message::Text(serde_json::to_string(message)?.into()))
            .await?;
        Ok(())
    }

    pub async fn receive<T: DeserializeOwned>(&mut self) -> Result<Option<T>, Error> {
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
