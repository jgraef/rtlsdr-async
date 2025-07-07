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
use tracing::Instrument;

use crate::{
    AsyncReadSamples,
    AsyncReadSamplesExt,
    Configure,
    Gain,
    RtlSdr,
    TunerGainMode,
    rtl_tcp::{
        COMMAND_LENGTH,
        Command,
        DongleInfo,
        HEADER_LENGTH,
        MAGIC,
    },
};

/// Server errors
#[derive(Debug, thiserror::Error)]
#[error("rtl_tcp server error")]
pub enum Error {
    Io(#[from] std::io::Error),

    /// Error from the underlying stream, e.g. the rtlsdr device, or another
    /// `rtl_tcp`` client.
    Device(Box<dyn std::error::Error + Send + Sync + 'static>),
}

/// A `rtl_tcp` server.
///
/// Different from the original `rtl_tcp` this accepts multiple connections at
/// once.
///
/// It is usually created from a [`RtlSdr`], but be created from
/// anything that implements the [`AsyncReadSamples`] and [`Configure`] traits,
/// e.g. a [`RtlTcpClient`][crate::rtl_tcp::client::RtlTcpClient]
#[derive(Debug)]
pub struct RtlTcpServer<S> {
    stream: S,
    dongle_info: DongleInfo,
    tcp_listener: TcpListener,
    shutdown: CancellationToken,
}

impl<S> RtlTcpServer<S> {
    pub fn new(stream: S, tcp_listener: TcpListener, dongle_info: DongleInfo) -> Self {
        Self {
            stream,
            dongle_info,
            tcp_listener,
            shutdown: CancellationToken::new(),
        }
    }

    /// Provide a [`CancellationToken`] with which the server (and all client
    /// connections) can be shut down.
    pub fn with_shutdown(mut self, shutdown: CancellationToken) -> Self {
        self.shutdown = shutdown;
        self
    }
}

impl RtlTcpServer<RtlSdr> {
    /// This will populate a [`DongleInfo`] and call [`RtlTcpServer::new`] with
    /// it.
    pub fn from_rtl_sdr(rtl_sdr: RtlSdr, tcp_listener: TcpListener) -> Self {
        let dongle_info = DongleInfo {
            tuner_type: rtl_sdr.get_tuner_type(),
            tuner_gain_count: rtl_sdr
                .get_tuner_gains()
                .len()
                .try_into()
                .expect("number of tuner gains doesn't fit into an u32"),
        };
        Self::new(rtl_sdr, tcp_listener, dongle_info)
    }
}

impl<S> RtlTcpServer<S>
where
    S: Clone + AsyncReadSamples + Configure + Send + Unpin + 'static,
    <S as AsyncReadSamples>::Error: std::error::Error + Send + Sync + 'static,
    <S as Configure>::Error: std::error::Error + Send + Sync + 'static,
{
    /// Serve incoming connections
    pub async fn serve(self) -> Result<(), Error> {
        tracing::debug!("waiting for connections");

        loop {
            tokio::select! {
                _ = self.shutdown.cancelled() => break,
                result = self.tcp_listener.accept() => {
                    let (connection, address) = result?;
                    let shutdown = self.shutdown.clone();
                    let stream = self.stream.clone();
                    let dongle_info = self.dongle_info;
                    let span = tracing::info_span!("connection", %address);
                    tokio::spawn(
                        async move {
                            tracing::debug!(%address, "new connection");
                            if let Err(error) = handle_client(connection, shutdown, stream, dongle_info ).await {
                                tracing::error!(?error);
                            }
                            tracing::debug!(%address, "closing connection");
                        }.instrument(span)
                    );
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
        header_buffer.put_u32(dongle_info.tuner_gain_count);
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
                    match Command::decode(&read_buffer[..]) {
                        Ok(command) => {
                            if let Err(error) = handle_client_command(command, &mut stream).await {
                                tracing::warn!(?command, ?error, "error while handling command");
                            }
                        }
                        Err(command) => {
                            tracing::warn!(?command, "invalid command");
                        }
                    }
                    read_buffer_cursor = &mut read_buffer[..];
                }
            }
            result = stream.read_samples(bytemuck::cast_slice_mut(&mut write_buffer)) => {
                let samples_read = result
                    .map_err(|error| Error::Device(Box::new(error)))?;

                if samples_read == 0 {
                    break;
                }

                // todo: we could put this write into the select, so that we can accept commands while writing out a chunk of samples
                tcp_write.write_all(&write_buffer[0..samples_read * 2]).await?;
                tcp_write.flush().await?;
            }
        }
    }

    Ok(())
}

async fn handle_client_command<S>(command: Command, stream: &mut S) -> Result<(), Error>
where
    S: Configure + Unpin,
    <S as Configure>::Error: std::error::Error + Send + Sync + 'static,
{
    tracing::debug!(?command);
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
        Command::SetTunerGain { gain } => {
            stream
                .set_tuner_gain(Gain::ManualValue(gain))
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
        Command::SetTunerGainIndex { index } => {
            if let Ok(index) = index.try_into() {
                stream
                    .set_tuner_gain(Gain::ManualIndex(index))
                    .await
                    .map_err(|error| Error::Device(Box::new(error)))?;
            }
            else {
                tracing::error!(?index, "gain index doesn't fit into an usize!");
            }
        }
        Command::SetBiasT { enable } => {
            stream
                .set_bias_tee(enable)
                .await
                .map_err(|error| Error::Device(Box::new(error)))?;
        }
    }

    Ok(())
}
