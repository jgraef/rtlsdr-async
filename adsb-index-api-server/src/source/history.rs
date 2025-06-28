// https://raw.githubusercontent.com/adsblol/globe_history_2025/refs/heads/main/PREFERRED_RELEASES.txt

use std::{
    fs::File,
    io::{
        BufReader,
        Read,
    },
    path::Path,
};

use adsb_index_api_types::{
    IcaoAddress,
    Squawk,
};
use chrono::{
    DateTime,
    Utc,
};
use libflate::gzip;
use serde::{
    Deserialize,
    de::Visitor,
};

use crate::{
    Error,
    database::Database,
    util::json::json_decode,
};

pub async fn index_archive_day_from_directory(
    database: &Database,
    path: impl AsRef<Path>,
) -> Result<(), Error> {
    let path = path.as_ref();

    for result in std::fs::read_dir(path.join("traces"))? {
        let dir_entry = result?;

        for result in std::fs::read_dir(dir_entry.path())? {
            let dir_entry = result?;

            let trace_path = dir_entry.path();
            tracing::debug!(path = %trace_path.display(), "reading trace");

            let reader = BufReader::new(File::open(&trace_path)?);
            let mut reader = gzip::Decoder::new(reader)?;
            let mut json = String::new();
            reader.read_to_string(&mut json)?;
            let trace: TraceFile = json_decode(&json)?;

            index_trace(&database, &trace).await?;
        }
    }

    Ok(())
}

pub async fn index_trace(database: &Database, trace: &TraceFile) -> Result<(), Error> {
    let icao_address = trace.icao.parse::<IcaoAddress>()?;

    let mut current_callsign = None;
    let mut current_squawk = None;

    let mut transaction = database.transaction().await?;

    // todo: check if we already have data for that date/icao

    let n = trace.trace.len();
    for (i, tp) in trace.trace.iter().enumerate() {
        let is_first = i == 0;
        let is_last = i == n - 1;

        let time = datetime_from_timestamp(trace.timestamp + tp.dt);
        let mut callsign_changed = false;
        let mut squawk_changed = false;

        if let Some(aircraft) = &tp.aircraft {
            if let Some(callsign) = &aircraft.flight {
                let callsign = callsign.trim();

                callsign_changed = current_callsign.as_ref().map_or(true, |c| c != callsign);
                if callsign_changed {
                    current_callsign = Some(callsign.to_owned());
                }
            }

            if let Some(squawk) = &aircraft.squawk {
                let squawk = squawk.parse::<Squawk>()?;
                squawk_changed = current_squawk.map_or(true, |p| p != squawk);
                current_squawk = Some(squawk);
            }
        }

        if is_first || is_last || callsign_changed || squawk_changed {
            tracing::debug!(icao = %icao_address, %time, callsign = ?current_callsign, squawk = ?current_squawk);

            sqlx::query_unchecked!(
                "insert into trace_info (time, icao_address, callsign, squawk) values ($1, $2, $3, $4)",
                time,
                icao_address,
                current_callsign,
                current_squawk,
            ).execute(&mut *transaction).await?;
        }
    }

    transaction.commit().await?;

    Ok(())
}

fn datetime_from_timestamp(timestamp: f32) -> DateTime<Utc> {
    DateTime::from_timestamp_millis((timestamp * 1000.0) as i64).expect("invalid timestamp")
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TraceFile {
    pub icao: String,
    pub r: Option<String>,
    pub t: Option<String>,
    #[serde(default)]
    pub db_flags: u32,
    pub desc: Option<String>,
    pub own_op: Option<String>,
    pub year: Option<String>,
    pub version: String,
    pub timestamp: f32,
    pub trace: Vec<TracePoint>,
}

#[derive(Debug, Deserialize)]
#[serde(try_from = "TraceTupleV2")]
pub struct TracePoint {
    pub dt: f32,
    pub lat: f32,
    pub long: f32,
    pub altitude: Option<Altitude>,
    pub ground_speed: Option<f32>,
    pub track: Option<f32>,
    pub db_flags: u32,
    pub vertical_rate: Option<i32>,
    pub aircraft: Option<Aircraft>,
    pub source: Option<String>,
    pub geo_alt: Option<i32>,
    pub geo_vr: Option<i32>,
    pub ind_airspeed: Option<u32>,
    pub roll_angle: Option<f32>,
}

type TraceTupleV1 = (
    f32,
    f32,
    f32,
    Option<Altitude>,
    Option<f32>,
    Option<f32>,
    u32,
    Option<i32>,
    Option<Aircraft>,
);

type TraceTupleV2 = (
    f32,
    f32,
    f32,
    Option<Altitude>,
    Option<f32>,
    Option<f32>,
    u32,
    Option<i32>,
    Option<Aircraft>,
    Option<String>,
    Option<i32>,
    Option<i32>,
    Option<u32>,
    Option<f32>,
);

#[derive(Deserialize)]
#[serde(untagged)]
enum TraceTuple {
    V1(TraceTupleV1),
    V2(TraceTupleV2),
}

impl From<TraceTupleV1> for TracePoint {
    fn from(value: TraceTupleV1) -> Self {
        TracePoint {
            dt: value.0,
            lat: value.1,
            long: value.2,
            altitude: value.3,
            ground_speed: value.4,
            track: value.5,
            db_flags: value.6,
            vertical_rate: value.7,
            aircraft: value.8,
            source: None,
            geo_alt: None,
            geo_vr: None,
            ind_airspeed: None,
            roll_angle: None,
        }
    }
}

impl From<TraceTupleV2> for TracePoint {
    fn from(value: TraceTupleV2) -> Self {
        TracePoint {
            dt: value.0,
            lat: value.1,
            long: value.2,
            altitude: value.3,
            ground_speed: value.4,
            track: value.5,
            db_flags: value.6,
            vertical_rate: value.7,
            aircraft: value.8,
            source: value.9,
            geo_alt: value.10,
            geo_vr: value.11,
            ind_airspeed: value.12,
            roll_angle: value.13,
        }
    }
}

impl From<TraceTuple> for TracePoint {
    fn from(value: TraceTuple) -> Self {
        match value {
            TraceTuple::V1(t) => t.into(),
            TraceTuple::V2(t) => t.into(),
        }
    }
}

#[derive(Debug)]
pub enum Altitude {
    Ground,
    Value(f32),
}

impl<'de> Deserialize<'de> for Altitude {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(AltitudeVisitor)
    }
}

struct AltitudeVisitor;

impl<'de> Visitor<'de> for AltitudeVisitor {
    type Value = Altitude;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("an float, or \"ground\"")
    }

    fn visit_i8<E>(self, v: i8) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(Altitude::Value(v as f32))
    }

    fn visit_u8<E>(self, v: u8) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(Altitude::Value(v as f32))
    }

    fn visit_i16<E>(self, v: i16) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(Altitude::Value(v as f32))
    }

    fn visit_u16<E>(self, v: u16) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(Altitude::Value(v as f32))
    }

    fn visit_i32<E>(self, v: i32) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(Altitude::Value(v as f32))
    }

    fn visit_u32<E>(self, v: u32) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(Altitude::Value(v as f32))
    }

    fn visit_i64<E>(self, v: i64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(Altitude::Value(v as f32))
    }

    fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(Altitude::Value(v as f32))
    }

    fn visit_f32<E>(self, v: f32) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(Altitude::Value(v))
    }

    fn visit_f64<E>(self, v: f64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(Altitude::Value(v as f32))
    }

    fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        if v == "ground" {
            Ok(Altitude::Ground)
        }
        else {
            Err(E::custom(format!(
                "Only \"ground\" is a valid string altitude value."
            )))
        }
    }

    fn visit_string<E>(self, v: String) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        self.visit_str(&v)
    }
}

#[derive(Debug, Deserialize)]
pub struct Aircraft {
    pub squawk: Option<String>,
    #[serde(default)]
    pub nav_modes: Vec<String>,
    pub emergency: Option<Emergency>,
    pub flight: Option<String>,
    // todo
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Emergency {
    None,
    General,
    Lifeguard,
    MinFuel,
    #[serde(rename = "nordo")]
    NoRadio,
    Unlawful,
    Downed,
    Reserved,
}
