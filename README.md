# Async bindings for [librtlsdr][1]

This crate provides async bindings for the [librtlsdr][1] C library.

## [Example](examples/hello_rtlsdr.rs)

```rust
// open first RTL-SDR found
let mut rtlsdr = RtlSdr::open(0).unwrap();

// set frequency to 144 MHz
rtlsdr.set_center_frequency(144_000_000).await.unwrap();

// set sample rate to 2 Mhz
rtlsdr.set_sample_rate(2_000_000).await.unwrap();

// enable tuner auto gain and AGC
rtlsdr.set_tuner_gain(Gain::Auto).await.unwrap();
rtlsdr.set_agc_mode(true).await.unwrap();

// create a buffer to hold all 2 million samples, i.e. all samples for 1 s.
let mut buf = vec![IqSample::default(); 2_000_000];

loop {
    // fill buffer
    rtlsdr.read_samples_exact(&mut buf).await.unwrap();

    // average
    let mut average = [0.0; 2];
    for sample in &buf {
        let i = u8_to_f32(sample.i);
        let q = u8_to_f32(sample.q);
        average[0] += i;
        average[1] += q;
    }
    let i = average[0] / 2_000_000.0;
    let q = average[1] / 2_000_000.0;

    println!("{i:.4} {q:.4}i");
}
```