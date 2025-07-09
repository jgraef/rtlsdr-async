use std::marker::PhantomData;

use bytes::Buf;
use parking_lot::Mutex;
use tokio::{
    io::{
        AsyncRead,
        AsyncReadExt,
        AsyncWrite,
        AsyncWriteExt,
        BufReader,
        BufWriter,
    },
    net::{
        TcpStream,
        ToSocketAddrs,
    },
    sync::{
        mpsc,
        oneshot,
    },
};

use crate::{
    Backend,
    DirectSamplingMode,
    Gain,
    Iq,
    SampleType,
    Samples,
    TunerType,
    buffer_queue,
    rtl_tcp::{
        BufReadBytesExt,
        Command,
        DongleInfo,
        HEADER_LENGTH,
        MAGIC,
        TunerGainMode,
    },
};

/// size of the read buffer: 8 KiB
const READ_BUFFER_SIZE: usize = 0x2000;

/// size of the write buffer: 1 KiB, plenty for a few command
const WRITE_BUFFER_SIZE: usize = 0x400;

const COMMAND_QUEUE_SIZE: usize = 32;
const SAMPLE_BUFFER_QUEUE_SIZE: usize = 32;
const SAMPLE_BUFFER_SIZE: usize = READ_BUFFER_SIZE;

#[derive(Debug, thiserror::Error)]
#[error("rtl_tcp client error")]
pub enum Error {
    Io(#[from] std::io::Error),
    InvalidMagic([u8; 4]),
    ConnectionClosed,
}

/// A client for `rtl_tcp`
#[derive(Clone, Debug)]
pub struct RtlTcpClient {
    dongle_info: DongleInfo,
    command_sender: mpsc::Sender<ControlMessage>,
    buffer_queue_subscriber: buffer_queue::Subscriber,
}

impl RtlTcpClient {
    /// Connnect to a `rtl_tcp` server.
    ///
    /// This implements [`AsyncReadSamples`] for async reading of IQ samples,
    /// and [`Configure`] to configure the receiver.
    pub async fn connect<A: ToSocketAddrs>(address: A) -> Result<Self, Error> {
        let (connect_result_sender, connect_result_receiver) = oneshot::channel();
        let (command_sender, command_receiver) = mpsc::channel(COMMAND_QUEUE_SIZE);
        let (buffer_queue_sender, buffer_queue_receiver) =
            buffer_queue::channel(SAMPLE_BUFFER_QUEUE_SIZE);

        let tcp = TcpStream::connect(address).await?;

        tokio::spawn(async move {
            if let Err(error) = handle_connection(
                tcp,
                connect_result_sender,
                command_receiver,
                buffer_queue_sender,
            )
            .await
            {
                // todo: propagate error correctly
                tracing::error!(?error);
            }
        });

        let dongle_info = connect_result_receiver
            .await
            .map_err(|_| Error::ConnectionClosed)??;

        tracing::debug!(?dongle_info);

        Ok(Self {
            dongle_info,
            command_sender,
            buffer_queue_subscriber: buffer_queue_receiver,
        })
    }

    pub fn dongle_info(&self) -> &DongleInfo {
        &self.dongle_info
    }

    /// Sends a command to the server.
    pub async fn send_command(&self, command: Command) -> Result<(), Error> {
        let (result_sender, result_receiver) = oneshot::channel();
        self.command_sender
            .send(ControlMessage {
                command,
                result_sender: Some(result_sender),
            })
            .await
            .map_err(|_| Error::ConnectionClosed)?;
        result_receiver.await.map_err(|_| Error::ConnectionClosed)?
    }

    pub async fn set_center_frequency(&self, frequency: u32) -> Result<(), Error> {
        self.send_command(Command::SetCenterFrequency { frequency })
            .await
    }

    pub async fn set_sample_rate(&self, sample_rate: u32) -> Result<(), Error> {
        self.send_command(Command::SetSampleRate { sample_rate })
            .await
    }

    pub async fn set_tuner_gain(&self, gain: Gain) -> Result<(), Error> {
        match gain {
            Gain::ManualValue(gain) => {
                self.send_command(Command::SetTunerGainMode {
                    mode: TunerGainMode::Manual,
                })
                .await?;
                self.send_command(Command::SetTunerGain { gain }).await?;
            }
            Gain::ManualIndex(index) => {
                if let Ok(index) = index.try_into() {
                    self.send_command(Command::SetTunerGainMode {
                        mode: TunerGainMode::Manual,
                    })
                    .await?;
                    self.send_command(Command::SetTunerGainIndex { index })
                        .await?;
                }
                else {
                    tracing::error!(?index, "gain index doesn't fit into an u32!");
                }
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

    pub async fn set_agc_mode(&self, enable: bool) -> Result<(), Error> {
        self.send_command(Command::SetAgcMode { enable }).await
    }

    pub async fn set_frequency_correction(&self, ppm: i32) -> Result<(), Error> {
        self.send_command(Command::SetFrequencyCorrection { ppm })
            .await
    }

    pub async fn set_tuner_if_gain(&self, stage: i16, gain: i16) -> Result<(), Error> {
        self.send_command(Command::SetTunerIfGain { stage, gain })
            .await
    }

    pub async fn set_offset_tuning(&self, enable: bool) -> Result<(), Error> {
        self.send_command(Command::SetOffsetTuning { enable }).await
    }

    pub async fn set_rtl_xtal(&self, frequency: u32) -> Result<(), Error> {
        self.send_command(Command::SetRtlXtal { frequency }).await
    }

    pub async fn set_tuner_xtal(&self, frequency: u32) -> Result<(), Error> {
        self.send_command(Command::SetTunerXtal { frequency }).await
    }

    pub async fn set_bias_tee(&self, enable: bool) -> Result<(), Error> {
        self.send_command(Command::SetBiasT { enable }).await
    }

    pub async fn samples(&self) -> Result<Samples<Iq>, Error> {
        self.send_command(Command::SetDirectSampling { mode: None })
            .await?;
        Ok(Samples {
            receiver: self.buffer_queue_subscriber.receiver(),
            sample_type: SampleType::Iq,
            _phantom: PhantomData,
        })
    }

    pub async fn direct_samples(&self, mode: DirectSamplingMode) -> Result<Samples<u8>, Error> {
        self.send_command(Command::SetDirectSampling { mode: Some(mode) })
            .await?;
        Ok(Samples {
            receiver: self.buffer_queue_subscriber.receiver(),
            sample_type: mode.into(),
            _phantom: PhantomData,
        })
    }
}

impl Backend for RtlTcpClient {
    type Error = Error;

    async fn set_center_frequency(&self, frequency: u32) -> Result<(), Error> {
        RtlTcpClient::set_center_frequency(self, frequency).await
    }

    async fn set_sample_rate(&self, sample_rate: u32) -> Result<(), Error> {
        RtlTcpClient::set_sample_rate(self, sample_rate).await
    }

    async fn set_tuner_gain(&self, gain: Gain) -> Result<(), Error> {
        RtlTcpClient::set_tuner_gain(self, gain).await
    }

    async fn set_agc_mode(&self, enable: bool) -> Result<(), Error> {
        RtlTcpClient::set_agc_mode(self, enable).await
    }

    async fn set_frequency_correction(&self, ppm: i32) -> Result<(), Error> {
        RtlTcpClient::set_frequency_correction(self, ppm).await
    }

    async fn set_tuner_if_gain(&self, stage: i16, gain: i16) -> Result<(), Error> {
        RtlTcpClient::set_tuner_if_gain(self, stage.into(), gain.into()).await
    }

    async fn set_offset_tuning(&self, enable: bool) -> Result<(), Error> {
        RtlTcpClient::set_offset_tuning(self, enable).await
    }

    async fn set_rtl_xtal(&self, frequency: u32) -> Result<(), Error> {
        RtlTcpClient::set_rtl_xtal(self, frequency).await
    }

    async fn set_tuner_xtal(&self, frequency: u32) -> Result<(), Error> {
        RtlTcpClient::set_tuner_xtal(self, frequency).await
    }

    async fn set_bias_tee(&self, enable: bool) -> Result<(), Error> {
        RtlTcpClient::set_bias_tee(self, enable).await
    }

    async fn samples(&self) -> Result<Samples<Iq>, Error> {
        RtlTcpClient::samples(self).await
    }

    async fn direct_samples(&self, mode: DirectSamplingMode) -> Result<Samples<u8>, Error> {
        RtlTcpClient::direct_samples(self, mode).await
    }
}

#[derive(Debug)]
struct ControlMessage {
    command: Command,
    result_sender: Option<oneshot::Sender<Result<(), Error>>>,
}

async fn handle_connection(
    mut tcp: TcpStream,
    connect_result_sender: oneshot::Sender<Result<DongleInfo, Error>>,
    command_receiver: mpsc::Receiver<ControlMessage>,
    buffer_queue_sender: buffer_queue::Sender,
) -> Result<(), Error> {
    let (tcp_read, tcp_write) = tcp.split();
    let mut tcp_read = BufReader::with_capacity(READ_BUFFER_SIZE, tcp_read);
    let tcp_write = BufWriter::with_capacity(WRITE_BUFFER_SIZE, tcp_write);

    match read_dongle_info(&mut tcp_read).await {
        Ok(dongle_info) => {
            let _ = connect_result_sender.send(Ok(dongle_info));
        }
        Err(error) => {
            let _ = connect_result_sender.send(Err(error));
            return Ok(());
        }
    }

    let receiver_state = Mutex::new(Default::default());

    tokio::select! {
        result = forward_commands(command_receiver, tcp_write, &receiver_state) => result?,
        result = forward_samples(tcp_read, buffer_queue_sender, &receiver_state, SAMPLE_BUFFER_SIZE) => result?,
    }

    todo!();
}

#[derive(Debug, Default)]
struct ReceiverState {
    sample_rate: u32,
    sample_type: SampleType,
}

async fn read_dongle_info<R: AsyncRead + Unpin>(mut reader: R) -> Result<DongleInfo, Error> {
    // read dongle info
    let mut header_buffer = [0; HEADER_LENGTH];
    reader.read_exact(&mut header_buffer).await?;

    let mut header_buffer = &header_buffer[..];
    let magic = header_buffer.get_bytes();
    if &magic != MAGIC {
        return Err(Error::InvalidMagic(magic));
    }

    let tuner_type = TunerType(header_buffer.get_u32());
    let tuner_gain_count = header_buffer.get_u32();

    Ok(DongleInfo {
        tuner_type,
        tuner_gain_count,
    })
}

async fn forward_commands<W: AsyncWrite + Unpin>(
    mut command_receiver: mpsc::Receiver<ControlMessage>,
    mut tcp_write: W,
    receiver_state: &Mutex<ReceiverState>,
) -> Result<(), Error> {
    let mut messages = Vec::with_capacity(COMMAND_QUEUE_SIZE);

    loop {
        if command_receiver
            .recv_many(&mut messages, COMMAND_QUEUE_SIZE)
            .await
            == 0
        {
            break;
        }

        for message in messages.drain(..) {
            match &message.command {
                Command::SetSampleRate { sample_rate } => {
                    let mut receiver_state = receiver_state.lock();
                    receiver_state.sample_rate = *sample_rate;
                }
                Command::SetDirectSampling { mode } => {
                    let mut receiver_state = receiver_state.lock();
                    receiver_state.sample_type = (*mode).into();
                }
                _ => {}
            }

            let mut buf = [0; 5];
            message.command.encode(&mut buf[..]);
            match tcp_write.write_all(&buf[..]).await {
                Ok(_) => {
                    if let Some(result_sender) = message.result_sender {
                        let _ = result_sender.send(Ok(()));
                    }
                }
                Err(error) => {
                    if let Some(result_sender) = message.result_sender {
                        let _ = result_sender.send(Err(error.into()));
                    }
                    break;
                }
            }
        }

        tcp_write.flush().await?;
    }

    Ok(())
}

async fn forward_samples<R: AsyncRead + Unpin>(
    mut tcp_read: R,
    mut buffer_queue_sender: buffer_queue::Sender,
    receiver_state: &Mutex<ReceiverState>,
    buffer_size: usize,
) -> Result<(), Error> {
    let mut push_buffer = None;

    loop {
        let Some(mut buffer) = buffer_queue_sender.swap_buffers(push_buffer.take(), buffer_size)
        else {
            // all receivers and subscribers dropped
            tracing::debug!("all readers dropped. exiting");
            break;
        };

        let buffer_mut = buffer.reclaim_or_allocate(buffer_size);
        match tcp_read.read_exact(buffer_mut).await {
            Ok(n_read) => {
                assert_eq!(n_read, buffer_mut.len());

                buffer.filled = n_read;

                let receiver_state = receiver_state.lock();
                buffer.sample_rate = receiver_state.sample_rate;
                buffer.sample_type = receiver_state.sample_type;

                push_buffer = Some(buffer);
            }
            Err(error) => {
                // todo: propagate error
                tracing::error!(?error);
                break;
            }
        }
    }

    Ok(())
}
