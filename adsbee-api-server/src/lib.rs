#![allow(dead_code)]

pub mod api;
pub mod country;
pub mod database;
pub mod source;
pub mod spatial;
pub mod tracker;
pub(crate) mod util;

// aircraft info:
// https://raw.githubusercontent.com/wiedehopf/tar1090-db/csv/aircraft.csv.gz
// FAA: https://www.faa.gov/licenses_certificates/aircraft_certification/aircraft_registry/releasable_aircraft_download
// military ICAO ranges: https://github.com/wiedehopf/readsb/blob/75decb53c0e66f4c12cf24127578a3fe7d919219/aircraft.c#L907
//
// photo api: https://api.planespotters.net/pub/photos//hex/740735?reg=JY-AYU&icaoType=A320
//
// traces:
// https://adsb.lol/data/traces/dd/trace_full_0101dd.json
// https://adsb.lol/data/traces/dd/trace_recent_0101dd.json
// https://adsb.lol/globe_history/2025/06/24/traces/dd/trace_full_0101dd.json
//
// advisories: https://github.com/wiedehopf/tar1090-aux
//
// ads-b protocol: http://www.anteni.net/adsb/Doc/1090-WP30-18-DRAFT_DO-260B-V42.pdf
//
// lots of data
// https://github.com/vradarserver/standing-data/
//
// routes:
// https://github.com/adsblol/api/blob/main/src/adsb_api/utils/api_routes.py
// https://api.adsb.lol/api/0/routeset
//
// good doc on mode-s: https://mode-s.org/1090mhz/misc/preface.html

#[derive(Debug, thiserror::Error)]
#[error("adsb-index error")]
pub enum Error {
    Io(#[from] std::io::Error),
    Json(#[from] crate::util::json::PrettyJsonError),
    Database(#[from] crate::database::Error),
    Http(#[from] reqwest::Error),
    Csv(#[from] csv::Error),
    IcaoAddress(#[from] adsbee_types::IcaoAddressFromStrError),
    Squawk(#[from] adsbee_types::SquawkFromStrError),
    Wtc(#[from] adsbee_api_types::WtcFromStrError),
    Tar1090AircraftFlags(#[from] crate::source::tar1090_db::AircraftFlagsFromStrError),
    #[error("tar1090-db has no commits")]
    Tar1090NoCommits,
    Beast(#[from] adsbee_beast::Error),
    Sbs(#[from] adsbee_sbs::Error),
}

impl From<sqlx::Error> for Error {
    fn from(value: sqlx::Error) -> Self {
        crate::database::Error::from(value).into()
    }
}
