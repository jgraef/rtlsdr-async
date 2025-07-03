//! <https://k3xec.com/rtl-tcp/>
//! <https://www.idc-online.com/technical_references/pdfs/electronic_engineering/Mode_S_Reply_Encoding.pdf>

use std::{
    pin::Pin,
    process::Stdio,
    task::{
        Context,
        Poll,
    },
};

use bytemuck::{
    Pod,
    Zeroable,
};
use bytes::{
    Buf,
    BufMut,
};
use pin_project_lite::pin_project;
use tokio::{
    io::{
        AsyncBufReadExt,
        AsyncRead,
        AsyncReadExt,
        AsyncWriteExt,
        BufReader,
        ReadBuf,
    },
    net::{
        TcpStream,
        ToSocketAddrs,
    },
    process::{
        Child,
        ChildStdout,
        Command,
    },
};

use crate::util::BufReadBytesExt;

#[derive(Debug, thiserror::Error)]
#[error("rtl_tcp error")]
pub enum Error {
    Io(#[from] std::io::Error),
}

const COMMAND_CENTER_FREQUENCY: u8 = 0x01;
const COMMAND_SAMPLE_RATE: u8 = 0x02;
const COMMAND_TUNER_GAIN_MODE: u8 = 0x03;
const COMMAND_TUNER_LEVEL_GAIN: u8 = 0x04;
const COMMAND_TUNER_FREQUENCY_CORRECTION: u8 = 0x05;
const COMMAND_IF_GAIN_LEVEL: u8 = 0x06;
const COMMAND_TEST_MODE: u8 = 0x07;
const COMMAND_AUTOMATIC_GAIN_CORRECTION: u8 = 0x08;
const COMMAND_DIRECT_SAMPLING: u8 = 0x09;
const COMMAND_OFFSET_TUNING: u8 = 0x0a;

const INPUT_BUFFER_SIZE: usize = 0x800000; // 8 KiB

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

    pub async fn set_center_frequency(&mut self, frequency: u32) -> Result<(), Error> {
        self.send_command(COMMAND_CENTER_FREQUENCY, |buffer| {
            buffer.put_u32(frequency);
        })
        .await
    }

    pub async fn set_sample_rate(&mut self, sample_rate: u32) -> Result<(), Error> {
        self.send_command(COMMAND_SAMPLE_RATE, |buffer| buffer.put_u32(sample_rate))
            .await
    }

    pub fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &mut [Sample],
    ) -> Poll<Result<usize, Error>> {
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

#[derive(Clone, Copy, Pod, Zeroable)]
#[repr(C)]
pub struct Sample {
    pub real: u8,
    pub complex: u8,
}

#[derive(Clone, Copy, Debug)]
struct DongleInfo {
    pub magic: [u8; 4],
    pub tuner_type: u32,
    pub tuner_gain_type: u32,
}

#[derive(Clone, Copy)]
pub enum RawFrame {
    ModeAc { data: [u8; 2] },
    ModeSShort { data: [u8; 7] },
    ModeSLong { data: [u8; 14] },
}

/// Quick and dirty demodulator.
///
/// Spawns `rtl_adsb` and reads it output.
#[derive(Debug)]
pub struct RtlAdsbCommand {
    process: Child,
    stdout: BufReader<ChildStdout>,
    buffer: String,
}

impl RtlAdsbCommand {
    pub async fn new() -> Result<Self, Error> {
        let mut process = Command::new("rtl_adsb")
            .arg("-S")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()?;
        let stdout = BufReader::new(process.stdout.take().expect("missing stdout"));
        Ok(Self {
            process,
            stdout,
            buffer: String::with_capacity(128),
        })
    }

    pub async fn next(&mut self) -> Result<Option<RawFrame>, Error> {
        loop {
            self.buffer.clear();
            if self.stdout.read_line(&mut self.buffer).await? == 0 {
                return Ok(None);
            }

            let line = self.buffer.trim();
            match line.len() {
                16 => {
                    let mut data = [0; 7];
                    if hex::decode_to_slice(&self.buffer[1..15], &mut data).is_ok() {
                        return Ok(Some(RawFrame::ModeSShort { data }));
                    }
                }
                30 => {
                    let mut data = [0; 14];
                    if hex::decode_to_slice(&self.buffer[1..29], &mut data).is_ok() {
                        return Ok(Some(RawFrame::ModeSLong { data }));
                    }
                }
                _ => {}
            }
        }
    }
}
