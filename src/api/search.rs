use axum::{
    Json,
    extract::{
        Query,
        State,
    },
};
use serde::{
    Deserialize,
    Serialize,
};

use crate::api::{
    Api,
    ErrorResponse,
};

#[derive(Debug, Serialize)]
pub struct SearchResults {
    // todo
}

#[derive(Debug, Deserialize)]
pub struct GetQuery {
    // todo
}

#[derive(Debug, Deserialize)]
pub struct PostQuery {
    // todo
}

pub async fn get_search(
    State(api): State<Api>,
    Query(query): Query<GetQuery>,
) -> Result<Json<SearchResults>, ErrorResponse> {
    todo!();
}

pub async fn post_search(
    State(api): State<Api>,
    Json(query): Json<PostQuery>,
) -> Result<Json<SearchResults>, ErrorResponse> {
    todo!();
}
