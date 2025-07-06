//! <https://k3xec.com/rtl-tcp/>

use std::{
    pin::Pin,
    task::{
        Context,
        Poll,
    },
};

use bytes::Buf;
use pin_project_lite::pin_project;
use tokio::{
    io::{
        AsyncRead,
        AsyncReadExt,
        AsyncWriteExt,
        BufStream,
        ReadBuf,
    },
    net::{
        TcpStream,
        ToSocketAddrs,
    },
};

use crate::{
    AsyncReadSamples,
    Configure,
    Gain,
    IqSample,
    TunerType,
    tcp::{
        COMMAND_LENGTH,
        Command,
        DongleInfo,
        HEADER_LENGTH,
        MAGIC,
        TunerGainMode,
    },
    util::BufReadBytesExt,
};

/// size of the read buffer: 8 KiB
const READ_BUFFER_SIZE: usize = 0x2000;

/// size of the write buffer: 1 KiB, plenty for a few command
const WRITE_BUFFER_SIZE: usize = 0x400;

#[derive(Debug, thiserror::Error)]
#[error("rtl_tcp client error")]
pub enum Error {
    Io(#[from] std::io::Error),
    InvalidMagic([u8; 4]),
}

pin_project! {
    /// A client for `rtl_tcp`
    #[derive(Debug)]
    pub struct RtlTcpClient {
        #[pin]
        stream: BufStream<TcpStream>,
        dongle_info: DongleInfo,
        incomplete_sample: Option<u8>,
    }
}

impl RtlTcpClient {
    /// Connnect to a `rtl_tcp` server.
    pub async fn connect<A: ToSocketAddrs>(address: A) -> Result<Self, Error> {
        let mut stream = BufStream::with_capacity(
            READ_BUFFER_SIZE,
            WRITE_BUFFER_SIZE,
            TcpStream::connect(address).await?,
        );

        // read dongle info
        let mut header_buffer = [0; HEADER_LENGTH];
        stream.read_exact(&mut header_buffer).await?;

        let mut header_buffer = &header_buffer[..];
        let magic = header_buffer.get_bytes();
        if &magic != MAGIC {
            return Err(Error::InvalidMagic(magic));
        }

        let tuner_type = TunerType(header_buffer.get_u32());
        let tuner_gain_type = header_buffer.get_u32();

        let dongle_info = DongleInfo {
            tuner_type,
            tuner_gain_type,
        };
        tracing::debug!(?dongle_info);

        Ok(Self {
            stream,
            dongle_info,
            incomplete_sample: None,
        })
    }

    pub fn dongle_info(&self) -> &DongleInfo {
        &self.dongle_info
    }

    // todo: this always flushes the stream. would be nice to only flush once you're
    // done configuring
    async fn send_command(&mut self, command: Command) -> Result<(), Error> {
        let mut output_buffer = [0; COMMAND_LENGTH];
        command.encode(&mut output_buffer[..]);
        self.stream.write_all(&output_buffer).await?;
        self.stream.flush().await?;
        Ok(())
    }
}

impl Configure for RtlTcpClient {
    type Error = Error;

    async fn set_center_frequency(&mut self, frequency: u32) -> Result<(), Error> {
        self.send_command(Command::SetCenterFrequency { frequency })
            .await
    }

    async fn set_sample_rate(&mut self, sample_rate: u32) -> Result<(), Error> {
        self.send_command(Command::SetSampleRate { sample_rate })
            .await
    }

    async fn set_tuner_gain(&mut self, gain: Gain) -> Result<(), Error> {
        match gain {
            Gain::Manual(gain) => {
                self.send_command(Command::SetTunerGainMode {
                    mode: TunerGainMode::Manual,
                })
                .await?;
                self.send_command(Command::SetTunerGainLevel { gain })
                    .await?;
            }
            Gain::Auto => {
                self.send_command(Command::SetTunerGainMode {
                    mode: TunerGainMode::Auto,
                })
                .await?;
            }
        }
        Ok(())
    }

    async fn set_agc_mode(&mut self, enable: bool) -> Result<(), Error> {
        self.send_command(Command::SetAgcMode { enable }).await
    }

    async fn set_frequency_correction(&mut self, ppm: i32) -> Result<(), Error> {
        self.send_command(Command::SetFrequencyCorrection { ppm })
            .await
    }

    async fn set_tuner_if_gain(&mut self, stage: i16, gain: i16) -> Result<(), Error> {
        self.send_command(Command::SetTunerIfGain { stage, gain })
            .await
    }

    async fn set_offset_tuning(&mut self, enable: bool) -> Result<(), Error> {
        self.send_command(Command::SetOffsetTuning { enable }).await
    }

    async fn set_rtl_xtal(&mut self, frequency: u32) -> Result<(), Error> {
        self.send_command(Command::SetRtlXtal { frequency }).await
    }

    async fn set_tuner_xtal(&mut self, frequency: u32) -> Result<(), Error> {
        self.send_command(Command::SetTunerXtal { frequency }).await
    }

    async fn set_bias_t(&mut self, enable: bool) -> Result<(), Error> {
        self.send_command(Command::SetBiasT { enable }).await
    }
}

impl AsyncReadSamples for RtlTcpClient {
    type Error = Error;

    fn poll_read_samples(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &mut [IqSample],
    ) -> Poll<Result<usize, Error>> {
        if buffer.is_empty() {
            return Poll::Ready(Ok(0));
        }

        let this = self.project();

        let buffer_bytes: &mut [u8] = bytemuck::cast_slice_mut(buffer);

        let read_offset = if let Some(incomplete_sample) = this.incomplete_sample.take() {
            buffer_bytes[0] = incomplete_sample;
            1
        }
        else {
            0
        };

        let mut read_buf = ReadBuf::new(&mut buffer_bytes[read_offset..]);
        match this.stream.poll_read(cx, &mut read_buf) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Err(error)) => Poll::Ready(Err(error.into())),
            Poll::Ready(Ok(())) => {
                let num_bytes_read = read_buf.filled().len();
                if num_bytes_read == 0 {
                    Poll::Ready(Ok(0))
                }
                else {
                    let num_bytes = read_offset + num_bytes_read;

                    // number of complete samples in buffer
                    let num_samples = num_bytes >> 1;

                    if num_bytes & 1 != 0 {
                        // the last sample is incomplete
                        *this.incomplete_sample = Some(buffer_bytes[num_bytes - 1]);
                    }

                    Poll::Ready(Ok(num_samples))
                }
            }
        }
    }
}
