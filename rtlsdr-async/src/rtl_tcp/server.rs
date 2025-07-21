use std::{
    fmt::Debug,
    net::SocketAddr,
};

use bytes::{
    BufMut,
    buf::UninitSlice,
};
use futures_util::TryStreamExt;
use tokio::{
    io::{
        AsyncReadExt,
        AsyncWrite,
        AsyncWriteExt,
        BufReader,
        BufWriter,
    },
    net::{
        TcpListener,
        TcpStream,
    },
};
use tokio_util::sync::CancellationToken;
use tracing::Instrument;

use crate::{
    Backend,
    Chunk,
    DongleInfo,
    Iq,
    RtlSdr,
    Samples,
    rtl_tcp::{
        COMMAND_LENGTH,
        Command,
        InvalidCommand,
    },
};

/// Server errors
#[derive(Debug, thiserror::Error)]
#[error("rtl_tcp server error")]
pub enum Error<H> {
    Io(#[from] std::io::Error),

    /// Error from the underlying stream, e.g. the rtlsdr device, or another
    /// `rtl_tcp`` client.
    Handler(H),

    InvalidCommand(#[from] InvalidCommand),
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
pub struct RtlTcpServer<H> {
    handler: H,
    tcp_listener: TcpListener,
    shutdown: CancellationToken,
}

impl<H> RtlTcpServer<H> {
    pub fn new(handler: H, tcp_listener: TcpListener) -> Self {
        Self {
            handler,
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

impl<B: Backend> RtlTcpServer<BackendHandler<B>> {
    pub fn from_backend(backend: B, tcp_listener: TcpListener) -> Self {
        Self::new(BackendHandler::new(backend), tcp_listener)
    }
}

impl RtlTcpServer<BackendHandler<RtlSdr>> {
    /// This will populate a [`DongleInfo`] and call [`RtlTcpServer::new`] with
    /// it.
    pub fn from_rtl_sdr(rtl_sdr: RtlSdr, tcp_listener: TcpListener) -> Self {
        Self::from_backend(rtl_sdr, tcp_listener)
    }
}

impl<H> RtlTcpServer<H>
where
    H: Handler,
{
    /// Serve incoming connections
    pub async fn serve(mut self) -> Result<(), Error<H::Error>> {
        tracing::debug!("waiting for connections");

        loop {
            tokio::select! {
                _ = self.shutdown.cancelled() => break,
                result = self.tcp_listener.accept() => {


                    let (connection, address) = result?;
                    if let Err(error) = self.handle_accept(connection, address).await {
                        tracing::error!(?error);
                    }
                }
            }
        }

        Ok(())
    }

    async fn handle_accept(
        &mut self,
        connection: TcpStream,
        address: SocketAddr,
    ) -> Result<(), Error<H::Error>> {
        let shutdown = self.shutdown.clone();

        if let Some(handler) = self
            .handler
            .accept_connection(address)
            .await
            .map_err(Error::Handler)?
        {
            let span = tracing::info_span!("connection", %address);

            tokio::spawn(
                async move {
                    tracing::debug!(%address, "new connection");
                    if let Err(error) = handle_connection(connection, shutdown, handler).await {
                        tracing::error!(?error);
                    }
                    tracing::debug!(%address, "closing connection");
                }
                .instrument(span),
            );
        }

        Ok(())
    }
}

#[derive(Debug, Default)]
struct CommandBuffer {
    data: [u8; COMMAND_LENGTH],
    filled: usize,
}

impl CommandBuffer {
    pub fn is_full(&self) -> bool {
        self.filled == COMMAND_LENGTH
    }

    pub fn reset(&mut self) {
        self.filled = 0;
    }

    pub fn try_decode(&mut self) -> Result<Option<Command>, InvalidCommand> {
        if self.is_full() {
            let command = Command::decode(&self.data[..])?;
            self.reset();
            Ok(Some(command))
        }
        else {
            Ok(None)
        }
    }
}

unsafe impl BufMut for CommandBuffer {
    fn remaining_mut(&self) -> usize {
        COMMAND_LENGTH - self.filled
    }

    unsafe fn advance_mut(&mut self, cnt: usize) {
        self.filled += cnt;
        assert!(self.filled <= COMMAND_LENGTH);
    }

    fn chunk_mut(&mut self) -> &mut UninitSlice {
        UninitSlice::new(&mut self.data[self.filled..])
    }
}

#[derive(Debug)]
struct SampleBuffer {
    samples: Vec<Iq>,
    write_pos: usize,
    read_pos: usize,
}

impl SampleBuffer {
    const DEFAULT_CAPACITY: usize = 0x4000;

    pub fn new(capacity: usize) -> Self {
        Self {
            samples: vec![Default::default(); capacity],
            write_pos: 0,
            read_pos: 0,
        }
    }

    pub fn can_read(&self) -> bool {
        self.read_pos < self.write_pos
    }

    pub fn read_buffer(&self) -> &[u8] {
        bytemuck::cast_slice(&self.samples[self.read_pos..self.write_pos])
    }

    pub fn confirm_read(&mut self, num_bytes: usize) {
        self.read_pos += num_bytes * std::mem::size_of::<Iq>();
        assert!(self.read_pos <= self.write_pos);
    }

    pub fn can_write(&self) -> bool {
        self.write_pos == 0
    }

    pub fn write_buffer(&mut self) -> &mut [Iq] {
        &mut self.samples[self.read_pos..]
    }

    pub fn confirm_write(&mut self, num_samples: usize) {
        self.write_pos += num_samples;
        assert!(self.write_pos <= self.samples.len());
    }
}

impl Default for SampleBuffer {
    fn default() -> Self {
        Self::new(Self::DEFAULT_CAPACITY)
    }
}

async fn handle_connection<H>(
    mut connection: TcpStream,
    shutdown: CancellationToken,
    mut handler: H,
) -> Result<(), Error<H::Error>>
where
    H: ConnectionHandler,
{
    let mut command_buffer = CommandBuffer::default();
    let mut sample_buffer = SampleBuffer::default();

    let (tcp_read, tcp_write) = connection.split();
    let mut tcp_read = BufReader::new(tcp_read);
    let mut tcp_write = BufWriter::new(tcp_write);

    loop {
        tokio::select! {
            _ = shutdown.cancelled() => {
                break;
            }
            result = tcp_read.read_buf(&mut command_buffer) => {
                if result? == 0 {
                    break;
                }
                if let Some(command) = command_buffer.try_decode()? {
                    handler.handle_command(command).await.map_err(Error::Handler)?;
                }
            }
            result = forward_samples(&mut sample_buffer, &mut handler, &mut tcp_write) => {
                result?;
            }
        }
    }

    Ok(())
}

async fn forward_samples<H, W>(
    sample_buffer: &mut SampleBuffer,
    handler: &mut H,
    mut tcp_write: W,
) -> Result<bool, Error<H::Error>>
where
    H: ConnectionHandler,
    W: AsyncWrite + Unpin,
{
    if sample_buffer.can_write() {
        let num_samples = handler
            .read_samples(sample_buffer.write_buffer())
            .await
            .map_err(Error::Handler)?;
        if num_samples == 0 {
            return Ok(true);
        }
        sample_buffer.confirm_write(num_samples);
    }
    else if sample_buffer.can_read() {
        let num_bytes = tcp_write.write(sample_buffer.read_buffer()).await?;
        if num_bytes == 0 {
            return Ok(true);
        }
        sample_buffer.confirm_read(num_bytes);
    }
    else {
        unreachable!(
            "either we should be able to read samples into the buffer or write them out to the stream"
        );
    }

    Ok(false)
}

pub trait Handler {
    type Error: std::error::Error + Send;
    type ConnectionHandler: ConnectionHandler<Error = Self::Error>;

    fn accept_connection(
        &mut self,
        address: SocketAddr,
    ) -> impl Future<Output = Result<Option<Self::ConnectionHandler>, Self::Error>>;
}

pub trait ConnectionHandler: Send + 'static {
    type Error: std::error::Error + Send;

    fn dongle_info(&self) -> DongleInfo;

    fn handle_command(
        &mut self,
        command: Command,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;

    fn read_samples(
        &mut self,
        buffer: &mut [Iq],
    ) -> impl Future<Output = Result<usize, Self::Error>> + Send;
}

#[derive(Clone, Debug)]
pub struct BackendHandler<B> {
    backend: B,
}

impl<B> BackendHandler<B> {
    pub fn new(backend: B) -> Self {
        Self { backend }
    }
}

impl<B> Handler for BackendHandler<B>
where
    B: Backend + Clone + Send + Sync + Unpin + 'static,
{
    type Error = B::Error;
    type ConnectionHandler = BackendConnectionHandler<B>;

    async fn accept_connection(
        &mut self,
        _address: SocketAddr,
    ) -> Result<Option<Self::ConnectionHandler>, Self::Error> {
        Ok(Some(
            BackendConnectionHandler::new(self.backend.clone()).await?,
        ))
    }
}

#[derive(Clone, Debug)]
pub struct BackendConnectionHandler<B> {
    backend: B,
    samples: Samples<Iq>,
    chunk: Option<Chunk<Iq>>,
}

impl<B> BackendConnectionHandler<B>
where
    B: Backend,
{
    pub async fn new(backend: B) -> Result<Self, B::Error> {
        let samples = backend.samples().await?;
        Ok(Self {
            backend,
            samples,
            chunk: None,
        })
    }
}

impl<B> ConnectionHandler for BackendConnectionHandler<B>
where
    B: Backend + Unpin + Send + Sync + 'static,
{
    type Error = B::Error;

    fn dongle_info(&self) -> DongleInfo {
        self.backend.dongle_info()
    }

    async fn handle_command(&mut self, command: Command) -> Result<(), Self::Error> {
        command.apply(&self.backend).await
    }

    async fn read_samples(&mut self, buffer: &mut [Iq]) -> Result<usize, Self::Error> {
        loop {
            if let Some(chunk) = &mut self.chunk {
                let n = buffer.len().min(chunk.len());
                buffer[..n].copy_from_slice(&chunk.samples()[..n]);
                chunk.slice(n..);
                if chunk.is_empty() {
                    self.chunk = None;
                }
                return Ok(n);
            }
            else {
                if let Some(chunk) = self.samples.try_next().await.map_err(|_| todo!())? {
                    self.chunk = Some(chunk);
                }
                else {
                    return Ok(0);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::Iq;

    #[test]
    fn size_of_iq_is_what_we_expect() {
        assert_eq!(std::mem::size_of::<Iq>(), 2);
    }
}
