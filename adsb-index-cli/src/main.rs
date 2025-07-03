use std::{
    fmt::{
        Debug,
        Write as _,
    },
    path::PathBuf,
    pin::Pin,
    time::Instant,
};

use adsb_index_api_client::ApiClient;
use adsb_index_api_server::{
    api::Api,
    database::Database,
    source::{
        beast,
        history::index_archive_day_from_directory,
        mode_s::{
            self,
            adsb,
        },
        rtlsdr,
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
                |i, message| {
                    println!("{i:>4}: {message:?}");
                    Ok::<(), Error>(())
                },
            )
            .await?;
        }
        Command::BeastClient(args) => {
            let mut frame_processor = FrameProcessor::default();

            //todo!();
            // todo: try sending:
            // P
            // 5 heartbeats
            // WO
            args.run(beast::output::Reader::new, |_i, packet| {
                match packet {
                    beast::output::OutputPacket::ModeAc { data, .. } => {
                        println!("modeac: {data:?}");
                    }
                    beast::output::OutputPacket::ModeSLong { data, .. } => {
                        frame_processor.handle_mode_s_data(&data)
                    }
                    beast::output::OutputPacket::ModeSShort { data, .. } => {
                        frame_processor.handle_mode_s_data(&data)
                    }
                    _ => todo!("{packet:?}"),
                }
                Ok::<(), Error>(())
            })
            .await?;

            frame_processor.finish();
        }
        Command::RtlSdr => {
            let mut frame_processor = FrameProcessor::default();

            //let mut rtl_adsb = rtl_tcp::RtlAdsbCommand::new().await?;
            let mut rtl_tcp = rtlsdr::tcp::RtlTcpClient::connect("localhost:1234").await?;
            println!("{:#?}", rtl_tcp.dongle_info());
            rtl_tcp
                .set_center_frequency(rtlsdr::DOWNLINK_FREQUENCY)
                .await?;
            rtl_tcp.set_sample_rate(rtlsdr::SAMPLE_RATE).await?;
            rtl_tcp.set_gain(rtlsdr::tcp::Gain::Auto).await?;

            let mut rtl_adsb = rtlsdr::demodulator::DemodulateStream::new(
                rtl_tcp,
                rtlsdr::demodulator::Quality::NoChecks,
                0x800000,
            );

            while let Some(data) = rtl_adsb.try_next().await? {
                match data {
                    rtlsdr::RawFrame::ModeAc { data } => todo!("mode ac: {data:?}"),
                    rtlsdr::RawFrame::ModeSShort { data } => {
                        frame_processor.handle_mode_s_data(&data)
                    }
                    rtlsdr::RawFrame::ModeSLong { data } => {
                        frame_processor.handle_mode_s_data(&data)
                    }
                }
            }

            frame_processor.finish();
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
    RtlSdr,
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
    pub async fn run<T, F, R, E1, P, E2>(&self, f: F, mut p: P) -> Result<(), Error>
    where
        F: FnOnce(BufReader<Pin<Box<dyn AsyncRead>>>) -> R,
        R: Stream<Item = Result<T, E1>>,
        Error: From<E1> + From<E2>,
        P: FnMut(usize, T) -> Result<(), E2>,
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
            p(i, message)?;
            i += 1;
            if self.limit.map_or(false, |limit| i >= limit) {
                break;
            }
        }

        Ok(())
    }
}

fn make_test(data: &[u8], frame: &mode_s::Frame) {
    let mut bytes_str = String::new();
    let mut bits_str = String::new();

    for b in data {
        write!(&mut bytes_str, "\\x{b:02x}").unwrap();
    }

    for b in &data[4..11] {
        write!(&mut bits_str, " {b:08b}").unwrap();
    }

    println!("//{bits_str}");
    println!("let bytes = b\"{bytes_str}\";");
    println!("frame: {frame:#?}");
    todo!();
}

#[derive(Debug)]
struct FrameProcessor {
    state: State,
    t_start: Instant,
    num_frames: usize,
    num_bytes: usize,
}

impl Default for FrameProcessor {
    fn default() -> Self {
        Self {
            state: Default::default(),
            t_start: Instant::now(),
            num_frames: 0,
            num_bytes: 0,
        }
    }
}

impl FrameProcessor {
    fn handle_mode_s_data(&mut self, data: &[u8]) {
        match mode_s::Frame::decode_and_check_checksum(&mut &data[..]) {
            Ok(frame) => {
                match &frame {
                    mode_s::Frame::ExtendedSquitter(_) => {
                        //make_test(data, &frame);
                        println!("{frame:#?}");
                        println!();
                    }
                    _ => {}
                }

                self.state.update_with_mode_s(Utc::now(), &frame);
                self.num_bytes += data.len();
                self.num_frames += 1;
            }
            Err(error) => {
                match &error {
                    mode_s::DecodeError::CrcCheckFailed(_frame_with_checksum) => {}
                    mode_s::DecodeError::InvalidDf { value: _ } => {
                        // todo: DF-23 ??
                    }
                    _ => panic!("{error:?}"),
                }
                //tracing::error!(?error, ?data);
            }
        }
    }

    fn finish(self) {
        let t_elapsed = self.t_start.elapsed();
        println!(
            "{} frames and {} bytes in {t_elapsed:?}",
            self.num_frames, self.num_bytes
        );
        let seconds = t_elapsed.as_secs_f32();
        println!(
            "{} frames/s, {} MB/s",
            self.num_frames as f32 / seconds,
            self.num_bytes as f32 / seconds / 1024.0 / 1024.0
        );
    }
}
