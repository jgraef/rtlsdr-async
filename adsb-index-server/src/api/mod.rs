pub mod flights;
pub mod live;

use std::sync::{atomic::{AtomicUsize, Ordering}, Arc, Mutex, RwLock};

use axum::{
    Json,
    Router,
    response::{
        IntoResponse,
        Response,
    },
    routing,
};
use reqwest::StatusCode;
use serde::Serialize;
use tokio::net::{
    TcpListener,
    ToSocketAddrs,
};
use tokio_util::sync::CancellationToken;

use crate::{broker::Broker, database::Database};

#[derive(Clone, Debug)]
pub struct Api {
    pub database: Database,
    pub broker: Broker,
    pub shutdown: CancellationToken,
    next_client_id: Arc<AtomicUsize>,
}

impl Api {
    pub fn new(database: Database, broker: Broker) -> Self {
        Self {
            database,
            broker,
            shutdown: CancellationToken::new(),
            next_client_id: Arc::new(AtomicUsize::new(1)),
        }
    }

    pub fn router(&self) -> Router<()> {
        Router::new()
            .nest(
                "v1",
                Router::new()
                    .route("flights", routing::get(flights::get_search_flights))
                    .route("flights", routing::post(flights::post_search_flights))
                    .route("live", routing::get(live::get_live)),
            )
            .fallback(routing::get(not_found))
            .with_state(self.clone())
    }

    pub async fn serve(&self, listen_addresses: impl ToSocketAddrs) -> Result<(), crate::Error> {
        let tcp_listener = TcpListener::bind(listen_addresses).await?;
        let shutdown = self.shutdown.clone();

        axum::serve(tcp_listener, self.router().into_make_service())
            .with_graceful_shutdown(async move {
                shutdown.cancelled().await;
            })
            .await?;

        Ok(())
    }

    pub fn next_client_id(&self) -> usize {
        self.next_client_id.fetch_add(1, Ordering::Relaxed)
    }
}

async fn not_found() -> impl IntoResponse {
    (StatusCode::NOT_FOUND, "not found")
}

#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    error: ErrorResponseInner,
}

#[derive(Debug, Serialize)]
pub struct ErrorResponseInner {
    message: String,
    error: ApiError,
}

impl From<ApiError> for ErrorResponse {
    fn from(value: ApiError) -> Self {
        Self {
            error: ErrorResponseInner {
                message: value.to_string(),
                error: value,
            },
        }
    }
}

impl IntoResponse for ErrorResponse {
    fn into_response(self) -> Response {
        (self.error.error.status_code(), Json(self)).into_response()
    }
}

#[derive(Debug, thiserror::Error, Serialize)]
#[error("api error")]
#[serde(rename_all = "snake_case")]
pub enum ApiError {
    InternalServerError,
}

impl ApiError {
    pub fn status_code(&self) -> StatusCode {
        match self {
            Self::InternalServerError => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl From<crate::database::Error> for ApiError {
    fn from(value: crate::database::Error) -> Self {
        Self::InternalServerError
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(value: sqlx::Error) -> Self {
        crate::database::Error::from(value).into()
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        ErrorResponse::from(self).into_response()
    }
}
