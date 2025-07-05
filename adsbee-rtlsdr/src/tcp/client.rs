//! <https://k3xec.com/rtl-tcp/>

use std::{
    pin::Pin,
    task::{
        Context,
        Poll,
    },
};

use bytes::{
    Buf,
    BufMut,
};
use pin_project_lite::pin_project;
use tokio::{
    io::{
        AsyncRead,
        AsyncReadExt,
        AsyncWriteExt,
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
    tcp::DongleInfo,
    util::BufReadBytesExt,
};

#[derive(Debug, thiserror::Error)]
#[error("rtl_tcp client error")]
pub enum Error {
    Io(#[from] std::io::Error),
}

const COMMAND_TUNER_FREQUENCY: u8 = 0x01;
const COMMAND_SAMPLE_RATE: u8 = 0x02;
const COMMAND_TUNER_GAIN_MODE: u8 = 0x03;
const COMMAND_TUNER_GAIN_LEVEL: u8 = 0x04;
//const COMMAND_TUNER_FREQUENCY_CORRECTION: u8 = 0x05;
//const COMMAND_IF_GAIN_LEVEL: u8 = 0x06;
//const COMMAND_TEST_MODE: u8 = 0x07;
const COMMAND_AUTOMATIC_GAIN_CORRECTION: u8 = 0x08;
//const COMMAND_DIRECT_SAMPLING: u8 = 0x09;
//const COMMAND_OFFSET_TUNING: u8 = 0x0a;

pin_project! {
    /// A client for `rtl_tcp`
    #[derive(Debug)]
    pub struct RtlTcpClient {
        #[pin]
        stream: TcpStream,
        dongle_info: DongleInfo,
        incomplete_sample: Option<u8>,
    }
}

impl RtlTcpClient {
    /// Connnect to a `rtl_tcp` server.
    pub async fn connect<A: ToSocketAddrs>(address: A) -> Result<Self, Error> {
        let mut stream = TcpStream::connect(address).await?;

        // read dongle info
        let mut header_buffer = [0; 12];
        stream.read_exact(&mut header_buffer).await?;

        let mut header_buffer = &header_buffer[..];
        let magic = header_buffer.get_bytes();
        let tuner_type = header_buffer.get_u32();
        let tuner_gain_type = header_buffer.get_u32();

        let dongle_info = DongleInfo {
            magic,
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
    async fn send_command<F>(&mut self, command: u8, argument: F) -> Result<(), Error>
    where
        F: FnOnce(&mut &mut [u8]),
    {
        let mut output_buffer = [0; 5];

        output_buffer[0] = command;
        argument(&mut &mut output_buffer[1..]);

        self.stream.write_all(&output_buffer).await?;
        self.stream.flush().await?;
        Ok(())
    }
}

impl Configure for RtlTcpClient {
    type Error = Error;

    async fn set_center_frequency(&mut self, frequency: u32) -> Result<(), Error> {
        self.send_command(COMMAND_TUNER_FREQUENCY, |buffer| {
            buffer.put_u32(frequency);
        })
        .await
    }

    async fn set_sample_rate(&mut self, sample_rate: u32) -> Result<(), Error> {
        self.send_command(COMMAND_SAMPLE_RATE, |buffer| buffer.put_u32(sample_rate))
            .await
    }

    async fn set_gain(&mut self, gain: Gain) -> Result<(), Error> {
        match gain {
            Gain::Manual(gain) => {
                //self.send_command(COMMAND_TUNER_GAIN_MODE, |buffer| buffer.put_u32(1))
                //    .await?;
                self.send_command(COMMAND_TUNER_GAIN_LEVEL, |buffer| buffer.put_u32(gain))
                    .await?;
            }
            Gain::Auto => {
                self.send_command(COMMAND_TUNER_GAIN_MODE, |buffer| buffer.put_u32(0))
                    .await?;
            }
        }
        Ok(())
    }

    async fn set_agc_mode(&mut self, enable: bool) -> Result<(), Error> {
        self.send_command(COMMAND_AUTOMATIC_GAIN_CORRECTION, |buffer| {
            buffer.put_u32(if enable { 1 } else { 0 })
        })
        .await
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
