use adsbee_api_types::live::{
    ClientToServerMessage,
    ServerToClientMessage,
    SubscriptionFilter,
};
use futures_util::{
    SinkExt,
    TryStreamExt,
};
use reqwest_websocket::RequestBuilderExt;
use serde::{
    Serialize,
    de::DeserializeOwned,
};
use url::Url;
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
#[error("adsb-index-api-client error")]
pub enum Error {
    Http(#[from] reqwest::Error),
    Websocket(#[from] reqwest_websocket::Error),
}

#[derive(Clone, Debug)]
pub struct ApiClient {
    client: reqwest::Client,
    api_url: Url,
}

impl ApiClient {
    pub fn new(client: reqwest::Client, mut api_url: Url) -> Self {
        if let Ok(mut path_segments) = api_url.path_segments_mut() {
            path_segments.pop_if_empty();
        }

        Self { client, api_url }
    }

    pub fn from_url(api_url: Url) -> Self {
        Self::new(Default::default(), api_url)
    }

    pub async fn live(&self) -> Result<Live, Error> {
        let websocket = self
            .client
            .get(self.api_url.join("live").unwrap())
            .upgrade()
            .send()
            .await?
            .into_websocket()
            .await?;
        Ok(Live {
            websocket: websocket.into(),
        })
    }
}

#[derive(Debug)]
pub struct Live {
    websocket: JsonWebSocket,
}

impl Live {
    pub async fn next(&mut self) -> Result<Option<ServerToClientMessage>, Error> {
        self.websocket.receive().await
    }

    pub async fn subscribe(
        &mut self,
        id: Uuid,
        filter: SubscriptionFilter,
        start_keyframe: bool,
    ) -> Result<(), Error> {
        self.websocket
            .send(ClientToServerMessage::Subscribe {
                id,
                filter,
                start_keyframe,
            })
            .await?;
        Ok(())
    }

    pub async fn unsubscribe(&mut self, id: Uuid) -> Result<(), Error> {
        self.websocket
            .send(ClientToServerMessage::Unsubscribe { id })
            .await?;
        Ok(())
    }
}

#[derive(Debug)]
struct JsonWebSocket {
    inner: reqwest_websocket::WebSocket,
}

impl From<reqwest_websocket::WebSocket> for JsonWebSocket {
    fn from(value: reqwest_websocket::WebSocket) -> Self {
        Self { inner: value }
    }
}

impl JsonWebSocket {
    pub async fn send<T: Serialize>(&mut self, value: T) -> Result<(), Error> {
        self.inner
            .send(reqwest_websocket::Message::text_from_json(&value)?)
            .await?;
        Ok(())
    }

    pub async fn receive<T: DeserializeOwned>(&mut self) -> Result<Option<T>, Error> {
        loop {
            if let Some(message) = self.inner.try_next().await? {
                if let Some(message) = message.json()? {
                    return Ok(Some(message));
                }
            }
        }
    }
}
