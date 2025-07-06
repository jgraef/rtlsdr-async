use std::{
    fmt::{
        Debug,
        Write as _,
    },
    io::{
        BufWriter,
        Write,
    },
    path::PathBuf,
    pin::Pin,
    time::Instant,
};

use adsbee_api_client::ApiClient;
use adsbee_api_server::{
    api::Api,
    database::Database,
    source::{
        history::index_archive_day_from_directory,
        tar1090_db::update_aircraft_db,
    },
    tracker::{
        Tracker,
        state::State,
    },
};
use adsbee_api_types::{
    flights::AircraftQuery,
    live::SubscriptionFilter,
};
use adsbee_beast as beast;
use adsbee_mode_s as mode_s;
use adsbee_rtlsdr::{
    AsyncReadSamples,
    Configure,
    RawFrame,
    RtlSdr,
    demodulator::{
        DemodulateStream,
        Demodulator,
        Quality,
    },
    tcp::{
        client::RtlTcpClient,
        server::RtlSdrServer,
    },
};
use adsbee_sbs as sbs;
use adsbee_types::{
    IcaoAddress,
    Squawk,
};
use byteorder::{
    BigEndian,
    WriteBytesExt,
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
    io::{
        AsyncRead,
        BufReader,
    },
    net::{
        TcpListener,
        TcpStream,
    },
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
                        frame_processor.handle_mode_s_data(&data);
                    }
                    beast::output::OutputPacket::ModeSShort { data, .. } => {
                        frame_processor.handle_mode_s_data(&data);
                    }
                    _ => todo!("{packet:?}"),
                }
                Ok::<(), Error>(())
            })
            .await?;

            frame_processor.finish();
        }
        Command::RtlSdr {
            dump,
            address,
            device,
            frequency,
        } => {
            let mut dump_file = dump
                .as_deref()
                .map(|path| std::fs::OpenOptions::new().append(true).open(path))
                .transpose()?
                .map(BufWriter::new);

            let dump_file = |tag, data: &[u8]| {
                if let Some(dump_file) = &mut dump_file {
                    let timestamp = Utc::now();
                    dump_file.write_u8(tag)?;
                    dump_file.write_i64::<BigEndian>(timestamp.timestamp_nanos_opt().unwrap())?;
                    dump_file.write_all(data)?;
                    dump_file.flush()?;
                }
                Ok::<(), Error>(())
            };

            if let Some(address) = &address {
                if device.is_some() {
                    bail!("Both --address and --device set. Either one must be used.");
                }

                let rtl_tcp = RtlTcpClient::connect(&address).await?;
                println!("{:#?}", rtl_tcp.dongle_info());

                run_rtl_sdr(rtl_tcp, frequency, dump_file).await?;
            }
            else {
                let rtl_sdr = RtlSdr::open(device.unwrap_or_default().try_into().unwrap())?;
                run_rtl_sdr(rtl_sdr, frequency, dump_file).await?;
            }

            async fn run_rtl_sdr<S: AsyncReadSamples + Configure + Unpin>(
                rtl_sdr: S,
                frequency: Option<u32>,
                mut dump_file: impl FnMut(u8, &[u8]) -> Result<(), Error>,
            ) -> Result<(), Error>
            where
                Error: From<<S as AsyncReadSamples>::Error> + From<<S as Configure>::Error>,
            {
                let mut rtl_adsb = DemodulateStream::new(
                    rtl_sdr,
                    Demodulator::new(Quality::NoChecks, 5),
                    0x800000,
                );
                rtl_adsb.configure(frequency).await?;

                let mut frame_processor = FrameProcessor::default();

                while let Some(data) = rtl_adsb.try_next().await? {
                    match data {
                        RawFrame::ModeAc { data } => todo!("mode ac: {data:?}"),
                        RawFrame::ModeSShort { data } => {
                            if frame_processor.handle_mode_s_data(&data) {
                                dump_file(7, &data)?;
                            }
                        }
                        RawFrame::ModeSLong { data } => {
                            if frame_processor.handle_mode_s_data(&data) {
                                dump_file(14, &data)?;
                            }
                        }
                    }
                }

                frame_processor.finish();
                Ok(())
            }
        }
        Command::RtlSdrServer { device, address } => {
            let rtl_sdr = RtlSdr::open(device.unwrap_or_default().try_into().unwrap())?;
            let tcp_listener = TcpListener::bind(address).await?;
            RtlSdrServer::from_rtl_sdr(rtl_sdr, tcp_listener)
                .serve()
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
    RtlSdr {
        /// Dump all verified frames to file.
        #[clap(short = 'D', long)]
        dump: Option<PathBuf>,

        /// Connect to rtl_tcp instead of using the RTL-SDR directly.
        #[clap(short, long)]
        address: Option<String>,

        /// Device index
        ///
        /// Defaults to using the first.
        #[clap(short, long)]
        device: Option<usize>,

        /// Select non-default frequency
        ///
        /// in Hz. Default: 1090Mhz
        #[clap(short, long)]
        frequency: Option<u32>,
    },
    RtlSdrServer {
        /// Device index
        ///
        /// Defaults to using the first.
        #[clap(short, long)]
        device: Option<usize>,

        address: String,
    },
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
            (None, Some(file)) => Box::pin(tokio::fs::File::open(&file).await?),
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

#[allow(dead_code)]
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
    fn handle_mode_s_data(&mut self, data: &[u8]) -> bool {
        //println!("{}", hex::encode(&data));
        match mode_s::Frame::decode_and_calculate_checksum(&mut &data[..]) {
            Ok(frame) => {
                match frame.check() {
                    Some(true) => {
                        //make_test(data, &frame);
                        println!("{:#?}", frame.frame);
                        self.state.update_with_mode_s(Utc::now(), &frame.frame);
                        self.num_bytes += data.len();
                        self.num_frames += 1;
                        return true;
                    }
                    _ => {}
                }
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

        false
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
