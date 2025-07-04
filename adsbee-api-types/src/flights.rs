use adsbee_types::{
    IcaoAddress,
    Squawk,
};
use chrono::{
    DateTime,
    Utc,
};
use serde::{
    Deserialize,
    Serialize,
};

use crate::Bbox;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GetSearchQuery {
    // todo
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PostSearchQuery {
    #[serde(default)]
    pub aircraft: AircraftQuery,

    #[serde(default)]
    pub time: TimeQuery,

    #[serde(default)]
    pub area: Vec<Bbox>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AircraftQuery {
    #[serde(default)]
    pub icao: Vec<IcaoAddress>,

    #[serde(default)]
    pub callsign: Vec<String>,

    #[serde(default)]
    pub squawk: Vec<Squawk>,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct TimeQuery {
    // note: i tested it and this converts from a string with any timezone into utc
    pub start: Option<DateTime<Utc>>,
    pub end: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
pub struct SearchResults {
    pub results: Vec<SearchResult>,
}

#[derive(Debug, Serialize)]
pub struct SearchResult {
    pub time: DateTime<Utc>,
    pub icao: IcaoAddress,
    pub callsign: Option<String>,
    pub squawk: Option<Squawk>,
}
