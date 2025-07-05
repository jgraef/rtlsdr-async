use bytes::BufMut;
use tokio::{
    io::AsyncWriteExt,
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
    tcp::DongleInfo,
};

#[derive(Debug, thiserror::Error)]
#[error("rtl_tcp server error")]
pub enum Error {
    Io(#[from] std::io::Error),
    Device,
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

impl<S> RtlSdrServer<S>
where
    S: Clone + AsyncReadSamples + Configure + Send + Unpin + 'static,
{
    pub async fn serve(mut self) -> Result<(), Error> {
        //let (command_sender, command_receiver) = mpsc::channel(16);

        //let mut device_task = tokio::spawn(handle_device(self.stream,
        // self.shutdown.clone(), command_receiver));

        tracing::debug!("waiting for connections");

        loop {
            tokio::select! {
                _ = self.shutdown.cancelled() => break,
                /*result = &mut device_task => {
                    return result.unwrap_or(Ok(()));
                }*/
                result = self.tcp_listener.accept() => {
                    let (connection, address) = result?;
                    tracing::debug!(%address, "new connection");
                    //tokio::spawn(handle_client(connection, self.shutdown.clone(), self.stream.clone(), self.dongle_info ));
                    todo!();
                }
            }
        }

        Ok(())
    }
}

async fn handle_client<S>(
    mut connection: TcpStream,
    shutdown: CancellationToken,
    mut stream: S,
    dongle_info: DongleInfo,
) -> Result<(), Error>
where
    S: AsyncReadSamples + Configure + Send + Unpin + 'static,
{
    const BUFFER_SIZE: usize = 16384;
    let mut buffer = vec![0u8; BUFFER_SIZE];

    {
        let mut header_buffer = &mut buffer[..12];
        header_buffer.put(&dongle_info.magic[..]);
        header_buffer.put_u32(dongle_info.tuner_type);
        header_buffer.put_u32(dongle_info.tuner_gain_type);
    }
    connection.write_all(&buffer[..12]).await?;
    connection.flush().await?;

    loop {
        tokio::select! {
            _ = shutdown.cancelled() => {
                break;
            }
            result = stream.read_samples(bytemuck::cast_slice_mut(&mut buffer)) => {
                let samples_read = result
                    .map_err(|_| Error::Device)?;

                if samples_read == 0 {
                    break;
                }
                connection.write_all(&buffer[0..samples_read * 2]).await?;
                connection.flush().await?;
            }
        }
    }

    tracing::debug!("closing connection");
    Ok(())
}

/*async fn handle_device<S: AsyncReadSamples + Configure + Send + Unpin>(stream: S, shutdown: CancellationToken, command_receiver: mpsc::Receiver<Command>) -> Result<(), Error> {
    todo!();
}

async fn handle_client(connection: TcpStream, address: SocketAddr, shutdown :CancellationToken, command_sender: mpsc::Sender<Command>) -> Result<(), Error> {
    todo!();
}

#[derive(Debug)]
enum Command {
    SetCenterFrequency { frequency: u32 },
    SetSampleRate { sample_rate: u32 },
    SetGain { gain: Gain },
    SetAgcMode { enabled: bool },
    Read { length: usize },
}

struct Buffers {
    buffers: VecDeque<Vec<IqSample>>,
    min_read_pos: usize,
    write_pos: usize,
}*/
