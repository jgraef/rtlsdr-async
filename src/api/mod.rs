pub mod live;
pub mod search;

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

use crate::database::Database;

#[derive(Clone, Debug)]
pub struct Api {
    pub database: Database,
}

impl Api {
    pub fn router(&self) -> Router<()> {
        Router::new()
            .route("search", routing::get(search::get_search))
            .route("search", routing::post(search::post_search))
            .route("live", routing::get(live::get_live))
            .fallback(routing::get(not_found))
            .with_state(self.clone())
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
