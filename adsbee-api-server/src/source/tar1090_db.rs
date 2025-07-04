use std::{
    collections::HashMap,
    fs::File,
    io::{
        BufReader,
        Read,
    },
    path::Path,
    str::FromStr,
    time::Instant,
};

use adsbee_api_types::Wtc;
use adsbee_types::IcaoAddress;
use bitflags::bitflags;
use bytes::Buf;
use humantime::format_duration;
use libflate::gzip;
use serde::Deserialize;

use crate::{
    Error,
    database::{
        Database,
        Transaction,
    },
    util::http_client,
};

const AIRCRAFT_DB_URL: &'static str =
    "https://raw.githubusercontent.com/wiedehopf/tar1090-db/csv/aircraft.csv.gz";
const AIRCRAFT_TYPES_JSON_URL: &'static str =
    "https://github.com/wiedehopf/tar1090-db/raw/refs/heads/master/icao_aircraft_types.json";

const AIRCRAFT_DB_BRANCH_API_URL: &'static str =
    "https://api.github.com/repos/wiedehopf/tar1090-db/branches/csv";

const AIRCRAFT_DB_COMMIT_METADATA_KEY: &'static str = "tar1090-db_commit";

async fn get_aircraft_db_commit(
    transaction: &mut Transaction<'_>,
) -> Result<Option<String>, Error> {
    Ok(transaction
        .get_metadata(AIRCRAFT_DB_COMMIT_METADATA_KEY)
        .await?)
}

async fn set_aircraft_db_commit(
    transaction: &mut Transaction<'_>,
    commit: &str,
) -> Result<(), Error> {
    transaction
        .set_metadata(AIRCRAFT_DB_COMMIT_METADATA_KEY, &commit)
        .await?;
    Ok(())
}

async fn get_latest_aircraft_db_commit() -> Result<String, Error> {
    #[derive(Debug, Deserialize)]
    struct Branch {
        commit: Option<Commit>,
    }
    #[derive(Debug, Deserialize)]
    struct Commit {
        sha: String,
    }

    let branch: Branch = http_client()
        .get(AIRCRAFT_DB_BRANCH_API_URL)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    Ok(branch.commit.ok_or_else(|| Error::Tar1090NoCommits)?.sha)
}

pub async fn update_aircraft_db(database: &Database, file: Option<&Path>) -> Result<(), Error> {
    let mut transaction = database.transaction().await?;

    tracing::info!("checking for aircraft-db updates");

    let latest_commit = get_latest_aircraft_db_commit().await?;
    tracing::info!(%latest_commit);

    let do_update = if let Some(current_commit) = get_aircraft_db_commit(&mut transaction).await? {
        tracing::info!(%current_commit);
        current_commit != latest_commit
    }
    else {
        tracing::info!("aircraft-db has never been updated.");
        true
    };

    if do_update {
        tracing::info!("updating aircraft-db");

        let t_start = Instant::now();

        let aircraft_csv: Box<dyn Read> = if let Some(file) = file {
            tracing::info!(path = %file.display(), "using provided aircraft db");
            let reader = BufReader::new(File::open(&file)?);
            if file.extension().map_or(false, |ext| ext == "gz") {
                Box::new(gzip::Decoder::new(reader)?)
            }
            else {
                Box::new(reader)
            }
        }
        else {
            tracing::info!(url = AIRCRAFT_DB_URL, "downloading aircraft db");
            let aircraft_csv_gz = http_client()
                .get(AIRCRAFT_DB_URL)
                .send()
                .await?
                .error_for_status()?
                .bytes()
                .await?;
            Box::new(gzip::Decoder::new(aircraft_csv_gz.reader())?)
        };

        let aircraft_csv = csv::ReaderBuilder::new()
            .has_headers(false)
            .delimiter(b';')
            .from_reader(aircraft_csv);

        #[derive(Debug, Deserialize)]
        struct AircraftType {
            desc: String,
            wtc: String,
        }

        let icao_aircraft_types_json: HashMap<String, AircraftType> = http_client()
            .get(AIRCRAFT_TYPES_JSON_URL)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;

        // https://github.com/wiedehopf/readsb/blob/75decb53c0e66f4c12cf24127578a3fe7d919219/aircraft.c#L841
        #[derive(Debug, Deserialize)]
        struct Row {
            icao_address: String,
            registration: String,
            icao_model_code: Option<String>,
            flags: String,
            model_name: Option<String>,
            year: Option<u16>,
            operator: Option<String>,
            unknown3: Option<String>,
        }

        for result in aircraft_csv.into_deserialize::<Row>() {
            let row = result?;

            let icao_address: IcaoAddress = row.icao_address.parse()?;
            let flags: AircraftFlags = row.flags.parse()?;

            sqlx::query_unchecked!(
                r#"
            insert into aircraft_registration (
                icao_address,
                registration,
                model
            ) values ($1, $2, $3)
            on conflict (icao_address) do update set
                registration = $2,
                model = $3
            "#,
                icao_address,
                row.registration,
                row.icao_model_code,
            )
            .execute(&mut *transaction)
            .await?;

            if let Some(icao_model_code) = &row.icao_model_code {
                let aircraft_type = icao_aircraft_types_json.get(icao_model_code);
                let model_desc = aircraft_type.map(|ty| &ty.desc);
                let model_wtc = aircraft_type.map(|ty| ty.wtc.parse::<Wtc>()).transpose()?;

                sqlx::query_unchecked!(
                    r#"
                insert into aircraft_model (
                    icao_code,
                    name,
                    description,
                    wtc
                ) values ($1, $2, $3, $4)
                on conflict (icao_code) do update set
                    name = $2,
                    description = $3,
                    wtc = $4
                "#,
                    icao_model_code,
                    row.model_name,
                    model_desc,
                    model_wtc,
                )
                .execute(&mut *transaction)
                .await?;
            }

            const TAGS: &'static [(AircraftFlags, &'static str)] =
                &[(AircraftFlags::MILITARY, "military")];
            for (flag, tag) in TAGS {
                if flags.contains(*flag) {
                    sqlx::query_unchecked!("insert into aircraft_tag (icao_address, tag) values ($1, $2) on conflict (icao_address, tag) do nothing", icao_address, tag)
                    .execute(&mut *transaction).await?;
                }
            }
        }

        set_aircraft_db_commit(&mut transaction, &latest_commit).await?;

        transaction.commit().await?;

        let elapsed = t_start.elapsed();
        tracing::info!("update finished: {}", format_duration(elapsed));
    }

    Ok(())
}

#[derive(Clone, Debug)]
pub struct Aircraft {
    pub icao_address: IcaoAddress,
    pub registration: String,
    pub icao_model_code: Option<String>,
    pub flags: AircraftFlags,
    pub model_name: Option<String>,
    pub registration_year: Option<u16>,
    pub operator: Option<String>,
}

bitflags! {
    /// Bits 1-4 are documented [here][1]. There are other bits that have unknown meaning. readsb uses 16 bits[2]
    ///
    /// [1]: https://github.com/wiedehopf/readsb/blob/75decb53c0e66f4c12cf24127578a3fe7d919219/README-json.md?plain=1#L112
    /// [2]: https://github.com/wiedehopf/readsb/blob/75decb53c0e66f4c12cf24127578a3fe7d919219/track.h#L587
    ///
    #[derive(Clone, Copy, Debug, Default)]
    pub struct AircraftFlags: u16 {
        const MILITARY = 0b0001;
        const INTERESTING = 0b0010;
        const PIA = 0b0100;
        const LADD = 0b1000;
    }
}

impl FromStr for AircraftFlags {
    type Err = AircraftFlagsFromStrError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // the dbFlags are just a binary number string
        // https://github.com/wiedehopf/readsb/blob/75decb53c0e66f4c12cf24127578a3fe7d919219/aircraft.c#L851
        let bits = u16::from_str_radix(s, 2).map_err(|_| {
            AircraftFlagsFromStrError {
                input: s.to_owned(),
            }
        })?;

        // at least one entry has flags of 0b10000. @wiedehopf doesn't know what it
        // means, so we just ignore unknown bits :shrug:
        Ok(AircraftFlags::from_bits_retain(bits))
    }
}

#[derive(Clone, Debug, thiserror::Error)]
#[error("invalid aircraft flags: {input}")]
pub struct AircraftFlagsFromStrError {
    input: String,
}
