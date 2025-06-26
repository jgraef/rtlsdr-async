#![allow(dead_code)]

pub mod app;
pub mod database;
pub mod source;
pub mod spatial;
pub mod types;
pub mod util;

use std::path::PathBuf;

use clap::{
    Parser,
    Subcommand,
};
use color_eyre::eyre::Error;

use crate::{
    database::Database,
    source::{
        aircraft::{fetch_aircraft_db, update_aircraft_db},
        history::index_archive_day_from_directory,
    },
};

#[tokio::main]
async fn main() -> Result<(), Error> {
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

    let _ = dotenvy::dotenv();
    color_eyre::install()?;
    tracing_subscriber::fmt::init();

    let args = Args::parse();

    let database = Database::connect(&args.database_url).await?;

    match args.command {
        Command::IndexArchiveDay { directory, options } => {
            index_archive_day_from_directory(&database, &options, &directory).await?;
        }
        Command::FetchAircraftDb => {
            fetch_aircraft_db().await?;
        }
        Command::UpdateAircraftDb => {
            update_aircraft_db(&database).await?;
        }
    }

    Ok(())
}

#[derive(Debug, Parser)]
pub struct Args {
    #[clap(long, env = "DATABASE_URL")]
    database_url: String,

    #[clap(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    IndexArchiveDay {
        directory: PathBuf,

        #[clap(flatten)]
        options: IndexOptions,
    },
    FetchAircraftDb,
    UpdateAircraftDb,
}

#[derive(Debug, clap::Args)]
pub struct IndexOptions {
    #[clap(long)]
    pub flight_info: bool,

    #[clap(long)]
    pub spatial: bool,
}

impl IndexOptions {
    pub fn normalize(&mut self) {
        if !self.flight_info && !self.spatial {
            self.flight_info = true;
            self.spatial = true;
        }
    }
}
