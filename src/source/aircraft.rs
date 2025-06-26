use std::{
    fs::File,
    io::{
        BufReader,
        Cursor,
    },
    str::FromStr,
};

use bitflags::bitflags;
use color_eyre::eyre::eyre;
use libflate::gzip;
use serde::Deserialize;

use crate::{
    Error,
    database::{
        Database,
        Transaction,
    },
    types::IcaoAddress,
};

const AIRCRAFT_DB_URL: &'static str =
    "https://raw.githubusercontent.com/wiedehopf/tar1090-db/csv/aircraft.csv.gz";

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

    let branch: Branch = reqwest::get(AIRCRAFT_DB_BRANCH_API_URL)
        .await?
        .error_for_status()?
        .json()
        .await?;

    Ok(branch
        .commit
        .ok_or_else(|| eyre!("tar1090-db/csv has no commits"))?
        .sha)
}

pub async fn update_aircraft_db(database: &Database) -> Result<(), Error> {
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

        let aircrafts = fetch_aircraft_db().await?;

        for aircraft in &aircrafts {
            tracing::debug!(icao_address = %aircraft.icao_address, "updating aircraft registration");
            sqlx::query_unchecked!(
                r#"
            insert into aircraft_registration (
                icao_address,
                registration,
                model
            ) values ($1, $2, $3)
            on conflict (icao_address) do update set registration = $2, model = $3
            "#,
                aircraft.icao_address,
                aircraft.registration,
                aircraft.icao_model_code,
            )
            .execute(&mut *transaction)
            .await?;

            tracing::debug!(icao_code = %aircraft.icao_model_code, "updating aircraft model");
            sqlx::query_unchecked!(
                r#"
                insert into aircraft_model (
                    icao_code,
                    name
                ) values ($1, $2)
                on conflict (icao_code) do update set name = $2
                "#,
                aircraft.icao_model_code,
                aircraft.model_name,
            ).execute(&mut *transaction).await?;
        }

        set_aircraft_db_commit(&mut transaction, &latest_commit).await?;

        transaction.commit().await?;
    }

    todo!();
}

pub async fn fetch_aircraft_db() -> Result<Vec<Aircraft>, Error> {
    tracing::info!(url = AIRCRAFT_DB_URL, "downloading aircraft db");
    //let csv_gz = reqwest::get(AIRCRAFT_DB_URL)
    //    .await?
    //    .error_for_status()?
    //    .bytes()
    //    .await?;
    //let csv = gzip::Decoder::new(csv_gz.as_ref())?;

    let csv = BufReader::new(File::open("tmp/aircraft.csv")?);
    let csv = csv::ReaderBuilder::new()
        .has_headers(false)
        .delimiter(b';')
        .from_reader(csv);

    #[derive(Debug, Deserialize)]
    struct Row {
        icao_address: String,
        registration: String,
        icao_model_code: String,
        flags: String,
        model_name: Option<String>,
        year: Option<u16>,
        operator: Option<String>,
        unknown3: Option<String>,
    }

    let mut aircrafts = vec![];
    for result in csv.into_deserialize::<Row>() {
        let row = result?;
        aircrafts.push(Aircraft {
            icao_address: row.icao_address.parse()?,
            registration: row.registration,
            icao_model_code: row.icao_model_code,
            flags: row.flags.parse()?,
            model_name: row.model_name,
            registration_year: row.year,
            operator: row.operator,
        });
    }

    Ok(aircrafts)
}

#[derive(Clone, Debug)]
pub struct Aircraft {
    pub icao_address: IcaoAddress,
    pub registration: String,
    pub icao_model_code: String,
    pub flags: AircraftFlags,
    pub model_name: Option<String>,
    pub registration_year: Option<u16>,
    pub operator: Option<String>,
}

bitflags! {
    /// Bits 1-4 are documented [here][1]
    ///
    /// [1]: https://github.com/wiedehopf/readsb/blob/75decb53c0e66f4c12cf24127578a3fe7d919219/README-json.md?plain=1#L112
    ///
    #[derive(Clone, Copy, Debug, Default)]
    pub struct AircraftFlags: u8 {
        const MILITARY = 0b0001;
        const INTERESTING = 0b0010;
        const PIA = 0b0100;
        const LADD = 0b1000;
        const UNKNOWN1 = 0b10000;
    }
}

impl FromStr for AircraftFlags {
    type Err = AircraftFlagsFromStrError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut bits: u8 = 0;

        for c in s.chars() {
            bits <<= 1;
            if c == '1' {
                bits |= 1;
            }
            else if c != '0' {
                return Err(AircraftFlagsFromStrError {
                    input: s.to_owned(),
                });
            }
        }

        AircraftFlags::from_bits(bits).ok_or_else(|| {
            AircraftFlagsFromStrError {
                input: s.to_owned(),
            }
        })
    }
}

#[derive(Clone, Debug, thiserror::Error)]
#[error("invalid aircraft flags: {input}")]
pub struct AircraftFlagsFromStrError {
    input: String,
}
