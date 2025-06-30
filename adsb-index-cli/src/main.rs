use std::{
    fmt::Debug,
    path::PathBuf,
    pin::Pin,
};

use adsb_index_api_client::ApiClient;
use adsb_index_api_server::{
    api::Api,
    database::Database,
    source::{
        adsb,
        beast,
        history::index_archive_day_from_directory,
        sbs,
        tar1090_db::update_aircraft_db,
    },
    tracker::{
        Tracker,
        state::State,
    },
};
use adsb_index_api_types::{
    IcaoAddress,
    Squawk,
    flights::AircraftQuery,
    live::SubscriptionFilter,
};
use chrono::Utc;
use clap::{
    Parser,
    Subcommand,
};
use color_eyre::eyre::{
    Error,
    bail,
};
use futures_util::{
    Stream,
    TryStreamExt,
    pin_mut,
};
use tokio::{
    fs::File,
    io::{
        AsyncRead,
        BufReader,
    },
    net::TcpStream,
};
use uuid::Uuid;

#[tokio::main]
async fn main() -> Result<(), Error> {
    let _ = dotenvy::dotenv();
    color_eyre::install()?;
    tracing_subscriber::fmt::init();

    let args = Args::parse();

    match args.command {
        Command::IndexArchiveDay {
            database_url,
            directory,
        } => {
            let database = Database::connect(&database_url).await?;
            index_archive_day_from_directory(&database, &directory).await?;
        }
        Command::UpdateAircraftDb { database_url, file } => {
            let database = Database::connect(&database_url).await?;
            update_aircraft_db(&database, file.as_deref()).await?;
        }
        Command::Serve {
            database_url,
            listen_address,
        } => {
            let database = Database::connect(&database_url).await?;
            let api = Api::new(Default::default(), database, Tracker::new());
            api.serve(listen_address).await?;
        }
        Command::Live {
            icao,
            callsign,
            squawk,
        } => {
            let api = ApiClient::from_url("https://localhost:8080".parse().unwrap());
            let mut live = api.live().await?;
            live.subscribe(
                Uuid::new_v4(),
                SubscriptionFilter {
                    aircraft: AircraftQuery {
                        icao,
                        callsign,
                        squawk,
                    },
                    area: vec![],
                },
                false,
            )
            .await?;
            while let Some(message) = live.next().await? {
                println!("{message:?}");
            }
        }
        Command::SbsClient(args) => {
            args.run(
                |connection| sbs::Reader::new(connection),
                |i, message| println!("{i:>4}: {message:?}"),
            )
            .await?;
        }
        Command::BeastClient(args) => {
            let mut state = State::default();
            args.run(beast::output::Reader::new, |_i, packet| {
                let frame = match packet {
                    beast::output::OutputPacket::ModeAc { .. } => return,
                    beast::output::OutputPacket::ModeSLong { data, .. } => {
                        adsb::Frame::from_bytes(&data)
                    }
                    beast::output::OutputPacket::ModeSShort { data, .. } => {
                        adsb::Frame::from_bytes(&data)
                    }
                    _ => todo!("{packet:?}"),
                }
                .expect("invalid packet");
                state.update_with_modes_frame(Utc::now(), &frame);
                println!("{frame:?}");
            })
            .await?;
        }
    }

    Ok(())
}

#[derive(Debug, Parser)]
struct Args {
    #[clap(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    IndexArchiveDay {
        #[clap(long, env = "DATABASE_URL")]
        database_url: String,

        directory: PathBuf,
    },
    UpdateAircraftDb {
        #[clap(long, env = "DATABASE_URL")]
        database_url: String,

        #[clap(short, long)]
        file: Option<PathBuf>,
    },
    Serve {
        #[clap(long, env = "DATABASE_URL")]
        database_url: String,

        #[clap(short, long, default_value = "localhost:8080")]
        listen_address: String,
    },
    Live {
        #[clap(short, long)]
        icao: Vec<IcaoAddress>,

        #[clap(short, long)]
        callsign: Vec<String>,

        #[clap(short, long)]
        squawk: Vec<Squawk>,
    },
    SbsClient(ClientTestArgs),
    BeastClient(ClientTestArgs),
}

#[derive(Debug, clap::Args)]
struct ClientTestArgs {
    #[clap(short, long)]
    address: Option<String>,
    #[clap(short, long)]
    file: Option<PathBuf>,
    #[clap(short, long)]
    limit: Option<usize>,
}

impl ClientTestArgs {
    pub async fn run<T, F, R, E, P>(&self, f: F, mut p: P) -> Result<(), Error>
    where
        F: FnOnce(BufReader<Pin<Box<dyn AsyncRead>>>) -> R,
        R: Stream<Item = Result<T, E>>,
        E: std::error::Error + Send + Sync + 'static,
        P: FnMut(usize, T),
    {
        let input: Pin<Box<dyn AsyncRead>> = match (&self.address, &self.file) {
            (Some(address), None) => Box::pin(TcpStream::connect(&address).await?),
            (None, Some(file)) => Box::pin(File::open(&file).await?),
            (Some(_), Some(_)) => bail!("Only one of --address or --file can be used."),
            (None, None) => return Ok(()),
        };

        let reader = f(BufReader::new(input));
        pin_mut!(reader);

        let mut i = 0;
        while let Some(message) = reader.try_next().await? {
            p(i, message);
            i += 1;
            if self.limit.map_or(false, |limit| i >= limit) {
                break;
            }
        }

        Ok(())
    }
}
