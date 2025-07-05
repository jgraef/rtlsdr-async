use std::{
    ops::DerefMut,
    pin::Pin,
    task::{
        Context,
        Poll,
    },
    thread,
};

use futures_util::{
    FutureExt,
    SinkExt,
};
use tokio::sync::{
    mpsc,
    oneshot,
};
use tokio_util::sync::PollSender;

use crate::{
    AsyncReadSamples,
    Configure,
    Gain,
    IqSample,
};

#[derive(Debug, thiserror::Error)]

pub enum Error {
    #[error("{0}")]
    RtlSdr(rtlsdr::RTLSDRError),
    #[error("device handler thread died unexpectedly")]
    DeviceThreadDead,
}

impl From<rtlsdr::RTLSDRError> for Error {
    fn from(value: rtlsdr::RTLSDRError) -> Self {
        Self::RtlSdr(value)
    }
}

pub fn list_devices() -> impl Iterator<Item = DeviceInfo> {
    let n = rtlsdr::get_device_count();
    (0..n).map(|index| DeviceInfo { index })
}

#[derive(Clone, Debug)]
pub struct DeviceInfo {
    index: i32,
}

impl DeviceInfo {
    pub fn name(&self) -> String {
        rtlsdr::get_device_name(self.index)
    }

    pub async fn open(&self) -> Result<RtlSdr, Error> {
        RtlSdr::open(self.index).await
    }
}

#[derive(Debug)]
pub struct RtlSdr {
    command_sender: PollSender<Command>,
    read_receiver: Option<oneshot::Receiver<Result<Vec<u8>, Error>>>,
    half_read_sample: Option<u8>,
    buffer: Vec<u8>,
    buffer_pos: usize,
}

impl AsyncReadSamples for RtlSdr {
    type Error = Error;

    fn poll_read_samples(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &mut [IqSample],
    ) -> Poll<Result<usize, Self::Error>> {
        loop {
            let this = self.deref_mut();

            if this.buffer_pos < this.buffer.len() {
                let buffer: &mut [u8] = bytemuck::cast_slice_mut(buffer);
                let mut buffer_write_pos = 0;

                if buffer.is_empty() {
                    return Poll::Ready(Ok(0));
                }

                if let Some(half_read_sample) = this.half_read_sample.take() {
                    buffer[0] = half_read_sample;
                    buffer_write_pos += 1;
                }

                let mut copy_amount =
                    (buffer.len() - buffer_write_pos).min(this.buffer.len() - this.buffer_pos);
                if copy_amount & 1 == 1 {
                    copy_amount -= 1;
                }

                buffer[buffer_write_pos..][..copy_amount]
                    .copy_from_slice(&this.buffer[this.buffer_pos..][..copy_amount]);
                buffer_write_pos += copy_amount;
                this.buffer_pos += copy_amount;

                if this.buffer_pos == this.buffer.len() - 1 {
                    this.half_read_sample = Some(this.buffer[this.buffer_pos]);
                    this.buffer_pos = 0;
                    this.buffer = vec![];
                }
                else if this.buffer_pos == this.buffer.len() {
                    this.buffer_pos = 0;
                    this.buffer = vec![];
                }

                assert!(buffer_write_pos & 1 == 0);
                return Poll::Ready(Ok(buffer_write_pos >> 1));
            }

            if let Some(read_receiver) = &mut this.read_receiver {
                assert_eq!(this.buffer_pos, 0);
                assert!(this.buffer.is_empty());

                match read_receiver.poll_unpin(cx) {
                    Poll::Pending => return Poll::Pending,
                    Poll::Ready(Err(_)) => return Poll::Ready(Err(Error::DeviceThreadDead)),
                    Poll::Ready(Ok(Err(error))) => return Poll::Ready(Err(error)),
                    Poll::Ready(Ok(Ok(buffer))) => {
                        this.buffer = buffer;
                        // todo: we might want to do this in other match arms as well
                        this.read_receiver = None;
                    }
                }
            }
            else {
                match this.command_sender.poll_reserve(cx) {
                    Poll::Pending => return Poll::Pending,
                    Poll::Ready(Err(_)) => return Poll::Ready(Err(Error::DeviceThreadDead)),
                    Poll::Ready(Ok(())) => {}
                }

                let (read_sender, read_receiver) = oneshot::channel();
                this.read_receiver = Some(read_receiver);

                const CHUNK_SIZE: usize = 16384;
                //let length = buffer.len().div_ceil(CHUNK_SIZE) * CHUNK_SIZE.clamp(CHUNK_SIZE,
                // CHUNK_SIZE * 16);
                let length = CHUNK_SIZE;

                if this
                    .command_sender
                    .send_item(Command::Read {
                        length,
                        result_sender: read_sender,
                    })
                    .is_err()
                {
                    return Poll::Ready(Err(Error::DeviceThreadDead));
                }
            }
        }
    }
}

impl RtlSdr {
    pub async fn open(index: i32) -> Result<Self, Error> {
        let (command_sender, command_receiver) = mpsc::channel(16);
        let (open_result_sender, open_result_receiver) = oneshot::channel();

        thread::spawn(move || {
            device_thread(command_receiver, open_result_sender, index);
        });

        open_result_receiver
            .await
            .map_err(|_| Error::DeviceThreadDead)??;

        Ok(Self {
            command_sender: PollSender::new(command_sender),
            read_receiver: None,
            half_read_sample: None,
            buffer: vec![],
            buffer_pos: 0,
        })
    }
}

impl Configure for RtlSdr {
    type Error = Error;

    async fn set_center_frequency(&mut self, frequency: u32) -> Result<(), Error> {
        let (result_sender, result_receiver) = oneshot::channel();
        self.command_sender
            .send(Command::SetCenterFrequency {
                frequency,
                result_sender,
            })
            .await
            .map_err(|_| Error::DeviceThreadDead)?;
        result_receiver.await.map_err(|_| Error::DeviceThreadDead)?
    }

    async fn set_sample_rate(&mut self, sample_rate: u32) -> Result<(), Error> {
        let (result_sender, result_receiver) = oneshot::channel();
        self.command_sender
            .send(Command::SetSampleRate {
                sample_rate,
                result_sender,
            })
            .await
            .map_err(|_| Error::DeviceThreadDead)?;
        result_receiver.await.map_err(|_| Error::DeviceThreadDead)?
    }

    async fn set_gain(&mut self, gain: Gain) -> Result<(), Error> {
        let (result_sender, result_receiver) = oneshot::channel();
        self.command_sender
            .send(Command::SetGain {
                gain,
                result_sender,
            })
            .await
            .map_err(|_| Error::DeviceThreadDead)?;
        result_receiver.await.map_err(|_| Error::DeviceThreadDead)?
    }

    async fn set_agc_mode(&mut self, enabled: bool) -> Result<(), Error> {
        let (result_sender, result_receiver) = oneshot::channel();
        self.command_sender
            .send(Command::SetAgcMode {
                enabled,
                result_sender,
            })
            .await
            .map_err(|_| Error::DeviceThreadDead)?;
        result_receiver.await.map_err(|_| Error::DeviceThreadDead)?
    }
}

fn device_thread(
    mut command_receiver: mpsc::Receiver<Command>,
    open_result_sender: oneshot::Sender<Result<(), Error>>,
    index: i32,
) {
    match rtlsdr::open(index) {
        Ok(mut device) => {
            let _ = open_result_sender.send(Ok(()));

            while let Some(command) = command_receiver.blocking_recv() {
                //tracing::debug!(?command);

                match command {
                    Command::SetCenterFrequency {
                        frequency,
                        result_sender,
                    } => {
                        let result = device.set_center_freq(frequency);
                        let _ = result_sender.send(result.map_err(Into::into));
                    }
                    Command::SetSampleRate {
                        sample_rate,
                        result_sender,
                    } => {
                        let result = device.set_sample_rate(sample_rate);
                        let _ = result_sender.send(result.map_err(Into::into));
                    }
                    Command::SetGain {
                        gain,
                        result_sender,
                    } => {
                        let result = set_gain(&mut device, gain);
                        let _ = result_sender.send(result);
                    }
                    Command::SetAgcMode {
                        enabled,
                        result_sender,
                    } => {
                        let result = device.set_agc_mode(enabled);
                        let _ = result_sender.send(result.map_err(Into::into));
                    }
                    Command::Read {
                        length,
                        result_sender,
                    } => {
                        if let Err(error) = device.reset_buffer() {
                            let _ = result_sender.send(Err(error.into()));
                        }
                        else {
                            let result = device.read_sync(length);
                            let _ = result_sender.send(result.map_err(Into::into));
                        }
                    }
                }
            }

            let _ = device.close();
        }
        Err(error) => {
            let _ = open_result_sender.send(Err(error.into()));
        }
    }
}

fn set_gain(device: &mut rtlsdr::RTLSDRDevice, gain: Gain) -> Result<(), Error> {
    match gain {
        Gain::Manual(gain) => {
            device.set_tuner_gain_mode(true)?;
            device.set_tuner_gain(gain.try_into().unwrap())?;
        }
        Gain::Auto => {
            device.set_tuner_gain_mode(false)?;
        }
    }
    Ok(())
}

#[derive(Debug)]
enum Command {
    SetCenterFrequency {
        frequency: u32,
        result_sender: oneshot::Sender<Result<(), Error>>,
    },
    SetSampleRate {
        sample_rate: u32,
        result_sender: oneshot::Sender<Result<(), Error>>,
    },
    SetGain {
        gain: Gain,
        result_sender: oneshot::Sender<Result<(), Error>>,
    },
    SetAgcMode {
        enabled: bool,
        result_sender: oneshot::Sender<Result<(), Error>>,
    },
    Read {
        length: usize,
        result_sender: oneshot::Sender<Result<Vec<u8>, Error>>,
    },
}
