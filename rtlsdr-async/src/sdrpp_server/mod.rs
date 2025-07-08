use std::fmt::Debug;

use bytemuck::{
    Pod,
    Zeroable,
};
use tokio::{
    io::{
        AsyncRead,
        AsyncReadExt,
        BufReader,
        BufStream,
        BufWriter,
    },
    net::{
        TcpListener,
        TcpStream,
    },
};
use tokio_util::sync::CancellationToken;
use tracing::Instrument;

//use crate::RtlSdr;
type RtlSdr = ();

#[derive(Debug, thiserror::Error)]
#[error("sdr++ server error")]
pub enum Error {
    Io(#[from] std::io::Error),
    RtlSdr(#[from] crate::Error),
    UnknownPacketType {
        packet_type: PacketType,
    },
    UnknownClientCommand {
        command_type: CommandType,
    },
    InvalidCommandSize {
        command_type: CommandType,
        command_size: usize,
    },
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

const SDRPP_STREAM_BUFFER_SIZE: usize = 1000000;

#[derive(Clone, Copy, Debug, Pod, Zeroable)]
#[repr(C)]
struct Sample {
    pub i: f32,
    pub q: f32,
}

impl From<crate::IqSample> for Sample {
    fn from(value: crate::IqSample) -> Self {
        #[inline(always)]
        fn u8_to_f32(x: u8) -> f32 {
            ((x as f32) - 128.0) / 128.0
        }

        Sample {
            i: u8_to_f32(value.i),
            q: u8_to_f32(value.q),
        }
    }
}

const SDRPP_MAX_PACKET_SIZE: usize = SDRPP_STREAM_BUFFER_SIZE * size_of::<Sample>() * 2;

const RECEIVE_BUFFER_SIZE: usize = SDRPP_MAX_PACKET_SIZE;
const SEND_BUFFER_SIZE: usize = SDRPP_MAX_PACKET_SIZE;

async fn handle_client(
    mut tcp: TcpStream,
    shutdown: CancellationToken,
    rtlsdr: RtlSdr,
) -> Result<(), Error> {
    let (tcp_receiver, tcp_sender) = tcp.split();
    let mut tcp_receiver = BufReader::with_capacity(RECEIVE_BUFFER_SIZE, tcp_receiver);
    let tcp_sender = BufWriter::with_capacity(SEND_BUFFER_SIZE, tcp_sender);

    loop {
        let packet_type = PacketType(tcp_receiver.read_u32_le().await?);

        // packet size includes all headers
        let packet_size = tcp_receiver.read_u32_le().await?;

        tracing::debug!(?packet_type, ?packet_size);

        let packet_size = usize::try_from(packet_size).unwrap() - 8;

        match packet_type {
            PacketType::COMMAND => {
                let command_type = CommandType(tcp_receiver.read_u32_le().await?);
                let command_size = packet_size - 4;

                match command_type {
                    // todo
                    CommandType::GET_UI => {
                        if command_size == 0 {
                            todo!();
                        }
                        else {
                            return Err(Error::InvalidCommandSize {
                                command_type,
                                command_size,
                            });
                        }
                    }
                    _ => {
                        return Err(Error::UnknownClientCommand { command_type });
                    }
                }
            }
            PacketType::COMMAND_ACK => {}
            PacketType::BASEBAND => {}
            PacketType::BASEBAND_COMPRESSED => {}
            PacketType::VFO => {}
            PacketType::FFT => {}
            PacketType::ERROR => {}
            _ => {
                return Err(Error::UnknownPacketType {
                    packet_type,
                });
            }
        }

        break; // todo
    }

    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Pod, Zeroable)]
#[repr(transparent)]
pub struct PacketType(pub u32);

impl PacketType {
    pub const COMMAND: Self = Self(0);
    pub const COMMAND_ACK: Self = Self(1);
    pub const BASEBAND: Self = Self(2);
    pub const BASEBAND_COMPRESSED: Self = Self(3);
    pub const VFO: Self = Self(4);
    pub const FFT: Self = Self(5);
    pub const ERROR: Self = Self(6);
}

impl Debug for PacketType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            Self::COMMAND => write!(f, "PacketType::COMMAND"),
            Self::COMMAND_ACK => write!(f, "PacketType::COMMAND_ACK"),
            Self::BASEBAND => write!(f, "PacketType::BASEBAND"),
            Self::BASEBAND_COMPRESSED => write!(f, "PacketType::BASEBAND_COMPRESSED"),
            Self::VFO => write!(f, "PacketType::VFO"),
            Self::FFT => write!(f, "PacketType::FFT"),
            Self::ERROR => write!(f, "PacketType::ERROR"),
            _ => write!(f, "PacketType({})", self.0),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Pod, Zeroable)]
#[repr(transparent)]
pub struct CommandType(u32);

impl CommandType {
    pub const GET_UI: Self = Self(0x00);
    pub const UI_ACTION: Self = Self(0x01);
    pub const STARRT: Self = Self(0x02);
    pub const STOP: Self = Self(0x03);
    pub const SET_FREQUENCY: Self = Self(0x04);
    pub const GET_SAMPLE_RATE: Self = Self(0x05);
    pub const SAMPLE_TYPE: Self = Self(0x06);
    pub const COMPRESSION: Self = Self(0x07);
    pub const SET_SAMPLE_RATE: Self = Self(0x80);
    pub const DISCONNECT: Self = Self(0x81);
}

impl Debug for CommandType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            Self::GET_UI => write!(f, "CommandType::GET_UI"),
            Self::UI_ACTION => write!(f, "CommandType::UI_ACTION"),
            Self::STARRT => write!(f, "CommandType::STARRT"),
            Self::STOP => write!(f, "CommandType::STOP"),
            Self::SET_FREQUENCY => write!(f, "CommandType::SET_FREQUENCY"),
            Self::GET_SAMPLE_RATE => write!(f, "CommandType::GET_SAMPLE_RATE"),
            Self::SAMPLE_TYPE => write!(f, "CommandType::SAMPLE_TYPE"),
            Self::COMPRESSION => write!(f, "CommandType::COMPRESSION"),
            Self::SET_SAMPLE_RATE => write!(f, "CommandType::SET_SAMPLE_RATE"),
            Self::DISCONNECT => write!(f, "CommandType::DISCONNECT"),
            _ => write!(f, "CommandType({}))", self.0),
        }
    }
}