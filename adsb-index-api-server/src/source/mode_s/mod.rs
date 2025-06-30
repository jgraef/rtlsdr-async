//! Mode S frame decoder
//!
//! [Reference][1] (page 39 ff), [The 1090 Megaherrtz Riddle][2]
//!
//! # Notes
//!
//! short = 56 bits / 7 bytes
//! long = 112 bits / 14 bytes
//!
//! [1]: http://www.anteni.net/adsb/Doc/1090-WP30-18-DRAFT_DO-260B-V42.pdf
//! [2]: https://mode-s.org/1090mhz/content/mode-s/1-basics.html

pub mod adsb;
pub mod tisb;
pub mod util;

use adsb_index_api_types::{
    IcaoAddress,
    Squawk,
};
use bytes::{
    Buf,
    BufMut,
};

use crate::{
    source::mode_s::util::decode_graham,
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
    CommD(CommD),
}

impl Frame {
    pub fn encode<B: BufMut>(buffer: &mut B) -> Result<(), EncodeError> {
        todo!();
    }

    pub fn decode<B: Buf>(buffer: &mut B) -> Result<Self, DecodeError> {
        let buffer_length = buffer.remaining();
        let byte_0 = buffer.try_get_u8().map_err(|_| DecodeError::NoDf)?;

        let bits_1_to_5 = byte_0 & 0b11111;
        let df = DownlinkFormat::from_u8(bits_1_to_5)?;

        let bits_6_to_8 = byte_0 >> 5;

        let expected_length = df.frame_length();
        if buffer_length < expected_length {
            return Err(DecodeError::Truncated {
                expected_length,
                buffer_length,
            });
        }

        let frame = match df {
            DownlinkFormat::ShortAirAirSurveillance => todo!(),
            DownlinkFormat::SurveillanceAltitudeReply => todo!(),
            DownlinkFormat::SurveillanceIdentityReply => todo!(),
            DownlinkFormat::AllCallReply => todo!(),
            DownlinkFormat::LongAirAirSurveillance => todo!(),
            DownlinkFormat::ExtendedSquitter => {
                Self::ExtendedSquitter(ExtendedSquitter::decode(buffer, bits_6_to_8)?)
            }
            DownlinkFormat::ExtendedSquitterNonTransponder => {
                Self::ExtendedSquitterNonTransponder(ExtendedSquitterNonTransponder::decode(
                    buffer,
                    bits_6_to_8,
                )?)
            }
            DownlinkFormat::MilitaryExtendedSquitter => todo!(),
            DownlinkFormat::CommBAltitudeReply => todo!(),
            DownlinkFormat::CommBIdentityReply => todo!(),
            DownlinkFormat::CommD => todo!(),
        };

        Ok(frame)
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
            Frame::CommD(_) => DownlinkFormat::CommD,
        }
    }

    pub fn length(&self) -> usize {
        self.downlink_format().frame_length()
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
    CommD,
}

impl DownlinkFormat {
    pub fn from_u8(byte: u8) -> Result<Self, DecodeError> {
        if byte & 0b11 == 0b11 {
            Ok(Self::CommD)
        }
        else {
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
                _ => Err(DecodeError::InvalidDf { value: byte }),
            }
        }
    }

    pub fn frame_length(&self) -> usize {
        match self {
            DownlinkFormat::ShortAirAirSurveillance => todo!(),
            DownlinkFormat::SurveillanceAltitudeReply => todo!(),
            DownlinkFormat::SurveillanceIdentityReply => todo!(),
            DownlinkFormat::AllCallReply => LENGTH_SHORT,
            DownlinkFormat::LongAirAirSurveillance => todo!(),
            DownlinkFormat::ExtendedSquitter => LENGTH_LONG,
            DownlinkFormat::ExtendedSquitterNonTransponder => LENGTH_LONG,
            DownlinkFormat::MilitaryExtendedSquitter => LENGTH_LONG,
            DownlinkFormat::CommBAltitudeReply => todo!(),
            DownlinkFormat::CommBIdentityReply => todo!(),
            DownlinkFormat::CommD => todo!(),
        }
    }
}

/// 3 bit capability value
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
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

/// 3-bit code format
///
/// Determines the type of non-transmitter extended squitter message.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CodeFormat(u8);

impl CodeFormat {
    pub const ADSB_WITH_ICAO_ADDRESS: Self = Self(0);
    pub const ADSB_WITH_NON_ICAO_ADDRESS: Self = Self(1);
    pub const TISB_WITH_ICAO_ADDRESS_1: Self = Self(2);
    pub const TISB_WITH_ICAO_ADDRESS_2: Self = Self(3);
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

/// 3-bit flight status
///
/// <https://mode-s.org/1090mhz/content/mode-s/3-surveillance.html>
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
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

/// 5-bit downlink request
///
/// <https://mode-s.org/1090mhz/content/mode-s/3-surveillance.html>
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
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

/// 6-bit utility message
///
/// <https://mode-s.org/1090mhz/content/mode-s/3-surveillance.html>
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UtilityMessage {
    pub interrogator_identifier_subfield: InterrogatorIdentifierSubfield,
    pub interrogator_reservation_type: InterragatorReservationType,
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
            interrogator_identifier_subfield: InterrogatorIdentifierSubfield(byte & 0b001111),
            interrogator_reservation_type: InterragatorReservationType(byte >> 4),
        }
    }

    pub fn as_u8(&self) -> u8 {
        self.interrogator_identifier_subfield.as_u8()
            | (self.interrogator_reservation_type.as_u8() << 4)
    }
}

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

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct InterragatorReservationType(u8);

impl InterragatorReservationType {
    pub const NO_INFORMATION: Self = Self(0);
    pub const IIS_CONTAINS_COMMB_INTERROGATOR_IDENTIFIER_CODE: Self = Self(1);
    pub const IIS_CONTAINS_COMMC_INTERROGATOR_IDENTIFIER_CODE: Self = Self(2);
    pub const IIS_CONTAINS_COMMD_INTERROGATOR_IDENTIFIER_CODE: Self = Self(3);

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

/// 13-bit altitude code
///
/// <https://mode-s.org/1090mhz/content/mode-s/3-surveillance.html>
/// <http://www.aeroelectric.com/articles/Altitude_Encoding/modec.htm>
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
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

    pub fn decode(&self) -> Option<DecodedAltitude> {
        // note: 11 bits altitude with 25 feet resolution and -1000 feet offset gives a
        // max value of 50175, so we need a i32 for the decoded altitude
        if self.0 == 0 || self.0 == 0b0001111111111111 {
            None
        }
        else {
            let m_bit = self.0 & 0b0000001000000 != 0;
            let q_bit = self.0 & 0b0000000010000 != 0;

            let plain_altitude = || {
                i32::from(
                    ((self.0 & 0b1111110000000) >> 2)
                        | ((self.0 & 0b0000000100000) >> 1)
                        | (self.0 & 0b0000000001111),
                )
            };

            if m_bit {
                Some(DecodedAltitude {
                    altitude: plain_altitude(),
                    unit: AltitudeUnit::Meter,
                })
            }
            else if q_bit {
                Some(DecodedAltitude {
                    altitude: 25 * plain_altitude() - 1000,
                    unit: AltitudeUnit::Feet,
                })
            }
            else {
                Some(DecodedAltitude {
                    altitude: 100 * i32::from(decode_graham(self.0)) - 1200,
                    unit: AltitudeUnit::Feet,
                })
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DecodedAltitude {
    pub altitude: i32,
    pub unit: AltitudeUnit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AltitudeUnit {
    Feet,
    Meter,
}

/// 13-bit identity code
/// <https://mode-s.org/1090mhz/content/mode-s/3-surveillance.html>
/// <http://www.aeroelectric.com/articles/Altitude_Encoding/modec.htm>
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
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

    pub fn as_u16(&self) -> u16 {
        self.0
    }

    pub fn ident(&self) -> bool {
        // todo: verify
        self.0 & 0b0000001000000 != 0
    }

    pub fn squawk(&self) -> Squawk {
        Squawk::from_u16_unchecked(decode_graham(self.0))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Parity(pub [u8; 3]);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SurveillanceAltitudeReply {
    pub flight_status: FlightStatus,
    pub downlink_request: DownlinkRequest,
    pub utility_message: UtilityMessage,
    pub altitude_code: AltitudeCode,
    pub address_parity: Parity,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SurveillanceIdentityReply {
    pub flight_status: FlightStatus,
    pub downlink_request: DownlinkRequest,
    pub utility_message: UtilityMessage,
    pub identity_code: IdentityCode,
    pub address_parity: Parity,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AllCallReply {
    pub capability: Capability,
    pub address_announced: IcaoAddress,
    pub parity_interrogator: Parity,
}

impl AllCallReply {
    pub fn decode<B: Buf>(buffer: &mut B, bits_6_to_8: u8) -> Result<Self, DecodeError> {
        Ok(Self {
            capability: Capability::from_u8_unchecked(bits_6_to_8),
            address_announced: IcaoAddress::from_bytes(buffer.get_bytes()),
            parity_interrogator: Parity(buffer.get_bytes()),
        })
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
        data: [u8; 7],
        parity_interrogator: Parity,
    },
    TisbWithNonIcaoAddress {
        address_announced: IcaoAddress,
        tisb_message: tisb::Message,
        parity_interrogator: Parity,
    },
    /// ADS-R rebroadcstr
    ///
    /// 2.2.18.4 ([Reference][1] page 289 ff)
    ///
    /// # TODO
    ///
    /// Identify the ICAO/Mode A Flag (IMF) - i think it's in the ME field
    /// - IMF=0 -> rebroadcast is identified by 24bit ICAO address
    /// - IMF=1 -> rebroadcast data is identified by an anonymous 24-bit address
    ///   or ground vehicle address or fixed obstruction address
    /// Otherwise this is a normal [adsb::Message]
    ///
    /// [1]: http://www.anteni.net/adsb/Doc/1090-WP30-18-DRAFT_DO-260B-V42.pdf
    AdsrRebroadcast {
        data: [u8; 7],
        parity_interrogator: Parity,
    },
    Reserved {
        data: [u8; 7],
        parity_interrogator: Parity,
    },
}

impl ExtendedSquitterNonTransponder {
    pub fn decode<B: Buf>(buffer: &mut B, bits_6_to_8: u8) -> Result<Self, DecodeError> {
        let code_format = CodeFormat::from_u8_unchecked(bits_6_to_8);

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
            CodeFormat::TISB_WITH_ICAO_ADDRESS_1 => {
                // todo: not always valid icao address, see 2.2.3.2.1.5
                ExtendedSquitterNonTransponder::TisbWithIcaoAddress1 {
                    address_announced: IcaoAddress::from_bytes(buffer.get_bytes()),
                    tisb_message: tisb::Message::decode(buffer)?,
                    parity_interrogator: Parity(buffer.get_bytes()),
                }
            }
            CodeFormat::TISB_WITH_ICAO_ADDRESS_2 => {
                // todo: not always valid icao address, see 2.2.3.2.1.5
                ExtendedSquitterNonTransponder::TisbWithIcaoAddress1 {
                    address_announced: IcaoAddress::from_bytes(buffer.get_bytes()),
                    tisb_message: tisb::Message::decode(buffer)?,
                    parity_interrogator: Parity(buffer.get_bytes()),
                }
            }
            CodeFormat::TISB_AND_ADSR_MANAGEMENT => {
                ExtendedSquitterNonTransponder::TisbAndAdsrManagement {
                    data: buffer.get_bytes(),
                    parity_interrogator: Parity(buffer.get_bytes()),
                }
            }
            CodeFormat::TISB_WITH_NON_ICAO_ADDRESS => {
                ExtendedSquitterNonTransponder::TisbWithIcaoAddress1 {
                    address_announced: IcaoAddress::from_bytes(buffer.get_bytes())
                        .with_non_icao_flag(),
                    tisb_message: tisb::Message::decode(buffer)?,
                    parity_interrogator: Parity(buffer.get_bytes()),
                }
            }
            CodeFormat::ADSB_REBROADCAST => {
                ExtendedSquitterNonTransponder::AdsrRebroadcast {
                    data: buffer.get_bytes(),
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
