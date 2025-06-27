use adsb_index_api_types::{
    IcaoAddress,
    Squawk,
    flights::{
        GetSearchQuery,
        PostSearchQuery,
        SearchResult,
        SearchResults,
    },
};
use axum::{
    Json,
    extract::{
        Query,
        State,
    },
};
use chrono::{
    DateTime,
    Utc,
};
use futures_util::TryStreamExt;

use crate::api::{
    Api,
    ApiError,
};

pub async fn get_search_flights(
    State(api): State<Api>,
    Query(query): Query<GetSearchQuery>,
) -> Result<Json<SearchResults>, ApiError> {
    //Ok(Json(search_impl(api, query.try_into()?).await?))
    todo!();
}

pub async fn post_search_flights(
    State(api): State<Api>,
    Json(query): Json<PostSearchQuery>,
) -> Result<Json<SearchResults>, ApiError> {
    Ok(Json(search_impl(api, query).await?))
}

async fn search_impl(api: Api, query: PostSearchQuery) -> Result<SearchResults, ApiError> {
    if query.area.is_empty() {
        let mut transaction = api.database.transaction().await?;

        #[derive(Debug)]
        struct Row {
            time: DateTime<Utc>,
            icao_address: IcaoAddress,
            callsign: Option<String>,
            squawk: Option<Squawk>,
        }

        let mut stream = sqlx::query_as_unchecked!(
            Row,
            r#"
                select
                    time,
                    icao_address,
                    callsign,
                    squawk
                from trace_info
                where
                    (
                        (time >= $1 or $1 is null)
                        and (time <= $2 or $2 is null)
                    )
                    and (
                        icao_address = any($3) or array_length($3, 1) = 0
                        or callsign = any($4) or array_length($4, 1) = 0
                        or squawk = any($5) or array_length($5, 1) = 0
                    )
            "#,
            query.time.start,
            query.time.end,
            query.aircraft.icao,
            query.aircraft.callsign,
            query.aircraft.squawk
        )
        .fetch(&mut *transaction);

        let mut results = vec![];

        while let Some(row) = stream.try_next().await? {
            results.push(SearchResult {
                time: row.time,
                icao: row.icao_address,
                callsign: row.callsign,
                squawk: row.squawk,
            });
        }

        Ok(SearchResults { results })
    }
    else {
        todo!();
    }
}
