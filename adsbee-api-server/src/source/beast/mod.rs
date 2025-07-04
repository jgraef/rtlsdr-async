//! BEAST format
//!
//! The BEAST format is a stream protocol to transmit ADS-B related data frames
//! as they're captured by a receiver. It's an escaped format, meaning frames
//! start with an escape byte, and any occurances of the escape byte in the
//! payload is escaped as two escape bytes. This allows for finding the start of
//! a packet and skipping malformed/unrecognized packets.
//!
//! The original format transports Mode A/C, Mode S (short and long) and
//! configuration data. But the readsb program extends the protocol to transport
//! various other bits of information, such as receiver ID.
//!
//! The protocol is actually bi-directional, but it's mostly used to receive
//! data from a BEAST module (or readsb) - this is BEAST output. The other
//! direction, BEAST input is only used to set configuration.
//!
//! - [Original documentation][1]
//! - [wiedehopf/readsb encoding][2]
//! - [wiedehopf/readsb decoding][3]
//! - [HULC extension][4]
//!
//! # TODO
//!
//! - make this a separate crate
//! - make reader/writer work with both futures and tokio AsyncRead/AsyncWrite
//! - make reader/writer work both as Stream/Sink and with plain methods
//!
//! [1]: https://wiki.jetvision.de/wiki/Mode-S_Beast:Data_Output_Formats
//! [2]: https://github.com/wiedehopf/readsb/blob/75decb53c0e66f4c12cf24127578a3fe7d919219/net_io.c#L1754
//! [3]: https://github.com/wiedehopf/readsb/blob/75decb53c0e66f4c12cf24127578a3fe7d919219/net_io.c#L4885
//! [4]: https://static.avionix-tech.com/statics/cms/2023-11-21/GNS5894T_ADSB_Module_datasheet_V1.1.pdf

use bytes::{
    Buf,
    BufMut,
};

pub mod input;
pub mod output;

/// the "escape" byte.
const ESCAPE: u8 = 0x1a;

#[derive(Debug, thiserror::Error)]
#[error("beast error")]
pub enum Error {
    Io(#[from] std::io::Error),
}

/// Timestamp used for multilateration.
///
/// # TODO
///
/// decode this into a DateTime? it's big-endian
/// <https://github.com/wiedehopf/readsb/blob/75decb53c0e66f4c12cf24127578a3fe7d919219/net_io.c#L1701>
///
/// some values have special meaning:
/// <https://github.com/wiedehopf/readsb/blob/75decb53c0e66f4c12cf24127578a3fe7d919219/readsb.h#L341>
///
/// The [original BEAST documentation][1] also references "Legacy 12MHz" and
/// "GPS" timestamps. This should be bit 4 in
/// [`OutputPacket::DipSwitches`][output::OutputPacket::DipSwitches].
///
/// ## HULC
///
/// [HULC datasheet][2]
///
/// > Upper 18 bits are seconds since last midnight 00:00:00 UTC
/// > Lower 30 bits are nanoseconds of current second
///
/// > There are two distinct timestamp formats, absolute and relative timestamps
/// > depending on whether a GPS receiver is attached or not. To find out which
/// > format is used check bit 15 (Gps Detected) in the status flag field of the
/// > HULC Status Message. Both timestamp formats are compatible with the
/// > respective timestamp formats used in the Beast BinaryProtocol.
///
/// [1]: https://wiki.jetvision.de/wiki/Mode-S_Beast:Data_Input_Formats
/// [2]: https://static.avionix-tech.com/statics/cms/2023-11-21/GNS5894T_ADSB_Module_datasheet_V1.1.pdf
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MlatTimestamp(pub [u8; 6]);

impl MlatTimestamp {
    /// A timestamp that indicates the data is synthetic, created from a
    /// multilateration result.
    pub const SYNTHETIC_MLAT: Self = Self(*b"\xFF\x00\x4D\x4C\x41\x54");

    /// Timestamp used for synthetic messages from UAT.
    pub const SYNTHETIC_UAT: Self = Self(*b"\xFF\x00\x4D\x4C\x41\x55");
    pub const NO_FORWARD: Self = Self(*b"\xFF\x00\x4D\x4C\x41\x60");

    /// Removed timestamp
    ///
    /// > clobber timestamp for better compression
    /// <https://github.com/wiedehopf/readsb/blob/75decb53c0e66f4c12cf24127578a3fe7d919219/net_io.c#L1840>
    pub const ANY_TIMESTAMP: Self = Self([0xff; 6]);

    pub fn is_synthetic(&self) -> bool {
        &self.0[0..5] == b"\xFF\x00\x4D\x4C\x41" || self == &Self::ANY_TIMESTAMP
    }
}

/// RSSI encoded as one byte.
///
/// # References
///
/// <https://github.com/wiedehopf/readsb/blob/75decb53c0e66f4c12cf24127578a3fe7d919219/readsb.h#L970>
/// <https://github.com/wiedehopf/readsb/blob/75decb53c0e66f4c12cf24127578a3fe7d919219/net_io.c#L1777>
/// <https://static.avionix-tech.com/statics/cms/2023-11-21/GNS5894T_ADSB_Module_datasheet_V1.1.pdf>
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SignalLevel(pub u8);

impl SignalLevel {
    /// Decode signal level information.
    ///
    /// This is how readsb encodes it:
    ///
    /// ```c
    /// double signalLevel; // RSSI, in the range [0..1], as a fraction of full-scale power
    /// ```
    /// <https://github.com/wiedehopf/readsb/blob/75decb53c0e66f4c12cf24127578a3fe7d919219/readsb.h#L970>
    ///
    /// ```c
    /// sig = nearbyint(sqrt(mm->signalLevel) * 255);
    /// ```
    /// <https://github.com/wiedehopf/readsb/blob/75decb53c0e66f4c12cf24127578a3fe7d919219/net_io.c#L1777>
    pub fn decode(&self) -> f32 {
        (f32::from(self.0) / 255.0).clamp(0.0, 1.0).powi(2)
    }
}

pub trait PacketType {
    type Packet;

    fn from_byte(byte: u8) -> Self;
    fn expected_length(&self) -> Option<usize>;
    fn is_known(&self) -> bool;
}

pub trait PacketDecode: PacketType {
    fn decode<B: Buf>(&self, buffer: &mut B) -> Option<Self::Packet>;
}

pub trait PacketEncode: PacketType {
    fn encode<B: BufMut>(&self, packet: &Self::Packet, buffer: &mut B);
}
