use tokio::net::{
    TcpListener,
    TcpStream,
};
use tokio_util::sync::CancellationToken;
use tracing::Instrument;

use crate::RtlSdr;

#[derive(Debug, thiserror::Error)]
#[error("sdr++ server error")]
pub enum Error {
    Io(#[from] std::io::Error),
    RtlSdr(#[from] crate::Error),
}

#[derive(Debug)]
pub struct SdrppServer {
    rtlsdr: RtlSdr,
    tcp_listener: TcpListener,
    shutdown: CancellationToken,
}

impl SdrppServer {
    pub fn new(rtlsdr: RtlSdr, tcp_listener: TcpListener) -> Self {
        Self {
            rtlsdr,
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

    /// Serve incoming connections
    pub async fn serve(self) -> Result<(), Error> {
        tracing::debug!("waiting for connections");

        loop {
            tokio::select! {
                _ = self.shutdown.cancelled() => break,
                result = self.tcp_listener.accept() => {
                    let (connection, address) = result?;
                    let shutdown = self.shutdown.clone();
                    let rtlsdr = self.rtlsdr.clone();
                    let span = tracing::info_span!("connection", %address);
                    tokio::spawn(
                        async move {
                            tracing::debug!(%address, "new connection");
                            if let Err(error) = handle_client(connection, shutdown, rtlsdr ).await {
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

async fn handle_client(
    tcp: TcpStream,
    shutdown: CancellationToken,
    rtlsdr: RtlSdr,
) -> Result<(), Error> {
    todo!();
}
