use std::str::FromStr;

use clap::Parser;
use color_eyre::eyre::{
    Error,
    eyre,
};
use rtlsdr_async::{
    RtlSdr,
    rtl_tcp::server::RtlTcpServer,
};
use tokio::net::TcpListener;

#[derive(Debug, Parser)]
struct Args {
    /// Address to listen on
    #[clap(short, long, default_value = "localhost:1234")]
    address: String,

    #[clap(short, long, default_value = "0")]
    device: u32,

    /// Frequency to tune to
    #[clap(short, long)]
    frequency: Option<u32>,

    /// Gain - either 'auto' or in dB
    #[clap(short, long)]
    gain: Option<Gain>,

    /// Sample rate in Hz
    #[clap(short, long, default_value = "2048000")]
    samplerate: u32,
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    let _ = dotenvy::dotenv();
    color_eyre::install()?;
    tracing_subscriber::fmt::init();

    let args = Args::parse();

    let rtl_sdr = RtlSdr::open(args.device)?;
    if let Some(frequency) = args.frequency {
        rtl_sdr.set_center_frequency(frequency).await?;
    }
    if let Some(gain) = args.gain {
        rtl_sdr.set_tuner_gain(gain.into()).await?;
    }
    rtl_sdr.set_sample_rate(args.samplerate).await?;

    let tcp_listener = TcpListener::bind(&args.address).await?;

    RtlTcpServer::from_rtl_sdr(rtl_sdr, tcp_listener)
        .serve()
        .await?;

    Ok(())
}

#[derive(Clone, Copy, Debug)]
enum Gain {
    Auto,
    Manual(i32),
}

impl FromStr for Gain {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s == "auto" {
            Ok(Self::Auto)
        }
        else {
            let gain: f32 = s.parse().map_err(|_| eyre!("Invalid gain value: {s}"))?;
            let gain = (gain * 10.0) as i32;
            Ok(Self::Manual(gain))
        }
    }
}

impl From<Gain> for rtlsdr_async::Gain {
    fn from(value: Gain) -> Self {
        match value {
            Gain::Auto => Self::Auto,
            Gain::Manual(value) => Self::ManualValue(value),
        }
    }
}
