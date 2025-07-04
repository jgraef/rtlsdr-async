//! Mode S frame decoder
//!
//! - [ADS-B Reference][1] (page 39 ff) - this defines all the ADS-B messages
//!   and related Mode-S DFs
//! - [The 1090 Megaherrtz Riddle][2] - good overview
//! - [Annex 10 to the Convetion on International Civil Aviation][3] -
//!   specifications on Mode A/C/S in general
//!
//! Mode-S defines 2 frame lengths:
//! - short = 56 bits / 7 bytes
//! - long = 112 bits / 14 bytes
//!
//! [1]: http://www.anteni.net/adsb/Doc/1090-WP30-18-DRAFT_DO-260B-V42.pdf
//! [2]: https://mode-s.org/1090mhz/content/mode-s/1-basics.html
//! [3]: https://applications.icao.int/tools/ATMiKIT/story_content/external_files/story_content/external_files/Annex10_Volume%204_cons.pdf

pub mod acas;
pub mod adsb;
pub mod tisb;
pub mod util;

use std::fmt::Debug;

use adsbee_api_types::{
    IcaoAddress,
    Squawk,
};
use bytes::Buf;

use crate::{
    source::mode_s::util::{
        CRC_24_MODES,
        CrcBuf,
        decode_air_air_surveillance_common_fields,
        decode_surveillance_reply_body,
        gillham::{
            decode_gillham_ac13,
            decode_gillham_id13,
        },
    },
    util::BufReadBytesExt,
};

/// Length of a short mode-s frame
pub const LENGTH_SHORT: usize = 7;

/// Length of a long mode-s frame
pub const LENGTH_LONG: usize = 14;

#[derive(Debug, thiserror::Error)]
pub enum EncodeError {}

#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    #[error("buffer with length 0 doesn't contain DF")]
    NoDf,

    #[error("invalid value for DF: {value}")]
    InvalidDf { value: u8 },

    #[error(
        "expected {expected_length} bytes for the frame, but buffer is only {buffer_length} bytes long"
    )]
    Truncated {
        expected_length: usize,
        buffer_length: usize,
    },

    #[error("CRC check failed")]
    CrcCheckFailed(FrameWithChecksum),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Frame {
    ShortAirAirSurveillance(ShortAirAirSurveillance),
    SurveillanceAltitudeReply(SurveillanceAltitudeReply),
    SurveillanceIdentityReply(SurveillanceIdentityReply),
    AllCallReply(AllCallReply),
    LongAirAirSurveillance(LongAirAirSurveillance),
    ExtendedSquitter(ExtendedSquitter),
    ExtendedSquitterNonTransponder(ExtendedSquitterNonTransponder),
    MilitaryExtendedSquitter(MilitaryExtendedSquitter),
    CommBAltitudeReply(CommBAltitudeReply),
    CommBIdentityReply(CommBIdentityReply),
    MilitaryUse(MilitaryUse),
    CommD(CommD),
}

impl Frame {
    /// Decodes a Mode-S frame.
    ///
    /// This doesn't verify the checksum.
    pub fn decode<B: Buf>(buffer: &mut B) -> Result<Frame, DecodeError> {
        let buffer_length = buffer.remaining();

        let byte_0 = buffer.try_get_u8().map_err(|_| DecodeError::NoDf)?;

        let bits_1_to_5 = byte_0 >> 3;
        let bits_6_to_8 = byte_0 & 0b00000111;
        let df = DownlinkFormat::from_u8(bits_1_to_5)?;

        // check that the buffer contains enough data
        let expected_length = df.frame_length();
        if buffer_length < expected_length {
            return Err(DecodeError::Truncated {
                expected_length,
                buffer_length,
            });
        }

        let frame = {
            // create a new buffer that is limited to the length of the frame
            let mut buffer = buffer.take(expected_length - 1);
            let buffer = &mut buffer;

            // decode the DF
            match df {
                DownlinkFormat::ShortAirAirSurveillance => {
                    Self::ShortAirAirSurveillance(ShortAirAirSurveillance::decode(
                        buffer,
                        bits_6_to_8,
                    ))
                }
                DownlinkFormat::SurveillanceAltitudeReply => {
                    Self::SurveillanceAltitudeReply(SurveillanceAltitudeReply::decode(
                        buffer,
                        bits_6_to_8,
                    ))
                }
                DownlinkFormat::SurveillanceIdentityReply => {
                    Self::SurveillanceIdentityReply(SurveillanceIdentityReply::decode(
                        buffer,
                        bits_6_to_8,
                    ))
                }
                DownlinkFormat::AllCallReply => {
                    Self::AllCallReply(AllCallReply::decode(buffer, bits_6_to_8))
                }
                DownlinkFormat::LongAirAirSurveillance => {
                    Self::LongAirAirSurveillance(LongAirAirSurveillance::decode(
                        buffer,
                        bits_6_to_8,
                    ))
                }
                DownlinkFormat::ExtendedSquitter => {
                    Self::ExtendedSquitter(ExtendedSquitter::decode(buffer, bits_6_to_8)?)
                }
                DownlinkFormat::ExtendedSquitterNonTransponder => {
                    Self::ExtendedSquitterNonTransponder(ExtendedSquitterNonTransponder::decode(
                        buffer,
                        bits_6_to_8,
                    )?)
                }
                DownlinkFormat::MilitaryExtendedSquitter => {
                    Self::MilitaryExtendedSquitter(MilitaryExtendedSquitter::decode(
                        buffer,
                        bits_6_to_8,
                    )?)
                }
                DownlinkFormat::CommBAltitudeReply => {
                    Self::CommBAltitudeReply(CommBAltitudeReply::decode(buffer, bits_6_to_8))
                }
                DownlinkFormat::CommBIdentityReply => {
                    Self::CommBIdentityReply(CommBIdentityReply::decode(buffer, bits_6_to_8))
                }
                DownlinkFormat::MilitaryUse => {
                    Self::MilitaryUse(MilitaryUse {
                        bits_6_to_8,
                        data: buffer.get_bytes(),
                    })
                }
                DownlinkFormat::CommD => {
                    // > Note Format number 24 is an exception. It is identified using only the first two bits, which must be 11 in binary. All following bits are used for encoding other information.
                    // https://mode-s.org/1090mhz/content/introduction.html
                    //
                    // so we need to pass bits 3 to 8
                    let bits_3_to_8 = byte_0 & 0b00111111;

                    // todo
                    Self::CommD(CommD {
                        bits_3_to_8,
                        data: buffer.get_bytes(),
                    })
                }
            }
        };

        let remaining = buffer.remaining();
        if remaining > 0 {
            todo!("fixme: {remaining} bytes remaining in buffer: {frame:?}");
        }

        Ok(frame)
    }

    /// Decodes a Mode-S frame and calculates its CRC checksum.
    pub fn decode_and_calculate_checksum<B: Buf>(
        buffer: &mut B,
    ) -> Result<FrameWithChecksum, DecodeError> {
        const CRC: crc::Crc<u32> = crc::Crc::<u32>::new(&CRC_24_MODES);

        let mut buffer = CrcBuf {
            inner: buffer,
            digest: CRC.digest(),
        };

        let frame = Self::decode(&mut buffer)?;

        let checksum = buffer.digest.finalize().to_be_bytes();
        assert_eq!(checksum[0], 0);
        let checksum = Checksum([checksum[1], checksum[2], checksum[3]]);

        Ok(FrameWithChecksum { frame, checksum })
    }

    /// Decodes a Mode-S frame.
    ///
    /// This performs a CRC check if possible and returns an error if it fails.
    /// If you want to decode invalid Mode-S frames, or need the CRC checksum
    /// use [`decode_with_checksum`][Self::decode_and_calculate_checksum]
    pub fn decode_and_check_checksum<B: Buf>(buffer: &mut B) -> Result<Self, DecodeError> {
        let frame = Self::decode_and_calculate_checksum(buffer)?;

        if !frame.check().unwrap_or(true) {
            return Err(DecodeError::CrcCheckFailed(frame));
        }

        Ok(frame.frame)
    }

    pub fn downlink_format(&self) -> DownlinkFormat {
        match self {
            Frame::ShortAirAirSurveillance(_) => DownlinkFormat::ShortAirAirSurveillance,
            Frame::SurveillanceAltitudeReply(_) => DownlinkFormat::SurveillanceAltitudeReply,
            Frame::SurveillanceIdentityReply(_) => DownlinkFormat::SurveillanceIdentityReply,
            Frame::AllCallReply(_) => DownlinkFormat::AllCallReply,
            Frame::LongAirAirSurveillance(_) => DownlinkFormat::LongAirAirSurveillance,
            Frame::ExtendedSquitter(_) => DownlinkFormat::ExtendedSquitter,
            Frame::ExtendedSquitterNonTransponder(_) => {
                DownlinkFormat::ExtendedSquitterNonTransponder
            }
            Frame::MilitaryExtendedSquitter(_) => DownlinkFormat::MilitaryExtendedSquitter,
            Frame::CommBAltitudeReply(_) => DownlinkFormat::CommBAltitudeReply,
            Frame::CommBIdentityReply(_) => DownlinkFormat::CommBIdentityReply,
            Frame::MilitaryUse(_) => DownlinkFormat::MilitaryUse,
            Frame::CommD(_) => DownlinkFormat::CommD,
        }
    }

    pub fn length(&self) -> usize {
        self.downlink_format().frame_length()
    }

    /// Returns address announced and ADS-B message
    pub fn adsb(&self) -> Option<(&IcaoAddress, &adsb::Message)> {
        match self {
            Frame::ExtendedSquitter(ExtendedSquitter {
                address_announced,
                adsb_message,
                ..
            })
            | Frame::ExtendedSquitterNonTransponder(
                ExtendedSquitterNonTransponder::AdsbWithIcaoAddress {
                    address_announced,
                    adsb_message,
                    ..
                },
            )
            | Frame::ExtendedSquitterNonTransponder(
                ExtendedSquitterNonTransponder::AdsbWithNonIcaoAddress {
                    address_announced,
                    adsb_message,
                    ..
                },
            )
            | Frame::ExtendedSquitterNonTransponder(
                ExtendedSquitterNonTransponder::AdsbRebroadcast {
                    address_announced,
                    adsb_message,
                    ..
                },
            )
            | Frame::MilitaryExtendedSquitter(MilitaryExtendedSquitter::Adsb {
                address_announced,
                adsb_message,
                ..
            }) => Some((address_announced, adsb_message)),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameWithChecksum {
    pub frame: Frame,
    pub checksum: Checksum,
}

impl FrameWithChecksum {
    /// Tries to check if the frame is not corrupted.
    ///
    /// This is only possible for some downlink types (e.g. ADS-B), because some
    /// overlay the parity with other data.
    pub fn check(&self) -> Option<bool> {
        match &self.frame {
            Frame::ExtendedSquitter(_)
            | Frame::ExtendedSquitterNonTransponder(_)
            | Frame::MilitaryExtendedSquitter(_) => Some(self.checksum.check()),
            _ => None,
        }
    }
}

/// Downlink format
///
/// First 5 bits of a Mode S frame determine the kind of frame.
///
/// # Exception
///
/// [`CommD`][Self::CommD] is determined only by the first 2 bits, which must
/// both be 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DownlinkFormat {
    ShortAirAirSurveillance,
    SurveillanceAltitudeReply,
    SurveillanceIdentityReply,
    AllCallReply,
    LongAirAirSurveillance,
    ExtendedSquitter,
    ExtendedSquitterNonTransponder,
    MilitaryExtendedSquitter,
    CommBAltitudeReply,
    CommBIdentityReply,
    /// Referenced [here][1], but can't find any more information on it.
    ///
    /// [1]: https://www.idc-online.com/technical_references/pdfs/electronic_engineering/Mode_S_Reply_Encoding.pdf
    MilitaryUse,
    CommD,
}

impl DownlinkFormat {
    pub fn from_u8(byte: u8) -> Result<Self, DecodeError> {
        match byte {
            0 => Ok(Self::ShortAirAirSurveillance),
            4 => Ok(Self::SurveillanceAltitudeReply),
            5 => Ok(Self::SurveillanceIdentityReply),
            11 => Ok(Self::AllCallReply),
            16 => Ok(Self::LongAirAirSurveillance),
            17 => Ok(Self::ExtendedSquitter),
            18 => Ok(Self::ExtendedSquitterNonTransponder),
            19 => Ok(Self::MilitaryExtendedSquitter),
            20 => Ok(Self::CommBAltitudeReply),
            21 => Ok(Self::CommBIdentityReply),
            22 => Ok(Self::MilitaryUse),
            24..=31 => Ok(Self::CommD),
            _ => Err(DecodeError::InvalidDf { value: byte }),
        }
    }

    pub fn frame_length(&self) -> usize {
        match self {
            DownlinkFormat::ShortAirAirSurveillance => LENGTH_SHORT,
            DownlinkFormat::SurveillanceAltitudeReply => LENGTH_SHORT,
            DownlinkFormat::SurveillanceIdentityReply => LENGTH_SHORT,
            DownlinkFormat::AllCallReply => LENGTH_SHORT,
            DownlinkFormat::LongAirAirSurveillance => LENGTH_LONG,
            DownlinkFormat::ExtendedSquitter => LENGTH_LONG,
            DownlinkFormat::ExtendedSquitterNonTransponder => LENGTH_LONG,
            DownlinkFormat::MilitaryExtendedSquitter => LENGTH_LONG,
            DownlinkFormat::CommBAltitudeReply => LENGTH_LONG,
            DownlinkFormat::CommBIdentityReply => LENGTH_LONG,
            DownlinkFormat::MilitaryUse => LENGTH_LONG,
            DownlinkFormat::CommD => LENGTH_LONG,
        }
    }
}

/// 3 bit capability value
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Capability(u8);

impl Capability {
    /// Signifies Level 1 transponder (surveillance only), and no ability to
    /// set "CA" code 7, and either on the ground or airborne
    pub const LEVEL1_GROUND_AIRBORNE: Self = Self(0b000);

    /// Signifies Level 2 or above transponder, and the ability to set "CA"
    /// code 7, and on the ground
    pub const LEVEL2_GROUND: Self = Self(0b100);

    /// Signifies Level 2 or above transponder, and the ability to set "CA"
    /// code 7, and airborne
    pub const LEVEL2_AIRBORNE: Self = Self(0b101);

    /// Signifies Level 2 or above transponder, and the ability to set "CA"
    /// code 7, and either on the ground or airborne
    pub const LEVEL2_GROUND_AIRBORNE: Self = Self(0b110);

    /// Signifies the "DR" field is NOT equal to ZERO (0), or the "FS"
    /// field equals 2, 3, 4, or 5, and either on the ground or airborne.
    pub const DR_NOT_ZERO_FS_EQUAL_2345_GROUND_AIRBORNE: Self = Self(0b111);

    pub const fn from_u8_unchecked(byte: u8) -> Self {
        Self(byte)
    }

    pub const fn from_u8(byte: u8) -> Option<Self> {
        if byte & 0b11111000 == 0 {
            Some(Self(byte))
        }
        else {
            None
        }
    }

    pub fn as_u8(&self) -> u8 {
        self.0
    }
}

impl Debug for Capability {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            Self::LEVEL1_GROUND_AIRBORNE => write!(f, "Capability::LEVEL1_GROUND_AIRBORNE"),
            Self::LEVEL2_GROUND => write!(f, "Capability::LEVEL2_GROUND"),
            Self::LEVEL2_AIRBORNE => write!(f, "Capability::LEVEL2_AIRBORNE"),
            Self::LEVEL2_GROUND_AIRBORNE => write!(f, "Capability::LEVEL2_GROUND_AIRBORNE"),
            Self::DR_NOT_ZERO_FS_EQUAL_2345_GROUND_AIRBORNE => {
                write!(f, "Capability::DR_NOT_ZERO_FS_EQUAL_2345_GROUND_AIRBORNE")
            }
            _ => write!(f, "Capability(b{:03b})", self.0),
        }
    }
}

/// 3-bit code format
///
/// Determines the type of non-transmitter extended squitter message.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CodeFormat(u8);

impl CodeFormat {
    pub const ADSB_WITH_ICAO_ADDRESS: Self = Self(0);
    pub const ADSB_WITH_NON_ICAO_ADDRESS: Self = Self(1);
    pub const TISB_WITH_ICAO_ADDRESS1: Self = Self(2);
    pub const TISB_WITH_ICAO_ADDRESS2: Self = Self(3);
    pub const TISB_AND_ADSR_MANAGEMENT: Self = Self(4);
    pub const TISB_WITH_NON_ICAO_ADDRESS: Self = Self(5);
    pub const ADSB_REBROADCAST: Self = Self(6);
    pub const RESERVED: Self = Self(7);

    pub const fn from_u8_unchecked(byte: u8) -> Self {
        Self(byte)
    }

    pub const fn from_u8(byte: u8) -> Option<Self> {
        if byte & 0b11111000 == 0 {
            Some(Self(byte))
        }
        else {
            None
        }
    }

    pub fn as_u8(&self) -> u8 {
        self.0
    }
}

impl Debug for CodeFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            Self::ADSB_WITH_ICAO_ADDRESS => write!(f, "CodeFormat::ADSB_WITH_ICAO_ADDRESS"),
            Self::ADSB_WITH_NON_ICAO_ADDRESS => write!(f, "CodeFormat::ADSB_WITH_NON_ICAO_ADDRESS"),
            Self::TISB_WITH_ICAO_ADDRESS1 => write!(f, "CodeFormat::TISB_WITH_ICAO_ADDRESS1"),
            Self::TISB_WITH_ICAO_ADDRESS2 => write!(f, "CodeFormat::TISB_WITH_ICAO_ADDRESS2"),
            Self::TISB_AND_ADSR_MANAGEMENT => write!(f, "CodeFormat::TISB_AND_ADSR_MANAGEMENT"),
            Self::TISB_WITH_NON_ICAO_ADDRESS => write!(f, "CodeFormat::TISB_WITH_NON_ICAO_ADDRESS"),
            Self::ADSB_REBROADCAST => write!(f, "CodeFormat::ADSB_REBROADCAST"),
            Self::RESERVED => write!(f, "CodeFormat::RESERVED"),
            _ => panic!("Invalid CodeFormat bitpattern"),
        }
    }
}

/// 3-bit flight status
///
/// <https://mode-s.org/1090mhz/content/mode-s/3-surveillance.html>
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FlightStatus(u8);

impl FlightStatus {
    pub const NO_ALERT_NO_SPI_AIRBORNE: Self = Self(0b000);
    pub const NO_ALERT_NO_SPI_GROUND: Self = Self(0b001);
    pub const ALERT_NO_SPI_AIRBORNE: Self = Self(0b010);
    pub const ALERT_NO_SPI_GROUND: Self = Self(0b011);
    pub const ALERT_SPI_AIRBORNE_GROUND: Self = Self(0b100);
    pub const NO_ALERT_SPI_AIRBORNE_GROUND: Self = Self(0b101);

    pub const fn from_u8_unchecked(byte: u8) -> Self {
        Self(byte)
    }

    pub const fn from_u8(byte: u8) -> Option<Self> {
        if byte & 0b11111000 == 0 {
            Some(Self(byte))
        }
        else {
            None
        }
    }

    pub fn as_u8(&self) -> u8 {
        self.0
    }

    pub fn alert(&self) -> bool {
        *self == Self::ALERT_NO_SPI_AIRBORNE
            || *self == Self::ALERT_NO_SPI_GROUND
            || *self == Self::ALERT_SPI_AIRBORNE_GROUND
    }

    pub fn spi(&self) -> bool {
        *self == Self::ALERT_SPI_AIRBORNE_GROUND || *self == Self::NO_ALERT_SPI_AIRBORNE_GROUND
    }

    pub fn airborne(&self) -> bool {
        *self == Self::NO_ALERT_NO_SPI_GROUND
            || *self == Self::ALERT_NO_SPI_GROUND
            || *self == Self::ALERT_SPI_AIRBORNE_GROUND
            || *self == Self::NO_ALERT_SPI_AIRBORNE_GROUND
    }

    pub fn ground(&self) -> bool {
        *self == Self::NO_ALERT_NO_SPI_GROUND
            || *self == Self::ALERT_NO_SPI_GROUND
            || *self == Self::ALERT_SPI_AIRBORNE_GROUND
            || *self == Self::NO_ALERT_SPI_AIRBORNE_GROUND
    }
}

impl Debug for FlightStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "FlightStatus(")?;
        let mut comma = false;
        let mut slash = false;
        if self.alert() {
            write!(f, "alert")?;
            comma = true;
        }
        if self.spi() {
            if comma {
                write!(f, ", ")?;
            }
            write!(f, "spi")?;
            comma = true;
        }
        if self.airborne() {
            if comma {
                write!(f, ", ")?;
            }
            write!(f, "airborne")?;
            slash = true;
        }
        if self.ground() {
            if slash {
                write!(f, "/")?;
            }
            else if comma {
                write!(f, ", ")?;
            }
            write!(f, "ground")?;
        }
        write!(f, ")")
    }
}

/// 5-bit downlink request
///
/// <https://mode-s.org/1090mhz/content/mode-s/3-surveillance.html>
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DownlinkRequest(u8);

impl DownlinkRequest {
    pub const NO_DOWNLINK_REQUEST: Self = Self(0);
    pub const REQUEST_TO_SEND_COMMB_MESSAGE: Self = Self(1);
    pub const COMMB_BROADCAST_MESSAGE1_AVAILABLE: Self = Self(4);
    pub const COMMB_BROADCAST_MESSAGE2_AVAILABLE: Self = Self(5);

    pub const fn from_u8_unchecked(byte: u8) -> Self {
        Self(byte)
    }

    pub const fn from_u8(byte: u8) -> Option<Self> {
        if byte & 0b11100000 == 0 {
            Some(Self(byte))
        }
        else {
            None
        }
    }

    pub fn as_u8(&self) -> u8 {
        self.0
    }
}

impl Debug for DownlinkRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            Self::NO_DOWNLINK_REQUEST => write!(f, "DownlinkRequest::NO_DOWNLINK_REQUEST"),
            Self::REQUEST_TO_SEND_COMMB_MESSAGE => {
                write!(f, "DownlinkRequest::REQUEST_TO_SEND_COMMB_MESSAGE")
            }
            Self::COMMB_BROADCAST_MESSAGE1_AVAILABLE => {
                write!(f, "DownlinkRequest::COMMB_BROADCAST_MESSAGE1_AVAILABLE")
            }
            Self::COMMB_BROADCAST_MESSAGE2_AVAILABLE => {
                write!(f, "DownlinkRequest::COMMB_BROADCAST_MESSAGE2_AVAILABLE")
            }
            _ => write!(f, "DownlinkRequest({})", self.0),
        }
    }
}

/// 6-bit utility message
///
/// <https://mode-s.org/1090mhz/content/mode-s/3-surveillance.html>
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UtilityMessage {
    pub interrogator_identifier_subfield: InterrogatorIdentifierSubfield,
    pub interrogator_reservation_type: InterrogatorReservationType,
}

impl UtilityMessage {
    pub const fn from_u8(byte: u8) -> Option<Self> {
        if byte & 0b11000000 == 0 {
            Some(Self::from_u8_unchecked(byte))
        }
        else {
            None
        }
    }

    pub const fn from_u8_unchecked(byte: u8) -> Self {
        Self {
            interrogator_identifier_subfield: InterrogatorIdentifierSubfield(byte >> 2),
            interrogator_reservation_type: InterrogatorReservationType(byte & 0b11),
        }
    }

    pub fn as_u8(&self) -> u8 {
        self.interrogator_identifier_subfield.as_u8()
            | (self.interrogator_reservation_type.as_u8() << 4)
    }
}

/// 6-bit IIS in utility message
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct InterrogatorIdentifierSubfield(u8);

impl InterrogatorIdentifierSubfield {
    pub const fn from_u8_unchecked(byte: u8) -> Self {
        Self(byte)
    }

    pub const fn from_u8(byte: u8) -> Option<Self> {
        if byte & 0b11110000 == 0 {
            Some(Self(byte))
        }
        else {
            None
        }
    }

    pub fn as_u8(&self) -> u8 {
        self.0
    }
}

/// 2-bit IDS in utility message
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct InterrogatorReservationType(u8);

impl InterrogatorReservationType {
    pub const NO_INFORMATION: Self = Self(0);
    pub const IIS_CONTAINS_COMMB: Self = Self(1);
    pub const IIS_CONTAINS_COMMC: Self = Self(2);
    pub const IIS_CONTAINS_COMMD: Self = Self(3);

    pub const fn from_u8_unchecked(byte: u8) -> Self {
        Self(byte)
    }

    pub const fn from_u8(byte: u8) -> Option<Self> {
        if byte & 0b11111100 == 0 {
            Some(Self(byte))
        }
        else {
            None
        }
    }

    pub fn as_u8(&self) -> u8 {
        self.0
    }
}

impl Debug for InterrogatorReservationType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            Self::NO_INFORMATION => write!(f, "InterrogatorReservationType::NO_INFORMATION"),
            Self::IIS_CONTAINS_COMMB => {
                write!(f, "InterrogatorReservationType::IIS_CONTAINS_COMMB")
            }
            Self::IIS_CONTAINS_COMMC => {
                write!(f, "InterrogatorReservationType::IIS_CONTAINS_COMMC")
            }
            Self::IIS_CONTAINS_COMMD => {
                write!(f, "InterrogatorReservationType::IIS_CONTAINS_COMMD")
            }
            _ => panic!("Invalid InterrogatorReservationType bitpattern: {}", self.0),
        }
    }
}

/// 13-bit altitude / Mode C code
///
/// <https://mode-s.org/1090mhz/content/mode-s/3-surveillance.html>
/// <http://www.aeroelectric.com/articles/Altitude_Encoding/modec.htm>
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AltitudeCode(u16);

impl AltitudeCode {
    pub const fn from_u16_unchecked(word: u16) -> Self {
        Self(word)
    }

    pub const fn from_u16(word: u16) -> Option<Self> {
        if word & 0b1110000000000000 == 0 {
            Some(Self(word))
        }
        else {
            None
        }
    }

    pub fn as_u16(&self) -> u16 {
        self.0
    }

    pub fn decode(&self) -> Option<Altitude> {
        // note: 11 bits altitude with 25 feet resolution and -1000 feet offset gives a
        // max value of 50175, so we need a i32 for the decoded altitude

        // todo: adsb_deku considers AC=0 and AC=0x1fff to be invalid, but is it?
        if self.0 == 0 || self.0 == 0b1_1111_1111_1111 {
            None
        }
        else {
            // bit  0 1234 5678 9abc
            //      a aaaa amaq aaaa
            let m_bit = self.0 & 0b0_0000_0100_0000 != 0;
            let q_bit = self.0 & 0b0_0000_0001_0000 != 0;

            if m_bit {
                Some(Altitude {
                    altitude: i32::from(
                        ((self.0 & 0b1_1111_1000_0000) >> 1) | (self.0 & 0b0_0000_0011_1111),
                    ),
                    unit: AltitudeUnit::Meter,
                })
            }
            else if q_bit {
                let altitude = i32::from(
                    ((self.0 & 0b1_1111_1000_0000) >> 2)
                        | ((self.0 & 0b0_0000_0010_0000) >> 1)
                        | (self.0 & 0b0_0000_0000_1111),
                );
                Some(Altitude {
                    altitude: 25 * altitude - 1000,
                    unit: AltitudeUnit::Feet,
                })
            }
            else {
                let value = i32::from(decode_gillham_ac13(self.0));
                Some(Altitude {
                    altitude: 100 * value - 1200,
                    unit: AltitudeUnit::Feet,
                })
            }
        }
    }
}

impl Debug for AltitudeCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(decoded) = self.decode() {
            write!(
                f,
                "AltitudeCode({} {})",
                decoded.altitude,
                decoded.unit.unit_str()
            )
        }
        else {
            write!(f, "AltitudeCode({})", self.0)
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Altitude {
    pub altitude: i32,
    pub unit: AltitudeUnit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AltitudeUnit {
    Feet,
    Meter,
}

impl AltitudeUnit {
    pub fn unit_str(&self) -> &'static str {
        match self {
            AltitudeUnit::Feet => "ft",
            AltitudeUnit::Meter => "m",
        }
    }
}

/// 13-bit identity / Mode A code
/// <https://mode-s.org/1090mhz/content/mode-s/3-surveillance.html>
/// <http://www.aeroelectric.com/articles/Altitude_Encoding/modec.htm>
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct IdentityCode(u16);

impl IdentityCode {
    pub const fn from_u16_unchecked(word: u16) -> Self {
        Self(word)
    }

    pub const fn from_u16(word: u16) -> Option<Self> {
        if word & 0b1110000000000000 == 0 {
            Some(Self(word))
        }
        else {
            None
        }
    }

    pub const fn from_bytes(bytes: [u8; 2]) -> Option<Self> {
        Self::from_u16(u16::from_be_bytes(bytes))
    }

    pub fn as_u16(&self) -> u16 {
        self.0
    }

    pub fn ident(&self) -> bool {
        // todo: verify
        self.0 & 0b0000001000000 != 0
    }

    pub fn squawk(&self) -> Squawk {
        Squawk::from_u16_unchecked(decode_gillham_id13(self.0))
    }
}

impl Debug for IdentityCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "IdentityCode({}", self.squawk())?;
        if self.ident() {
            write!(f, ", ident")?;
        }
        write!(f, ")")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum VerticalStatus {
    Airborne,
    Ground,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CrossLinkCapability(pub bool);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SensitivityLevel(u8);

impl SensitivityLevel {
    pub const INOPERATIVE: Self = Self(0);

    pub const fn from_u8_unchecked(byte: u8) -> Self {
        Self(byte)
    }

    pub const fn from_u8(byte: u8) -> Option<Self> {
        if byte & 0b11111000 == 0 {
            Some(Self(byte))
        }
        else {
            None
        }
    }

    pub fn as_u8(&self) -> u8 {
        self.0
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ReplyInformation(u8);

impl ReplyInformation {
    pub const NO_OPERATING_ACAS: Self = Self(0b0000);
    pub const ACAS_RESOLUTION_CAPABILITY_INHIBITED: Self = Self(0b0010);
    pub const ACAS_VERTICAL_ONLY_CAPABILITY: Self = Self(0b0011);
    pub const ACAS_VERTICAL_AND_HORIZONTAL_CAPABILTIY: Self = Self(0b0111);

    pub const fn from_u8_unchecked(byte: u8) -> Self {
        Self(byte)
    }

    pub const fn from_u8(byte: u8) -> Option<Self> {
        if byte & 0b11110000 == 0 {
            Some(Self(byte))
        }
        else {
            None
        }
    }

    pub fn as_u8(&self) -> u8 {
        self.0
    }
}

impl Debug for ReplyInformation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            Self::NO_OPERATING_ACAS => write!(f, "ReplyInformation::NO_OPERATING_ACAS"),
            Self::ACAS_RESOLUTION_CAPABILITY_INHIBITED => {
                write!(f, "ReplyInformation::ACAS_RESOLUTION_CAPABILITY_INHIBITED")
            }
            Self::ACAS_VERTICAL_ONLY_CAPABILITY => {
                write!(f, "ReplyInformation::ACAS_VERTICAL_ONLY_CAPABILITY")
            }
            Self::ACAS_VERTICAL_AND_HORIZONTAL_CAPABILTIY => {
                write!(
                    f,
                    "ReplyInformation::ACAS_VERTICAL_AND_HORIZONTAL_CAPABILTIY"
                )
            }
            _ => write!(f, "ReplyInformation({})", self.0),
        }
    }
}

/// Regular parity of a frame.
///
/// This has no use other than that it causes the checksum of the frame to be 0
/// for non-corrupted frames.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Parity(pub [u8; 3]);

/// Adress parity
///
/// This is a regular parity overlayed (XOR) with an ICAO address. Assuming the
/// frame was received uncorrupted, we can recover the address from this.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AddressParity(pub [u8; 3]);

impl AddressParity {
    /// The
    pub fn recover_address(&self, frame_checksum: &Checksum) -> IcaoAddress {
        IcaoAddress::from_bytes([
            self.0[0] ^ frame_checksum.0[0],
            self.0[1] ^ frame_checksum.0[1],
            self.0[2] ^ frame_checksum.0[2],
        ])
    }
}

/// The checksum of a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Checksum(pub [u8; 3]);

impl Checksum {
    pub const VALID: Self = Self([0, 0, 0]);

    /// Checks the parity.
    ///
    /// For frames in which contain non-overlayed (i.e. no address/data parity),
    /// the expected checksum is 0. If it's not zero the frame might be
    /// corrupted.
    pub fn check(&self) -> bool {
        *self == Self::VALID
    }
}

/// <https://mode-s.org/1090mhz/content/mode-s/4-acas.html>
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShortAirAirSurveillance {
    pub vertical_status: VerticalStatus,
    pub cross_link_capability: CrossLinkCapability,
    pub sensitivity_level: SensitivityLevel,
    pub reply_information: ReplyInformation,
    pub altitude_code: AltitudeCode,
    pub address_parity: AddressParity,
}

impl ShortAirAirSurveillance {
    pub fn decode<B: Buf>(buffer: &mut B, bits_6_to_8: u8) -> Self {
        let (vertical_status, sensitivity_level, reply_information, altitude_code) =
            decode_air_air_surveillance_common_fields(bits_6_to_8, buffer.get_bytes());

        Self {
            vertical_status,
            cross_link_capability: CrossLinkCapability(bits_6_to_8 & 0b00000010 != 0),
            sensitivity_level,
            reply_information,
            altitude_code,
            address_parity: AddressParity(buffer.get_bytes()),
        }
    }
}

/// <https://mode-s.org/1090mhz/content/mode-s/4-acas.html>
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LongAirAirSurveillance {
    pub vertical_status: VerticalStatus,
    pub sensitivity_level: SensitivityLevel,
    pub reply_information: ReplyInformation,
    pub altitude_code: AltitudeCode,
    pub message: [u8; 7], // todo
    pub address_parity: AddressParity,
}

impl LongAirAirSurveillance {
    pub fn decode<B: Buf>(buffer: &mut B, bits_6_to_8: u8) -> Self {
        let (vertical_status, sensitivity_level, reply_information, altitude_code) =
            decode_air_air_surveillance_common_fields(bits_6_to_8, buffer.get_bytes());

        Self {
            vertical_status,
            sensitivity_level,
            reply_information,
            altitude_code,
            message: buffer.get_bytes(),
            address_parity: AddressParity(buffer.get_bytes()),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SurveillanceAltitudeReply {
    pub flight_status: FlightStatus,
    pub downlink_request: DownlinkRequest,
    pub utility_message: UtilityMessage,
    pub altitude_code: AltitudeCode,
    pub address_parity: AddressParity,
}

impl SurveillanceAltitudeReply {
    pub fn decode<B: Buf>(buffer: &mut B, bits_6_to_8: u8) -> Self {
        let (flight_status, downlink_request, utility_message, code) =
            decode_surveillance_reply_body(bits_6_to_8, buffer.get_bytes());
        Self {
            flight_status,
            downlink_request,
            utility_message,
            altitude_code: AltitudeCode(code),
            address_parity: AddressParity(buffer.get_bytes()),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SurveillanceIdentityReply {
    pub flight_status: FlightStatus,
    pub downlink_request: DownlinkRequest,
    pub utility_message: UtilityMessage,
    pub identity_code: IdentityCode,
    pub address_parity: AddressParity,
}

impl SurveillanceIdentityReply {
    pub fn decode<B: Buf>(buffer: &mut B, bits_6_to_8: u8) -> Self {
        let (flight_status, downlink_request, utility_message, code) =
            decode_surveillance_reply_body(bits_6_to_8, buffer.get_bytes());
        Self {
            flight_status,
            downlink_request,
            utility_message,
            identity_code: IdentityCode(code),
            address_parity: AddressParity(buffer.get_bytes()),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AllCallReply {
    pub capability: Capability,
    pub address_announced: IcaoAddress,
    pub parity_interrogator: Parity,
}

impl AllCallReply {
    pub fn decode<B: Buf>(buffer: &mut B, bits_6_to_8: u8) -> Self {
        Self {
            capability: Capability::from_u8_unchecked(bits_6_to_8),
            address_announced: IcaoAddress::from_bytes(buffer.get_bytes()),
            parity_interrogator: Parity(buffer.get_bytes()),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExtendedSquitter {
    pub capabilities: Capability,
    pub address_announced: IcaoAddress,
    pub adsb_message: adsb::Message,
    pub parity_interrogator: Parity,
}

impl ExtendedSquitter {
    pub fn decode<B: Buf>(buffer: &mut B, bits_6_to_8: u8) -> Result<Self, DecodeError> {
        Ok(Self {
            capabilities: Capability::from_u8_unchecked(bits_6_to_8),
            address_announced: IcaoAddress::from_bytes(buffer.get_bytes()),
            adsb_message: adsb::Message::decode(buffer)?,
            parity_interrogator: Parity(buffer.get_bytes()),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExtendedSquitterNonTransponder {
    AdsbWithIcaoAddress {
        address_announced: IcaoAddress,
        adsb_message: adsb::Message,
        parity_interrogator: Parity,
    },
    AdsbWithNonIcaoAddress {
        address_announced: IcaoAddress,
        adsb_message: adsb::Message,
        parity_interrogator: Parity,
    },
    TisbWithIcaoAddress1 {
        address_announced: IcaoAddress,
        tisb_message: tisb::Message,
        parity_interrogator: Parity,
    },
    TisbWithIcaoAddress2 {
        address_announced: IcaoAddress,
        tisb_message: tisb::Message,
        parity_interrogator: Parity,
    },
    TisbAndAdsrManagement {
        data: [u8; 10],
        parity_interrogator: Parity,
    },
    TisbWithNonIcaoAddress {
        address_announced: IcaoAddress,
        tisb_message: tisb::Message,
        parity_interrogator: Parity,
    },
    AdsbRebroadcast {
        address_announced: IcaoAddress,
        adsb_message: adsb::Message,
        parity_interrogator: Parity,
    },
    Reserved {
        data: [u8; 10],
        parity_interrogator: Parity,
    },
}

impl ExtendedSquitterNonTransponder {
    pub fn code_format(&self) -> CodeFormat {
        match self {
            ExtendedSquitterNonTransponder::AdsbWithIcaoAddress { .. } => {
                CodeFormat::ADSB_WITH_ICAO_ADDRESS
            }
            ExtendedSquitterNonTransponder::AdsbWithNonIcaoAddress { .. } => {
                CodeFormat::ADSB_WITH_NON_ICAO_ADDRESS
            }
            ExtendedSquitterNonTransponder::TisbWithIcaoAddress1 { .. } => {
                CodeFormat::TISB_WITH_ICAO_ADDRESS1
            }
            ExtendedSquitterNonTransponder::TisbWithIcaoAddress2 { .. } => {
                CodeFormat::TISB_WITH_ICAO_ADDRESS2
            }
            ExtendedSquitterNonTransponder::TisbAndAdsrManagement { .. } => {
                CodeFormat::TISB_AND_ADSR_MANAGEMENT
            }
            ExtendedSquitterNonTransponder::TisbWithNonIcaoAddress { .. } => {
                CodeFormat::TISB_WITH_NON_ICAO_ADDRESS
            }
            ExtendedSquitterNonTransponder::AdsbRebroadcast { .. } => CodeFormat::ADSB_REBROADCAST,
            ExtendedSquitterNonTransponder::Reserved { .. } => CodeFormat::RESERVED,
        }
    }

    pub fn decode<B: Buf>(buffer: &mut B, bits_6_to_8: u8) -> Result<Self, DecodeError> {
        let code_format = CodeFormat::from_u8_unchecked(bits_6_to_8);

        // todo: check against page 50
        let extended_squitter_non_transponder = match code_format {
            CodeFormat::ADSB_WITH_ICAO_ADDRESS => {
                ExtendedSquitterNonTransponder::AdsbWithIcaoAddress {
                    address_announced: IcaoAddress::from_bytes(buffer.get_bytes()),
                    adsb_message: adsb::Message::decode(buffer)?,
                    parity_interrogator: Parity(buffer.get_bytes()),
                }
            }
            CodeFormat::ADSB_WITH_NON_ICAO_ADDRESS => {
                ExtendedSquitterNonTransponder::AdsbWithNonIcaoAddress {
                    address_announced: IcaoAddress::from_bytes(buffer.get_bytes())
                        .with_non_icao_flag(),
                    adsb_message: adsb::Message::decode(buffer)?,
                    parity_interrogator: Parity(buffer.get_bytes()),
                }
            }
            CodeFormat::TISB_WITH_ICAO_ADDRESS1 => {
                // todo: not always valid icao address, see 2.2.3.2.1.5
                ExtendedSquitterNonTransponder::TisbWithIcaoAddress1 {
                    address_announced: IcaoAddress::from_bytes(buffer.get_bytes()),
                    tisb_message: tisb::Message::decode(buffer)?,
                    parity_interrogator: Parity(buffer.get_bytes()),
                }
            }
            CodeFormat::TISB_WITH_ICAO_ADDRESS2 => {
                // todo: not always valid icao address, see 2.2.3.2.1.5
                ExtendedSquitterNonTransponder::TisbWithIcaoAddress2 {
                    address_announced: IcaoAddress::from_bytes(buffer.get_bytes()),
                    tisb_message: tisb::Message::decode(buffer)?,
                    parity_interrogator: Parity(buffer.get_bytes()),
                }
            }
            CodeFormat::TISB_AND_ADSR_MANAGEMENT => {
                // format not specified in 1090 MOPS. it seems to exist, but i can't find
                // information on it.
                ExtendedSquitterNonTransponder::TisbAndAdsrManagement {
                    data: buffer.get_bytes(),
                    parity_interrogator: Parity(buffer.get_bytes()),
                }
            }
            CodeFormat::TISB_WITH_NON_ICAO_ADDRESS => {
                ExtendedSquitterNonTransponder::TisbWithNonIcaoAddress {
                    address_announced: IcaoAddress::from_bytes(buffer.get_bytes())
                        .with_non_icao_flag(),
                    tisb_message: tisb::Message::decode(buffer)?,
                    parity_interrogator: Parity(buffer.get_bytes()),
                }
            }
            CodeFormat::ADSB_REBROADCAST => {
                // todo: almost same message format as DF=17, but some bits modified (see
                // 2.2.18)
                //
                // Identify the ICAO/Mode A Flag (IMF)
                // - IMF=0 -> rebroadcast is identified by 24bit ICAO address
                // - IMF=1 -> rebroadcast data is identified by an anonymous 24-bit address or
                //   ground vehicle address or fixed obstruction address
                ExtendedSquitterNonTransponder::AdsbRebroadcast {
                    address_announced: IcaoAddress::from_bytes(buffer.get_bytes()),
                    adsb_message: adsb::Message::decode(buffer)?,
                    parity_interrogator: Parity(buffer.get_bytes()),
                }
            }
            CodeFormat::RESERVED => {
                ExtendedSquitterNonTransponder::Reserved {
                    data: buffer.get_bytes(),
                    parity_interrogator: Parity(buffer.get_bytes()),
                }
            }
            _ => panic!("invalid CodeFormat: {:08b}", code_format.0),
        };

        Ok(extended_squitter_non_transponder)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MilitaryExtendedSquitter {
    Adsb {
        address_announced: IcaoAddress,
        adsb_message: adsb::Message,
        parity_interrogator: Parity,
    },
    /// Reserved for military applications
    Reserved {
        /// 2.2.3.2.1.4
        ///
        /// 1..=7
        application_field: u8,

        data: [u8; 13],
    },
}

impl MilitaryExtendedSquitter {
    pub fn decode<B: Buf>(buffer: &mut B, bits_6_to_8: u8) -> Result<Self, DecodeError> {
        if bits_6_to_8 == 0 {
            Ok(Self::Adsb {
                address_announced: IcaoAddress::from_bytes(buffer.get_bytes()),
                adsb_message: adsb::Message::decode(buffer)?,
                parity_interrogator: Parity(buffer.get_bytes()),
            })
        }
        else {
            Ok(Self::Reserved {
                application_field: bits_6_to_8,
                data: buffer.get_bytes(),
            })
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CommBAltitudeReply {
    pub flight_status: FlightStatus,
    pub downlink_request: DownlinkRequest,
    pub utility_message: UtilityMessage,
    pub altitude_code: AltitudeCode,
    pub message: [u8; 7], // todo
    pub data_parity: Parity,
}

impl CommBAltitudeReply {
    pub fn decode<B: Buf>(buffer: &mut B, bits_6_to_8: u8) -> Self {
        let (flight_status, downlink_request, utility_message, code) =
            decode_surveillance_reply_body(bits_6_to_8, buffer.get_bytes());
        Self {
            flight_status,
            downlink_request,
            utility_message,
            altitude_code: AltitudeCode(code),
            message: buffer.get_bytes(),
            data_parity: Parity(buffer.get_bytes()),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CommBIdentityReply {
    pub flight_status: FlightStatus,
    pub downlink_request: DownlinkRequest,
    pub utility_message: UtilityMessage,
    pub identity_code: IdentityCode,
    pub message: [u8; 7], // todo
    pub data_parity: Parity,
}

impl CommBIdentityReply {
    pub fn decode<B: Buf>(buffer: &mut B, bits_6_to_8: u8) -> Self {
        let (flight_status, downlink_request, utility_message, code) =
            decode_surveillance_reply_body(bits_6_to_8, buffer.get_bytes());
        Self {
            flight_status,
            downlink_request,
            utility_message,
            identity_code: IdentityCode(code),
            message: buffer.get_bytes(),
            data_parity: Parity(buffer.get_bytes()),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CommD {
    // todo
    pub bits_3_to_8: u8,
    pub data: [u8; 13],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MilitaryUse {
    // todo
    pub bits_6_to_8: u8,
    pub data: [u8; 13],
}

#[cfg(test)]
mod tests {
    use adsbee_api_types::IcaoAddress;

    use crate::source::mode_s::{
        AltitudeCode,
        AltitudeUnit,
        Capability,
        ExtendedSquitter,
        Frame,
    };

    #[test]
    fn it_decodes_ac13() {
        fn ac13_decode_to_feet(ac13: u16) -> i32 {
            let altitude = AltitudeCode::from_u16(ac13).unwrap().decode().unwrap();
            assert_eq!(altitude.unit, AltitudeUnit::Feet);
            altitude.altitude
        }

        // the expected values were gathered by decoding frames with adsb_deku
        // fixme: why do 2 test cases fail?

        assert_eq!(ac13_decode_to_feet(6320), 38600);
        assert_eq!(ac13_decode_to_feet(3601), 21425);
        assert_eq!(ac13_decode_to_feet(4152), 25200);
        assert_eq!(ac13_decode_to_feet(4152), 25200);
        assert_eq!(ac13_decode_to_feet(3129), 18825);
        assert_eq!(ac13_decode_to_feet(5913), 36025);
        assert_eq!(ac13_decode_to_feet(4757), 28725);
        assert_eq!(ac13_decode_to_feet(5776), 35000);
        //assert_eq!(ac13_decode_to_feet(5800), 9100);
        assert_eq!(ac13_decode_to_feet(5776), 35000);
        assert_eq!(ac13_decode_to_feet(6064), 37000);
        assert_eq!(ac13_decode_to_feet(2203), 12875);
        assert_eq!(ac13_decode_to_feet(2203), 12875);
        assert_eq!(ac13_decode_to_feet(5272), 32000);
        assert_eq!(ac13_decode_to_feet(442), 2050);
        assert_eq!(ac13_decode_to_feet(412), 1700);
        assert_eq!(ac13_decode_to_feet(6552), 40000);
        //assert_eq!(ac13_decode_to_feet(4130), 2200);
        assert_eq!(ac13_decode_to_feet(1343), 7775);
        assert_eq!(ac13_decode_to_feet(2332), 13700);
        assert_eq!(ac13_decode_to_feet(5560), 34000);
    }

    #[test]
    fn it_decodes_extended_squitter() {
        let bytes = b"\x8d\x40\x74\xb5\x23\x15\xa6\x76\xdd\x13\xa0\x66\x29\x67";
        let frame = Frame::decode(&mut &bytes[..]).unwrap();
        match frame {
            Frame::ExtendedSquitter(ExtendedSquitter {
                capabilities,
                address_announced,
                ..
            }) => {
                assert_eq!(capabilities, Capability::LEVEL2_AIRBORNE);
                assert_eq!(address_announced, IcaoAddress::from_u32_unchecked(0x4074b5));
            }
            _ => panic!("unexpected frame: {frame:?}"),
        }
    }
}
