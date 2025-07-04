#![allow(dead_code)]

//! BEAST input format encoder

use pin_project_lite::pin_project;

use crate::PacketType;

pin_project! {
    #[derive(Debug)]
    pub struct Writer<W> {
        #[pin]
        writer: W,
        write_buffer: WriteBuffer,
    }
}

// this only needs to be able to hold any packet we construct
const WRITE_BUFFER_SIZE: usize = 32;
const PACKET_BUFFER_SIZE: usize = 32;

#[derive(Debug)]
struct WriteBuffer {
    buffer: [u8; WRITE_BUFFER_SIZE],
    write_pos: usize,
    read_pos: usize,
}

impl WriteBuffer {}

#[derive(Debug)]
struct PacketEncoder {
    buffer: [u8; PACKET_BUFFER_SIZE],
    packet_type: Option<InputPacketType>,
}

/// BEAST out packet type
///
/// - [Original doc][1]
/// - [readsb command handling][2]
///
/// [1]: https://wiki.jetvision.de/wiki/Mode-S_Beast:Data_Input_Formats
/// [2]: https://github.com/wiedehopf/readsb/blob/75decb53c0e66f4c12cf24127578a3fe7d919219/net_io.c#L3889
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputPacketType {
    DipSwitches,
    Ping,
    ReadsbConfig, // ???
    Unknown(u8),
}

impl PacketType for InputPacketType {
    type Packet = InputPacket;

    fn from_byte(byte: u8) -> Self {
        match byte {
            b'1' => Self::DipSwitches,
            b'P' => Self::Ping,
            b'W' => Self::ReadsbConfig,
            _ => Self::Unknown(byte),
        }
    }

    fn expected_length(&self) -> Option<usize> {
        match self {
            InputPacketType::DipSwitches => Some(1),
            InputPacketType::Ping => Some(3),
            InputPacketType::ReadsbConfig => Some(1),
            _ => None,
        }
    }

    fn is_known(&self) -> bool {
        match self {
            InputPacketType::Unknown(_) => false,
            _ => true,
        }
    }
}

#[derive(Clone, Debug)]
pub enum InputPacket {
    DipSwitches(ToggleDipswitch),
    Ping([u8; 3]),
    ReadsbConfig(u8),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ToggleDipswitch(pub u8);

impl ToggleDipswitch {
    pub const FORMAT_AVR: Self = Self(b'c');
    pub const FORMAT_BINARY: Self = Self(b'C');
    pub const DF_11_17_ONLY_OFF: Self = Self(b'd');
    pub const DF_11_17_ONLY_ON: Self = Self(b'D');
    pub const TIMESTAMP_INFO_OFF: Self = Self(b'e');
    pub const TIMESTAMP_INFO_ON: Self = Self(b'E');
    pub const CRC_CHECK_ON: Self = Self(b'f');
    pub const CRC_CHECK_OFF: Self = Self(b'F');
    pub const GPS_TIMESTAMP_OFF: Self = Self(b'g');
    pub const GPS_TIMESTAMP_ON: Self = Self(b'G');
    pub const RTS_HANDSHAKE_OFF: Self = Self(b'h');
    pub const RTS_HANDSHAKE_ON: Self = Self(b'H');
    pub const FEC_ON: Self = Self(b'i');
    pub const FEC_OFF: Self = Self(b'I');
    pub const MODE_AC_OFF: Self = Self(b'j');
    pub const MODE_AC_ON: Self = Self(b'J');
}
