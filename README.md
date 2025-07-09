# Async bindings for [librtlsdr][1]

This crate provides async bindings for the [librtlsdr][1] C library.

## [Example](rtlsdr-async/examples/hello_rtlsdr.rs)

```rust
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

while let Some(chunk) = stream.try_next().await? {
    // average
    let mut average = [0.0; 2];
    for sample in chunk.samples() {
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

## `rtl_tcp`

This create comes with a client and server implementation for the [`rtl_tcp`][2] protocol.
It is gated behind the `tcp` feature, which is enabled by default.
The `RtlTcpClient` implements the `AsyncReadSamples` and `Configure` traits, and can thus be used somewhat interchangebly with `RtlSdr`.

A server binary is provided in the server directory. To install it, run:

```sh
cargo install --path server
```

Then run `rtl_tcp_rs` to start the server.

Different from the original `rtl_tcp` binary, this version allows multiple clients to connect at once.

If you want log output from `rtl_tcp_rs` set the `RUST_LOG` environment variable:

```sh
RUST_LOG="rtlsdr=debug" rtl_tcp_rs
```

Some client programs (e.g. SDR++) might not work well with IPv6. `rtl_tcp_rs` by default accepts connections on `localhost`, which might resolve to the IPv6 address `::1`. If you suspect having this issue try to use `127.0.0.1` instead:

```sh
rtl_tcp_rs --address "127.0.0.1:1234"
```


[1]: https://gitea.osmocom.org/sdr/rtl-sdr
[2]: https://github.com/rtlsdrblog/rtl-sdr-blog/blob/master/src/rtl_tcp.c