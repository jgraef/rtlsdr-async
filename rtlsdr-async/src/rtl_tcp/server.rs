use std::{
    fmt::Debug,
    pin::pin,
};

use bytemuck::Pod;
use bytes::BufMut;
use futures_util::TryStreamExt;
use tokio::{
    io::{
        AsyncRead,
        AsyncReadExt,
        AsyncWrite,
        AsyncWriteExt,
        BufReader,
    },
    net::{
        TcpListener,
        TcpStream,
    },
    sync::Mutex,
};
use tokio_util::sync::CancellationToken;
use tracing::Instrument;

use crate::{
    Backend,
    DirectSamplingMode,
    Iq,
    RtlSdr,
    Samples,
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
    Backend(Box<dyn std::error::Error + Send + Sync + 'static>),
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
pub struct RtlTcpServer<B> {
    backend: B,
    dongle_info: DongleInfo,
    tcp_listener: TcpListener,
    shutdown: CancellationToken,
}

impl<B> RtlTcpServer<B> {
    pub fn new(backend: B, tcp_listener: TcpListener, dongle_info: DongleInfo) -> Self {
        Self {
            backend,
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

impl<B> RtlTcpServer<B>
where
    B: Clone + Backend + Send + Sync + Unpin + 'static,
    <B as Backend>::Error: std::error::Error + Send + Sync + 'static,
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
                    let backend = self.backend.clone();
                    let dongle_info = self.dongle_info;
                    let span = tracing::info_span!("connection", %address);
                    tokio::spawn(
                        async move {
                            tracing::debug!(%address, "new connection");
                            if let Err(error) = handle_client(connection, shutdown, backend, dongle_info ).await {
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

async fn handle_client<B>(
    mut tcp: TcpStream,
    shutdown: CancellationToken,
    backend: B,
    dongle_info: DongleInfo,
) -> Result<(), Error>
where
    B: Backend + Send + Unpin + Clone + 'static,
    <B as Backend>::Error: std::error::Error + Send + Sync + 'static,
{
    let mut write_buffer = vec![0u8; WRITE_BUFFER_SIZE];

    {
        let mut header_buffer = &mut write_buffer[..HEADER_LENGTH];
        header_buffer.put(&MAGIC[..]);
        header_buffer.put_u32(dongle_info.tuner_type.0);
        header_buffer.put_u32(dongle_info.tuner_gain_count);
    }
    tcp.write_all(&write_buffer[..HEADER_LENGTH]).await?;
    tcp.flush().await?;

    let (tcp_read, tcp_write) = tcp.split();

    let sample_stream = Mutex::new(SampleStream::new(&backend, None).await?);

    // we only buffer the read half, since we only write in batches anyway.
    let tcp_read = BufReader::with_capacity(READ_BUFFER_SIZE, tcp_read);
    let handle_client_commands = pin!(handle_client_commands(tcp_read, backend, &sample_stream,));
    let forward_samples = pin!(forward_streams(tcp_write, &sample_stream));

    tokio::select! {
        _ = shutdown.cancelled() => {},
        result = handle_client_commands => result?,
        result = forward_samples => result?,
    }

    Ok(())
}

async fn forward_streams<'a, W>(
    mut writer: W,
    sample_stream: &Mutex<SampleStream>,
) -> Result<(), Error>
where
    W: AsyncWrite + Unpin,
{
    loop {
        let mut sample_stream = sample_stream.lock().await;

        match &mut *sample_stream {
            SampleStream::Iq(samples) => {
                forward_samples(&mut writer, samples).await?;
            }
            SampleStream::Direct(samples) => {
                forward_samples(&mut writer, samples).await?;
            }
        }
    }
}

async fn forward_samples<W, S>(mut writer: W, samples: &mut Samples<S>) -> Result<(), Error>
where
    W: AsyncWrite + Unpin,
    S: Pod,
{
    while let Some(chunk) = samples
        .try_next()
        .await
        .map_err(|error| Error::Backend(Box::new(error)))?
    {
        writer.write_all(chunk.as_bytes()).await?;
    }

    Ok(())
}

async fn handle_client_commands<'a, R, B>(
    mut tcp_read: R,
    backend: B,
    sample_stream: &Mutex<SampleStream>,
) -> Result<(), std::io::Error>
where
    R: AsyncRead + Unpin,
    B: Backend + Send + Unpin + 'static,
    <B as Backend>::Error: std::error::Error + Send + Sync + 'static,
{
    let mut read_buffer = [0u8; COMMAND_LENGTH];

    loop {
        if let Err(error) = tcp_read.read_exact(&mut read_buffer[..]).await {
            if error.kind() == std::io::ErrorKind::UnexpectedEof {
                break;
            }
        }

        match Command::decode(&read_buffer[..]) {
            Ok(command) => {
                tracing::debug!(?command);

                // we have to handle this separately, as Command::apply can't really apply it.
                // we'll create a new stream from the backend and send it over to the
                // sample-forwarding future. switching sampling mode will
                if let Command::SetDirectSampling { mode } = &command {
                    match SampleStream::new(&backend, *mode).await {
                        Ok(new_sample_stream) => {
                            *sample_stream.lock().await = new_sample_stream;
                        }
                        Err(error) => {
                            tracing::warn!(
                                ?command,
                                ?error,
                                "error while creating new sample stream"
                            );
                        }
                    }
                }
                else {
                    if let Err(error) = command.apply(&backend).await {
                        tracing::warn!(?command, ?error, "error while handling command");
                    }
                }
            }
            Err(command) => {
                tracing::warn!(?command, "invalid command");
            }
        }
    }

    Ok(())
}

#[derive(Debug)]
enum SampleStream {
    Iq(Samples<Iq>),
    Direct(Samples<u8>),
}

impl SampleStream {
    async fn new<B: Backend>(
        backend: &B,
        direct_sampling_mode: Option<DirectSamplingMode>,
    ) -> Result<Self, Error>
    where
        B: Backend + Send + Unpin + 'static,
        <B as Backend>::Error: std::error::Error + Send + Sync + 'static,
    {
        if let Some(direct_sampling_mode) = direct_sampling_mode {
            Ok(Self::Direct(
                backend
                    .direct_samples(direct_sampling_mode)
                    .await
                    .map_err(|error| Error::Backend(Box::new(error)))?,
            ))
        }
        else {
            Ok(Self::Iq(
                backend
                    .samples()
                    .await
                    .map_err(|error| Error::Backend(Box::new(error)))?,
            ))
        }
    }
}
