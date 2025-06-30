use std::fmt::Debug;

use bytes::Buf;

use crate::{
    source::mode_s::{
        AltitudeUnit,
        DecodeError,
        cpr::{
            Cpr,
            CprFormat,
        },
        util::decode_frame_aligned_encoded_position,
    },
    util::BufReadBytesExt,
};

/// Reference page 49
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Message {
    NoPosition,
    AircraftIdentification(AircraftIdentification),
    SurfacePosition(SurfacePosition),
    AirbornePosition(AirbornePosition),
    AirborneVelocity(AirborneVelocity),
    TestMessage([u8; 7]),
    SurfaceSystemMessage([u8; 7]),
    AircraftStatus(AircraftStatus),
    TargetStateAndStatusInformation(TargetStateAndStatusInformation),
    Reserved {
        type_code: u8,
        sub_type: u8,
        data: [u8; 7],
    },
}

impl Message {
    pub fn decode<B: Buf>(buffer: &mut B) -> Result<Self, DecodeError> {
        let byte_0 = buffer.get_u8();
        let type_code = byte_0 >> 3;
        let bits_6_to_8 = byte_0 & 0b111; // subtype code for some type codes

        let reserved = |buffer: &mut B| {
            Self::Reserved {
                type_code,
                sub_type: bits_6_to_8,
                data: buffer.get_bytes(),
            }
        };

        let message = match type_code {
            0 => {
                //Self::NoPosition
                todo!("no position");
            }
            1..=4 => {
                Self::AircraftIdentification(AircraftIdentification::decode(
                    buffer,
                    type_code,
                    bits_6_to_8,
                )?)
            }
            5..=8 => Self::SurfacePosition(SurfacePosition::decode(buffer, type_code, bits_6_to_8)),
            9..=18 | 20..=22 => {
                Self::AirbornePosition(AirbornePosition::decode(buffer, type_code, bits_6_to_8))
            }
            19 => {
                match bits_6_to_8 {
                    1..=4 => Self::AirborneVelocity(AirborneVelocity::decode(buffer, bits_6_to_8)?),
                    _ => reserved(buffer),
                }
            }
            23 => {
                match bits_6_to_8 {
                    0 => Self::TestMessage(buffer.get_bytes()),
                    _ => reserved(buffer),
                }
            }
            24 => {
                match bits_6_to_8 {
                    1 => Self::SurfaceSystemMessage(buffer.get_bytes()),
                    _ => reserved(buffer),
                }
            }
            27 => todo!("reserved for trajectory change message"),
            28 => Self::AircraftStatus(AircraftStatus::decode(buffer, bits_6_to_8)?),
            29 => {
                Self::TargetStateAndStatusInformation(TargetStateAndStatusInformation::decode(
                    buffer,
                    bits_6_to_8,
                )?)
            }
            _ => reserved(buffer),
        };

        Ok(message)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AircraftIdentification {
    pub wake_vortex_category: WakeVortexCategory,
    pub callsign: Callsign,
}

impl AircraftIdentification {
    pub fn decode<B: Buf>(
        buffer: &mut B,
        type_code: u8,
        bits_6_to_8: u8,
    ) -> Result<Self, DecodeError> {
        Ok(Self {
            wake_vortex_category: WakeVortexCategory::from_type_code_and_category_unchecked(
                type_code,
                bits_6_to_8,
            ),
            callsign: Callsign::from_bytes(buffer.get_bytes())?,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SurfacePosition {
    pub ground_speed: GroundSpeed,
    pub ground_track: Option<GroundTrack>,
    pub time: bool,
    pub cpr_format: CprFormat,
    pub cpr_position: Cpr,
}

impl SurfacePosition {
    pub fn decode<B: Buf>(buffer: &mut B, type_code: u8, bits_6_to_8: u8) -> Self {
        let bytes: [u8; 6] = buffer.get_bytes();
        let (cpr_format, cpr_position) = decode_frame_aligned_encoded_position(&bytes[1..]);
        Self {
            ground_speed: GroundSpeed((bits_6_to_8 << 4) | (bytes[0] >> 4)),
            ground_track: if bytes[0] & 0b00001000 == 0 {
                None
            }
            else {
                Some(GroundTrack((bytes[0] << 5) | (bytes[1] >> 4)))
            },
            time: bytes[1] & 0b00001000 != 0,
            cpr_format,
            cpr_position,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AirbornePosition {
    pub altitude_type: AltitudeType,
    pub surveillance_status: SurveillanceStatus,
    pub single_antenna_flag: bool,
    pub encoded_altitude: AltitudeCode,
    pub time: bool,
    pub cpr_format: CprFormat,
    pub cpr_position: Cpr,
}

impl AirbornePosition {
    pub fn decode<B: Buf>(buffer: &mut B, type_code: u8, bits_6_to_8: u8) -> Self {
        let bytes: [u8; 6] = buffer.get_bytes();
        let (cpr_format, cpr_position) = decode_frame_aligned_encoded_position(&bytes[2..]);
        Self {
            //        0        1        2        3        4        5        6
            // tttttssS aaaaaaaa aaaaTFll llllllll lllllllL LLLLLLLL LLLLLLLL
            altitude_type: AltitudeType::from_type_code(type_code),
            surveillance_status: SurveillanceStatus(bits_6_to_8 >> 1),
            single_antenna_flag: bits_6_to_8 & 0b1 == 1,
            encoded_altitude: AltitudeCode(u16::from(bytes[0] << 4) | u16::from(bytes[1] >> 4)),
            time: bytes[2] & 0b00001000 != 0,
            cpr_format,
            cpr_position,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AirborneVelocity {}

impl AirborneVelocity {
    pub fn decode<B: Buf>(buffer: &mut B, bits_6_to_8: u8) -> Result<Self, DecodeError> {
        let sub_type = bits_6_to_8;

        let byte_0 = buffer.get_u8();

        let intent_change_flag = byte_0 & 0b10000000 != 0;
        let ifr_capability_flag = byte_0 & 0b01000000 != 0;

        todo!();
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AircraftStatus {}

impl AircraftStatus {
    pub fn decode<B: Buf>(buffer: &mut B, bits_6_to_8: u8) -> Result<Self, DecodeError> {
        todo!();
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TargetStateAndStatusInformation {}

impl TargetStateAndStatusInformation {
    pub fn decode<B: Buf>(buffer: &mut B, bits_6_to_8: u8) -> Result<Self, DecodeError> {
        todo!();
    }
}

/// <https://mode-s.org/1090mhz/content/ads-b/2-identification.html>
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum WakeVortexCategory {
    Reserved { type_code: u8, category: u8 },
    NoCategoryInformation { type_code: u8 },
    SurfaceEmergencyVehicle,
    SurfaceServiceVehicle,
    GroundObstruction { category: u8 },
    GliderSailplane,
    LighterThanAir,
    ParachutistSkydiver,
    UltralightHangGliderParaGlider,
    UnmannedAerialVehicle,
    SpaceTransatmospherricVehicle,
    Light,
    Medium1,
    Medium2,
    HighVortexAirrcraft,
    Heavy,
    HighPerformance,
    Rotorcraft,
}

impl WakeVortexCategory {
    pub const fn from_type_code_and_category_unchecked(type_code: u8, category: u8) -> Self {
        match (type_code, category) {
            (_, 0) => Self::NoCategoryInformation { type_code },
            (2, 1) => Self::SurfaceEmergencyVehicle,
            (2, 3) => Self::SurfaceServiceVehicle,
            (2, 4..=7) => Self::GroundObstruction { category },
            (3, 1) => Self::GliderSailplane,
            (3, 2) => Self::LighterThanAir,
            (3, 3) => Self::ParachutistSkydiver,
            (3, 4) => Self::UltralightHangGliderParaGlider,
            (3, 6) => Self::UnmannedAerialVehicle,
            (3, 7) => Self::SpaceTransatmospherricVehicle,
            (4, 1) => Self::Light,
            (4, 2) => Self::Medium1,
            (4, 3) => Self::Medium2,
            (4, 4) => Self::HighVortexAirrcraft,
            (4, 5) => Self::Heavy,
            (4, 6) => Self::HighPerformance,
            (4, 7) => Self::Rotorcraft,

            _ => {
                Self::Reserved {
                    type_code,
                    category,
                }
            }
        }
    }

    pub const fn from_type_code_and_category(type_code: u8, category: u8) -> Option<Self> {
        if type_code & 0b11100000 == 0 && category & 0b00000111 == 0 {
            Some(Self::from_type_code_and_category_unchecked(
                type_code, category,
            ))
        }
        else {
            None
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Callsign {
    // note: we verified that this is valid ascii, and thus we can also create utf-8 strs from it.
    characters: [u8; 8],
    trimmed: [u8; 2],
}

impl Callsign {
    pub fn from_bytes(bytes: [u8; 6]) -> Result<Self, DecodeError> {
        // byte 0        1        2        3        4        5
        // bit  01234567 01234567 01234567 01234567 01234567 01234567
        // char 00000011 11112222 22333333 44444455 55556666 66777777

        // expand into 8 bits per character
        let mut expanded = [
            bytes[0] >> 2,
            ((bytes[0] & 0b11) << 4) | (bytes[1] >> 4),
            ((bytes[1] & 0b1111) << 2) | (bytes[2] >> 6),
            (bytes[2] & 0b111111),
            bytes[3] >> 2,
            ((bytes[3] & 0b11) << 4) | (bytes[4] >> 4),
            ((bytes[4] & 0b1111) << 2) | (bytes[5] >> 6),
            (bytes[5] & 0b111111),
        ];

        let mut first_non_space = 0;
        let mut last_non_space = 0;
        let mut saw_non_space = false;

        // resolve to ascii character
        for (i, byte) in expanded.iter_mut().enumerate() {
            let resolved = CALLSIGN_ENCODING[*byte as usize];

            if resolved == b'#' {
                return Err(DecodeError::InvalidCallsign {
                    encoding: bytes,
                    invalid_byte: resolved,
                });
            }

            if resolved != b' ' {
                last_non_space = i;
                if !saw_non_space {
                    first_non_space = i;
                }
                saw_non_space = true;
            }

            *byte = resolved;
        }

        Ok(Self {
            characters: expanded,
            trimmed: [first_non_space as u8, last_non_space as u8],
        })
    }

    pub fn as_str(&self) -> &str {
        // we check this, so we might use the unsafe variant here
        std::str::from_utf8(&self.characters).expect("bug: invalid utf-8 in callsign")
    }
}

/// <https://mode-s.org/1090mhz/content/ads-b/2-identification.html>
pub const CALLSIGN_ENCODING: &'static [u8] =
    b"#ABCDEFGHIJKLMNOPQRSTUVWXYZ##### ###############0123456789######";

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GroundSpeed(u8);

impl GroundSpeed {
    pub const fn from_u8_unchecked(byte: u8) -> Self {
        Self(byte)
    }

    pub const fn from_u8(byte: u8) -> Option<Self> {
        if byte & 0b10000000 == 0 {
            Some(Self(byte))
        }
        else {
            None
        }
    }

    pub fn as_u8(&self) -> u8 {
        self.0
    }

    pub fn decode_as_1_8th_kt(&self) -> Option<u32> {
        let q = GroundSpeedQuantization::from_encoded_value(self.0);
        match q {
            GroundSpeedQuantization::NotAvailable => None,
            GroundSpeedQuantization::Stopped => Some(0),
            GroundSpeedQuantization::Quantized {
                encoded_base,
                decoded_base,
                decoded_step,
            } => Some(u32::from(self.0 - *encoded_base) * *decoded_step + *decoded_base),
            GroundSpeedQuantization::Exceeding175Kt => Some(1400),
            GroundSpeedQuantization::Reserved => None,
        }
    }

    pub fn decode(&self) -> Option<f64> {
        self.decode_as_1_8th_kt().map(|speed| speed as f64 * 0.125)
    }
}

impl Debug for GroundSpeed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(kt) = self.decode() {
            write!(f, "GroundSpeed({} kt)", kt)
        }
        else {
            write!(f, "GroundSpeed(None)")
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GroundTrack(u8);

impl GroundTrack {
    pub const fn from_u8_unchecked(byte: u8) -> Self {
        Self(byte)
    }

    pub const fn from_u8(byte: u8) -> Option<Self> {
        if byte & 0b10000000 == 0 {
            Some(Self(byte))
        }
        else {
            None
        }
    }

    pub fn as_u8(&self) -> u8 {
        self.0
    }

    pub fn as_radians(&self) -> f64 {
        std::f64::consts::TAU * (self.0 as f64) / 128.0
    }

    pub fn as_degrees(&self) -> f64 {
        360.0 * (self.0 as f64) / 128.0
    }
}

impl Debug for GroundTrack {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "GroundTrack({:.1}°)", self.as_degrees())
    }
}

// todo: needs a neater interface to make it public
#[derive(Clone, Copy, Debug)]
enum GroundSpeedQuantization {
    NotAvailable,
    Stopped,
    Quantized {
        encoded_base: u8,
        decoded_base: u32, // in 1/8 kt
        decoded_step: u32, // in 1/8 kt
    },
    Exceeding175Kt,
    Reserved,
}

impl GroundSpeedQuantization {
    pub fn from_encoded_value(encoded: u8) -> &'static Self {
        match encoded {
            0 => &Self::NotAvailable,
            1 => &Self::Stopped,
            2..=8 => {
                &Self::Quantized {
                    encoded_base: 2,
                    decoded_base: 1,
                    decoded_step: 1,
                }
            }
            9..=12 => {
                &Self::Quantized {
                    encoded_base: 9,
                    decoded_base: 8,
                    decoded_step: 2,
                }
            }
            13..=38 => {
                &Self::Quantized {
                    encoded_base: 13,
                    decoded_base: 16,
                    decoded_step: 4,
                }
            }
            39..=93 => {
                &Self::Quantized {
                    encoded_base: 39,
                    decoded_base: 120,
                    decoded_step: 8,
                }
            }
            94..=108 => {
                &Self::Quantized {
                    encoded_base: 94,
                    decoded_base: 560,
                    decoded_step: 16,
                }
            }
            109..=123 => {
                &Self::Quantized {
                    encoded_base: 109,
                    decoded_base: 800,
                    decoded_step: 40,
                }
            }
            124 => &Self::Exceeding175Kt,
            _ => &Self::Reserved,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AltitudeType {
    Barometric,
    Gnss,
}

impl AltitudeType {
    pub fn from_type_code(type_code: u8) -> Self {
        match type_code {
            9..=18 => Self::Barometric,
            20..=22 => Self::Gnss,
            _ => panic!("invalid type code: {}", type_code),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SurveillanceStatus(u8);

impl SurveillanceStatus {
    pub const NO_CONDITION: Self = Self(0);
    pub const PERMANENT_ALERT: Self = Self(1);
    pub const TEMPORARY_ALERT: Self = Self(2);
    pub const SPI_CONDITION: Self = Self(3);

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

impl Debug for SurveillanceStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            Self::NO_CONDITION => write!(f, "SurveillanceStatus::NO_CONDITION"),
            Self::PERMANENT_ALERT => write!(f, "SurveillanceStatus::PERMANENT_ALERT"),
            Self::TEMPORARY_ALERT => write!(f, "SurveillanceStatus::TEMPORARY_ALERT"),
            Self::SPI_CONDITION => write!(f, "SurveillanceStatus::SPI_CONDITION"),
            _ => panic!("invalid SurveillanceStatus bitpattern: {}", self.0),
        }
    }
}

/// 12-bit altitude code
///
/// page 59
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AltitudeCode(u16);

impl AltitudeCode {
    pub const fn from_u16_unchecked(word: u16) -> Self {
        Self(word)
    }

    pub const fn from_u16(word: u16) -> Option<Self> {
        if word & 0b1111000000000000 == 0 {
            Some(Self(word))
        }
        else {
            None
        }
    }

    pub fn as_u16(&self) -> u16 {
        self.0
    }

    pub fn decode(&self, altitude_type: AltitudeType) -> Option<DecodedAltitude> {
        // note: 11 bits altitude with 25 feet resolution and -1000 feet offset gives a
        // max value of 50175, so we need a i32 for the decoded altitude

        if self.0 == 0 {
            None
        }
        else {
            let q_bit = self.0 & 0b000000010000 != 0;

            if q_bit {
                Some(DecodedAltitude {
                    altitude_type,
                    altitude: i32::from((self.0 >> 5) | (self.0 & 0b1111)) * 25 - 1000,
                })
            }
            else {
                todo!();
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DecodedAltitude {
    pub altitude_type: AltitudeType,
    pub altitude: i32,
}

impl DecodedAltitude {
    pub fn unit(&self) -> AltitudeUnit {
        match self.altitude_type {
            AltitudeType::Barometric => AltitudeUnit::Feet,
            AltitudeType::Gnss => AltitudeUnit::Meter,
        }
    }
}
