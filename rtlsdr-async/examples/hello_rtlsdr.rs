use futures_util::TryStreamExt;
use rtlsdr_async::{
    Error,
    Gain,
    RtlSdr,
};

#[tokio::main]
async fn main() -> Result<(), Error> {
    // open first RTL-SDR found
    let rtlsdr = RtlSdr::open(0)?;

    // set frequency to 144 MHz
    rtlsdr.set_center_frequency(144_000_000).await?;

    // set sample rate to 2 Mhz
    rtlsdr.set_sample_rate(2_000_000).await?;

    // enable tuner auto gain and AGC
    rtlsdr.set_tuner_gain(Gain::Auto).await?;
    rtlsdr.set_agc_mode(true).await?;

    // start sampling IQ
    let mut stream = rtlsdr.samples().await?;

    let mut power_sum = 0.0;
    let mut num_samples = 0;
    while let Some(chunk) = stream.try_next().await? {
        // average

        for sample in chunk.samples() {
            let i = u8_to_f32(sample.i);
            let q = u8_to_f32(sample.q);
            let power = i * i + q * q;

            power_sum += power;
            num_samples += 1;

            if num_samples == 2_000_000 {
                let n = num_samples as f32;
                let power_avg = power_sum / n;

                let db = 10.0 * power_avg.log10();
                println!("{db:.4} dBFS");

                power_sum = 0.0;
                num_samples = 0;
            }
        }
    }

    Ok(())
}

fn u8_to_f32(x: u8) -> f32 {
    // map the special rtlsdr encoding to f32
    (((x as f32) - 128.0) / 128.0).clamp(-1.0, 1.0)
}
