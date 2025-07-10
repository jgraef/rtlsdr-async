use std::error::Error;

use clap::Parser;
use futures_util::TryStreamExt;
use num_complex::Complex;
use rtlsdr_async::{
    Backend,
    Gain,
    RtlSdr,
    rtl_tcp::client::RtlTcpClient,
};

#[derive(Debug, Parser)]
struct Args {
    #[clap(short, long)]
    device: Option<u32>,

    #[clap(short, long)]
    address: Option<String>,

    #[clap(short, long, default_value = "7000000")]
    frequency: u32,

    #[clap(short, long = "samplerate", default_value = "2400000")]
    sample_rate: u32,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    tracing_subscriber::fmt::init();

    let args = Args::parse();
    match (&args.device, &args.address) {
        (device, None) => run(&args, RtlSdr::open(device.unwrap_or_default())?).await?,
        (None, Some(address)) => run(&args, RtlTcpClient::connect(address).await?).await?,
        (Some(_), Some(_)) => panic!("Only either --device or --address can be used at a time"),
    }

    Ok(())
}

async fn run<B: Backend>(args: &Args, rtl_sdr: B) -> Result<(), Box<dyn Error>>
where
    B::Error: Error + 'static,
{
    // set frequency
    rtl_sdr.set_center_frequency(args.frequency).await?;

    // set sample rate to 2.4 MHz
    rtl_sdr.set_sample_rate(args.sample_rate).await?;

    // enable tuner auto gain and AGC
    rtl_sdr.set_tuner_gain(Gain::Auto).await?;
    rtl_sdr.set_agc_mode(true).await?;

    // start sampling IQ
    let mut stream = rtl_sdr.samples().await?;

    // this is really a measure of energy
    let mut power_sum = 0.0;

    let mut num_samples = 0;
    while let Some(chunk) = stream.try_next().await? {
        // average

        for sample in chunk.samples() {
            let sample: Complex<f32> = (*sample).into();
            let power = sample.norm_sqr();

            power_sum += power;
            num_samples += 1;

            if num_samples == args.sample_rate {
                // average power (this is now a proper power measure)
                let power_avg = power_sum / num_samples as f32;

                // convert to dBFS
                let db = 10.0 * power_avg.log10();
                println!("{db:.4} dBFS");

                power_sum = 0.0;
                num_samples = 0;
            }
        }
    }

    Ok(())
}
