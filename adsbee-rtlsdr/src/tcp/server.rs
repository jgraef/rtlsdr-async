use bytes::BufMut;
use tokio::{
    io::{
        AsyncReadExt,
        AsyncWriteExt,
        BufReader,
    },
    net::{
        TcpListener,
        TcpStream,
    },
};
use tokio_util::sync::CancellationToken;

use crate::{
    AsyncReadSamples,
    AsyncReadSamplesExt,
    Configure,
    Gain,
    RtlSdr,
    TunerGainMode,
    tcp::{
        COMMAND_LENGTH,
        Command,
        DongleInfo,
        HEADER_LENGTH,
        MAGIC,
    },
};

#[derive(Debug, thiserror::Error)]
#[error("rtl_tcp server error")]
pub enum Error {
    Io(#[from] std::io::Error),
    Device(Box<dyn std::error::Error + Send + Sync + 'static>),
}

#[derive(Debug)]
pub struct RtlSdrServer<S> {
    stream: S,
    dongle_info: DongleInfo,
    tcp_listener: TcpListener,
    shutdown: CancellationToken,
    //buffer: Vec<u8>,
}

impl<S> RtlSdrServer<S> {
    pub fn new(stream: S, tcp_listener: TcpListener, dongle_info: DongleInfo) -> Self {
        Self {
            stream,
            dongle_info,
            tcp_listener,
            shutdown: CancellationToken::new(),
        }
    }

    pub fn with_shutdown(mut self, shutdown: CancellationToken) -> Self {
        self.shutdown = shutdown;
        self
    }
}

impl RtlSdrServer<RtlSdr> {
    pub fn from_rtl_sdr(rtl_sdr: RtlSdr, tcp_listener: TcpListener) -> Self {
        let dongle_info = DongleInfo {
            tuner_type: rtl_sdr.get_tuner_type(),
            tuner_gain_type: 0, // todo
        };
        Self::new(rtl_sdr, tcp_listener, dongle_info)
    }
}

impl<S> RtlSdrServer<S>
where
    S: Clone + AsyncReadSamples + Configure + Send + Unpin + 'static,
    <S as AsyncReadSamples>::Error: std::error::Error + Send + Sync + 'static,
    <S as Configure>::Error: std::error::Error + Send + Sync + 'static,
{
    pub async fn serve(self) -> Result<(), Error> {
        tracing::debug!("waiting for connections");

        loop {
            tokio::select! {
                _ = self.shutdown.cancelled() => break,
                result = self.tcp_listener.accept() => {
                    let (connection, address) = result?;
                    tracing::debug!(%address, "new connection");
                    tokio::spawn(handle_client(connection, self.shutdown.clone(), self.stream.clone(), self.dongle_info ));
                }
            }
        }

        Ok(())
    }
}

/// size of the read buffer: 1 KiB, plenty for a few command
const READ_BUFFER_SIZE: usize = 0x400;

/// size of the write buffer: 8 KiB
const WRITE_BUFFER_SIZE: usize = 0x2000;

async fn handle_client<S>(
    mut tcp: TcpStream,
    shutdown: CancellationToken,
    mut stream: S,
    dongle_info: DongleInfo,
) -> Result<(), Error>
where
    S: AsyncReadSamples + Configure + Send + Unpin + 'static,
    <S as AsyncReadSamples>::Error: std::error::Error + Send + Sync + 'static,
    <S as Configure>::Error: std::error::Error + Send + Sync + 'static,
{
    let mut write_buffer = vec![0u8; WRITE_BUFFER_SIZE];
    let mut read_buffer = [0u8; COMMAND_LENGTH];
    let mut read_buffer_cursor = &mut read_buffer[..];

    {
        let mut header_buffer = &mut write_buffer[..HEADER_LENGTH];
        header_buffer.put(&MAGIC[..]);
        header_buffer.put_u32(dongle_info.tuner_type.0);
        header_buffer.put_u32(dongle_info.tuner_gain_type);
    }
    tcp.write_all(&write_buffer[..HEADER_LENGTH]).await?;
    tcp.flush().await?;

    let (tcp_read, mut tcp_write) = tcp.split();

    // we only buffer the read half, since we only write in batches anyway.
    let mut tcp_read = BufReader::with_capacity(READ_BUFFER_SIZE, tcp_read);

    loop {
        tokio::select! {
            _ = shutdown.cancelled() => {
                break;
            }
            result = tcp_read.read_buf(&mut read_buffer_cursor) => {
                if result? == 0 {
                    break;
                }
                if !read_buffer_cursor.has_remaining_mut() {
                    handle_client_command(&read_buffer, &mut stream).await?;
                    read_buffer_cursor = &mut read_buffer[..];
                }
            }
            result = stream.read_samples(bytemuck::cast_slice_mut(&mut write_buffer)) => {
                let samples_read = result
                    .map_err(|error| Error::Device(Box::new(error)))?;

                if samples_read == 0 {
                    break;
                }

                // note: we could put this write into the select, so that we can accept commands while writing out a chunk of samples
                tcp_write.write_all(&write_buffer[0..samples_read * 2]).await?;
                tcp_write.flush().await?;
            }
        }
    }

    tracing::debug!("closing connection");
    Ok(())
}

async fn handle_client_command<S>(
    command: &[u8; COMMAND_LENGTH],
    stream: &mut S,
) -> Result<(), Error>
where
    S: Configure + Unpin,
    <S as Configure>::Error: std::error::Error + Send + Sync + 'static,
{
    match Command::decode(&command[..]) {
        Ok(command) => {
            match command {
                Command::SetCenterFrequency { frequency } => {
                    stream
                        .set_center_frequency(frequency)
                        .await
                        .map_err(|error| Error::Device(Box::new(error)))?
                }
                Command::SetSampleRate { sample_rate } => {
                    stream
                        .set_sample_rate(sample_rate)
                        .await
                        .map_err(|error| Error::Device(Box::new(error)))?;
                }
                Command::SetTunerGainMode { mode } => {
                    if mode == TunerGainMode::Auto {
                        stream
                            .set_tuner_gain(Gain::Auto)
                            .await
                            .map_err(|error| Error::Device(Box::new(error)))?;
                    }
                    else {
                        // don't do anything here. SetTunerGainLevel will set
                        // the mode to manual automatically
                    }
                }
                Command::SetTunerGainLevel { gain } => {
                    stream
                        .set_tuner_gain(Gain::Manual(gain))
                        .await
                        .map_err(|error| Error::Device(Box::new(error)))?;
                }
                Command::SetFrequencyCorrection { ppm } => {
                    stream
                        .set_frequency_correction(ppm)
                        .await
                        .map_err(|error| Error::Device(Box::new(error)))?;
                }
                Command::SetTunerIfGain { stage, gain } => {
                    stream
                        .set_tuner_if_gain(stage, gain)
                        .await
                        .map_err(|error| Error::Device(Box::new(error)))?;
                }
                Command::SetTestMode { enable: _ } => {
                    // not supported
                }
                Command::SetAgcMode { enable } => {
                    stream
                        .set_agc_mode(enable)
                        .await
                        .map_err(|error| Error::Device(Box::new(error)))?;
                }
                Command::SetDirectSampling { mode: _ } => {
                    // not supported
                }
                Command::SetOffsetTuning { enable } => {
                    stream
                        .set_offset_tuning(enable)
                        .await
                        .map_err(|error| Error::Device(Box::new(error)))?;
                }
                Command::SetRtlXtal { frequency } => {
                    stream
                        .set_rtl_xtal(frequency)
                        .await
                        .map_err(|error| Error::Device(Box::new(error)))?;
                }
                Command::SetTunerXtal { frequency } => {
                    stream
                        .set_tuner_xtal(frequency)
                        .await
                        .map_err(|error| Error::Device(Box::new(error)))?;
                }
                Command::SetTunerGainLevelIndex { index: _ } => {
                    // todo: we can obviously support it, but it's kind of
                    // awkward with the interface we have. open to suggestions
                }
                Command::SetBiasT { enable } => {
                    stream
                        .set_bias_t(enable)
                        .await
                        .map_err(|error| Error::Device(Box::new(error)))?;
                }
            }
        }
        Err(command) => {
            tracing::debug!(?command, "invalid command");
        }
    }

    Ok(())
}
