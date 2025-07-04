//! ADS-B messages

pub mod cpr;

use std::{
    f64::consts::TAU,
    fmt::{
        Debug,
        Display,
    },
    str::FromStr,
};

use adsbee_api_types::Squawk;
use bytes::Buf;

use crate::{
    source::mode_s::{
        DecodeError,
        adsb::cpr::Cpr,
        util::{
            decode_frame_aligned_altitude_or_identity_code,
            decode_frame_aligned_cpr,
            gillham::{
                decode_gillham_ac12,
                decode_gillham_id13,
            },
        },
    },
    util::BufReadBytesExt,
};

/// Reference page 39
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Message {
    AircraftIdentification(AircraftIdentification),
    SurfacePosition(SurfacePosition),
    AirbornePosition(AirbornePosition),
    AirborneVelocity(AirborneVelocity),
    TestMessage([u8; 6]),
    SurfaceSystemMessage(SurfaceSystemMessage),
    /// todo
    TrajectoryChangeMessage {
        sub_type: u8,
        data: [u8; 6],
    },
    AircraftStatus(AircraftStatus),
    TargetStateAndStatusInformation(TargetStateAndStatusInformation),
    AircraftOperationalStatus(AircraftOperationalStatus),
    Reserved {
        type_code: u8,
        sub_type: u8,
        data: [u8; 6],
    },
}

impl Message {
    pub fn decode<B: Buf>(buffer: &mut B) -> Result<Self, DecodeError> {
        let byte_0 = buffer.get_u8();
        let type_code = byte_0 >> 3;
        let bits_6_to_8 = byte_0 & 0b111; // subtype code for some type codes

        let reserved = |buffer: &mut B| {
            //tracing::debug!(?type_code, sub_type = ?bits_6_to_8, "reserved adsb-b
            // message");
            Self::Reserved {
                type_code,
                sub_type: bits_6_to_8,
                data: buffer.get_bytes(),
            }
        };

        let message = match type_code {
            1..=4 => {
                Self::AircraftIdentification(AircraftIdentification::decode(
                    buffer,
                    type_code,
                    bits_6_to_8,
                ))
            }
            5..=8 => Self::SurfacePosition(SurfacePosition::decode(buffer, type_code, bits_6_to_8)),
            0 | 9..=18 | 20..=22 => {
                Self::AirbornePosition(AirbornePosition::decode(buffer, type_code, bits_6_to_8))
            }
            19 => {
                match bits_6_to_8 {
                    1..=4 => Self::AirborneVelocity(AirborneVelocity::decode(buffer, bits_6_to_8)),
                    _ => reserved(buffer),
                }
            }
            23 => {
                match bits_6_to_8 {
                    0 => Self::TestMessage(buffer.get_bytes()),
                    _ => reserved(buffer),
                }
            }
            24 => Self::SurfaceSystemMessage(SurfaceSystemMessage::decode(buffer, bits_6_to_8)),
            27 => {
                Self::TrajectoryChangeMessage {
                    sub_type: bits_6_to_8,
                    data: buffer.get_bytes(),
                }
            }
            28 => Self::AircraftStatus(AircraftStatus::decode(buffer, bits_6_to_8)),
            29 => {
                // rare 2-bit sub type
                let sub_type = bits_6_to_8 >> 1;
                match sub_type {
                    1 => {
                        Self::TargetStateAndStatusInformation(
                            TargetStateAndStatusInformation::decode(buffer, bits_6_to_8 & 1 != 0),
                        )
                    }
                    _ => reserved(buffer),
                }
            }
            31 => {
                Self::AircraftOperationalStatus(AircraftOperationalStatus::decode(
                    buffer,
                    bits_6_to_8,
                ))
            }
            _ => reserved(buffer),
        };

        Ok(message)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AircraftIdentification {
    pub wake_vortex_category: WakeVortexCategory,
    pub callsign: EncodedCallsign,
}

impl AircraftIdentification {
    pub fn decode<B: Buf>(buffer: &mut B, type_code: u8, bits_6_to_8: u8) -> Self {
        Self {
            wake_vortex_category: WakeVortexCategory::from_type_code_and_category_unchecked(
                type_code,
                bits_6_to_8,
            ),
            callsign: EncodedCallsign(buffer.get_bytes()),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SurfacePosition {
    pub movement: Movement,
    pub ground_track: Option<GroundTrack>,
    pub time: bool,
    pub cpr: Cpr,
}

impl SurfacePosition {
    pub fn decode<B: Buf>(buffer: &mut B, _type_code: u8, bits_6_to_8: u8) -> Self {
        // byte       *0        0        1
        // bit  01234567 01234567 01234567
        //      .....aaa aaaabccc ccccd...
        // rest of bits is cpr

        let bytes: [u8; 6] = buffer.get_bytes();

        let a = (bits_6_to_8 << 4) | (bytes[0] >> 4);
        let b = bytes[0] & 0b0000_1000 != 0;
        let c = ((bytes[0] & 0b0000_0111) << 4) | (bytes[1] >> 4);
        let d = bytes[1] & 0b0000_1000 != 0;

        let cpr = decode_frame_aligned_cpr(&bytes[1..]);
        Self {
            movement: Movement(a),
            ground_track: b.then(|| GroundTrack(c)),
            time: d,
            cpr,
        }
    }
}

/// 2.2.3.2.3
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AirbornePosition {
    pub altitude_type: AltitudeType,
    pub surveillance_status: SurveillanceStatus,
    pub single_antenna_flag: bool,
    pub altitude_code: Option<AltitudeCode>,
    pub time: bool,
    pub cpr: Option<Cpr>,
}

impl AirbornePosition {
    pub fn decode<B: Buf>(buffer: &mut B, type_code: u8, bits_6_to_8: u8) -> Self {
        let bytes: [u8; 6] = buffer.get_bytes();

        //       -1        0        1
        // .....aab cccccccc ccccb...
        // rest is cpr if available
        let a = bits_6_to_8 >> 1;
        let b = bits_6_to_8 & 0b1 == 1;
        let c = (u16::from(bytes[0]) << 4) | u16::from(bytes[1] >> 4);
        let d = bytes[2] & 0b00001000 != 0;

        Self {
            altitude_type: AltitudeType::from_type_code(type_code),
            surveillance_status: SurveillanceStatus(a),
            single_antenna_flag: b,
            altitude_code: (c != 0).then(|| AltitudeCode(c)),
            time: d,
            cpr: (type_code != 0).then(|| decode_frame_aligned_cpr(&bytes[1..])),
        }
    }

    pub fn altitude(&self) -> Option<Altitude> {
        self.altitude_code
            .map(|ac| self.altitude_type.altitude(ac.decode()))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AirborneVelocity {
    pub supersonic: bool,
    pub intent_change_flag: bool,
    /// deprecated
    pub ifr_capability_flag: bool,
    pub nac_v: NacV,
    pub velocity_type: VelocityType,
    pub vertical_rate: VerticalRate,
    /// deprecated
    pub turn_indicator: TurnIndicator,
    pub altitude_difference: AltitudeDifference,
}

impl AirborneVelocity {
    pub fn decode<B: Buf>(buffer: &mut B, bits_6_to_8: u8) -> Self {
        let sub_type = bits_6_to_8;
        let supersonic = sub_type == 3 || sub_type == 4;
        let bytes: [u8; 6] = buffer.get_bytes();

        // byte               0        1        2        3        4        5
        // bit         01234567 01234567 01234567 01234567 01234567 01234567
        // field       abcccdee eeeeeeee fggggggg ggghijjj jjjjjjkk lmmmmmmm

        let a = bytes[0] & 0b1000_0000 != 0;
        let b = bytes[0] & 0b0100_0000 != 0;
        let c = (bytes[0] & 0b0011_1000) >> 3;
        let d = bytes[0] & 0b0000_0100 != 0;
        let e = (u16::from(bytes[0] & 0b1100_0000) << 3) | u16::from(bytes[1]);
        let f = bytes[2] & 0b1000_0000 != 0;
        let g = (u16::from(bytes[2] & 0b0111_1111) << 3) | u16::from(bytes[3] >> 5);
        let h = bytes[3] & 0b0001_0000 != 0;
        let i = bytes[3] & 0b0000_1000 != 0;
        let j = (u16::from(bytes[3] & 0b0000_0111) << 6) | u16::from(bytes[4] >> 2);
        let k = bytes[4] & 0b0000_0011;
        let l = bytes[5] & 0b1000_0000 != 0;
        let m = bytes[5] & 0b011_11111;

        let velocity = |v| (v != 0).then(|| Velocity(v));

        // sub-type specific
        let velocity_type = match sub_type {
            1 | 2 => {
                // ground speed
                VelocityType::GroundSpeed(GroundSpeed {
                    direction_east_west: if d {
                        DirectionEastWest::EastToWest
                    }
                    else {
                        DirectionEastWest::WestToEast
                    },
                    velocity_east_west: velocity(e),
                    direction_north_south: if f {
                        DirectionNorthSouth::NorthToSouth
                    }
                    else {
                        DirectionNorthSouth::SouthToNorth
                    },
                    velocity_north_south: velocity(g),
                })
            }
            3 | 4 => {
                // airspeed
                VelocityType::Airspeed(Airspeed {
                    magnetic_heading: d.then(|| MagneticHeading(e)),
                    airspeed_type: if f {
                        AirspeedType::True
                    }
                    else {
                        AirspeedType::Indicated
                    },
                    airspeed_value: velocity(g),
                })
            }
            _ => panic!("Invalid sub type for AirborneVelocity: {}", sub_type),
        };

        Self {
            supersonic,
            intent_change_flag: a,
            ifr_capability_flag: b,
            nac_v: NacV(c),
            velocity_type,
            vertical_rate: VerticalRate {
                source: if h {
                    VerticalRateSource::Barometric
                }
                else {
                    VerticalRateSource::Gnss
                },
                sign: if i {
                    VerticalRateSign::Down
                }
                else {
                    VerticalRateSign::Up
                },
                value: (j != 0).then(|| VerticalRateValue(j)),
            },
            turn_indicator: TurnIndicator(k),
            altitude_difference: AltitudeDifference {
                sign: if l {
                    AltitudeDifferenceSign::GnssBelowBarometric
                }
                else {
                    AltitudeDifferenceSign::GnssAboveBarometric
                },
                value: (m != 0).then(|| AltitudeDifferenceValue(m)),
            },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AircraftStatus {
    EmergencyPriorityStatusAndModeACode(EmergencyPriorityStatusAndModeACode),
    TcasResolutionAdvisoryBroadcast(TcasResolutionAdvisoryBroadcast),
    Reserved { sub_type: u8, data: [u8; 6] },
}

impl AircraftStatus {
    pub fn decode<B: Buf>(buffer: &mut B, bits_6_to_8: u8) -> Self {
        let sub_type = bits_6_to_8;

        match sub_type {
            1 => {
                Self::EmergencyPriorityStatusAndModeACode(
                    EmergencyPriorityStatusAndModeACode::decode(buffer),
                )
            }
            2 => {
                Self::TcasResolutionAdvisoryBroadcast(TcasResolutionAdvisoryBroadcast::decode(
                    buffer,
                ))
            }
            _ => {
                Self::Reserved {
                    sub_type,
                    data: buffer.get_bytes(),
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EmergencyPriorityStatusAndModeACode {
    pub emergency_priority_status: EmergencyPriorityStatus,
    pub mode_a_code: Squawk,
    pub reserved: u32,
}

impl EmergencyPriorityStatusAndModeACode {
    pub fn decode<B: Buf>(buffer: &mut B) -> Self {
        // byte         0        1
        // byte  01234567 01234567
        // bit   aaabbbbb bbbbbbbb
        // rest is reserved

        let bytes: [u8; 2] = buffer.get_bytes();

        EmergencyPriorityStatusAndModeACode {
            emergency_priority_status: EmergencyPriorityStatus(bytes[0] >> 5),
            mode_a_code: {
                // todo: should this include the ident bit? or should it always be zero?
                // (page 139). i think it should be the latter.
                Squawk::from_u16_unchecked(decode_gillham_id13(
                    decode_frame_aligned_altitude_or_identity_code(&bytes[..]),
                ))
            },
            reserved: buffer.get_u32(),
        }
    }

    pub fn from_squawk(squawk: Squawk) -> Self {
        Self {
            emergency_priority_status: EmergencyPriorityStatus::from_squawk(squawk)
                .unwrap_or_default(),
            mode_a_code: squawk,
            reserved: 0,
        }
    }
}

#[derive(Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EmergencyPriorityStatus(u8);

impl EmergencyPriorityStatus {
    pub const NO_EMERGENCY: Self = Self(0);
    pub const GENERAL_EMERGENCY: Self = Self(1);
    pub const LIFEGUARD_MEDICAL_EMERGENCY: Self = Self(2);
    pub const MINIMAL_FUEL: Self = Self(3);
    pub const NO_COMMUNICATIONS: Self = Self(4);
    pub const UNLAWFUL_INTERFERENCE: Self = Self(5);
    pub const DOWNED_AIRCRAFT: Self = Self(6);

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

    pub fn is_emergency(&self) -> bool {
        *self != Self::NO_EMERGENCY
    }

    /// Returns the emergency priority status that shall be set for a given Mode
    /// A code (squawk).
    ///
    /// See 2.2.3.2.7.8.1.1 (page 138)
    pub fn from_squawk(squawk: Squawk) -> Option<Self> {
        match squawk {
            Squawk::AIRCRAFT_HIJACKING => Some(Self::UNLAWFUL_INTERFERENCE),
            Squawk::RADIO_FAILURE => Some(Self::NO_COMMUNICATIONS),
            Squawk::EMERGENCY => Some(Self::GENERAL_EMERGENCY),
            _ => None,
        }
    }
}

impl Debug for EmergencyPriorityStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            Self::NO_EMERGENCY => write!(f, "EmergencyPriorityStatus::NO_EMERGENCY"),
            Self::GENERAL_EMERGENCY => write!(f, "EmergencyPriorityStatus::GENERAL_EMERGENCY"),
            Self::LIFEGUARD_MEDICAL_EMERGENCY => {
                write!(f, "EmergencyPriorityStatus::LIFEGUARD_MEDICAL_EMERGENCY")
            }
            Self::MINIMAL_FUEL => write!(f, "EmergencyPriorityStatus::MINIMAL_FUEL"),
            Self::NO_COMMUNICATIONS => write!(f, "EmergencyPriorityStatus::NO_COMMUNICATIONS"),
            Self::UNLAWFUL_INTERFERENCE => {
                write!(f, "EmergencyPriorityStatus::UNLAWFUL_INTERFERENCE")
            }
            Self::DOWNED_AIRCRAFT => write!(f, "EmergencyPriorityStatus::DOWNED_AIRCRAFT"),
            _ => write!(f, "EmergencyPriorityStatus({})", self.0),
        }
    }
}

/// 2.2.3.2.7.8.2
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TcasResolutionAdvisoryBroadcast {
    pub active_resolution_advisories: ActiveResolutionAdvisories,
    pub racs_record: RacsRecord,
    pub ra_terminated: bool,
    pub multiple_thread_encounter: bool,
    pub threat_type_indicator: ThreatTypeIndicator,
    pub threat_identity_data: ThreatIdentityData,
}

impl TcasResolutionAdvisoryBroadcast {
    pub fn decode<B: Buf>(buffer: &mut B) -> Self {
        // byte         0        1        2        3        4        5
        // bit   01234567 01234567 01234567 01234567 01234567 01234567
        // field aaaaaaaa aaaaaabb bbcdeeff ffffffff ffffffff ffffffff

        let bytes: [u8; 6] = buffer.get_bytes();

        let a = (u16::from(bytes[0]) << 6) | u16::from(bytes[1] >> 2);
        let b = (bytes[1] << 2) | (bytes[2] >> 6);
        let c = bytes[2] & 0b00100000 != 0;
        let d = bytes[2] & 0b00010000 != 0;
        let e = (bytes[2] & 0b00001100) >> 2;
        let f = (u32::from(bytes[2] & 0b11) << 24)
            | (u32::from(bytes[3]) << 16)
            | (u32::from(bytes[4]) << 8)
            | u32::from(bytes[5]);

        Self {
            active_resolution_advisories: ActiveResolutionAdvisories(a),
            racs_record: RacsRecord(b),
            ra_terminated: c,
            multiple_thread_encounter: d,
            threat_type_indicator: ThreatTypeIndicator(e),
            threat_identity_data: ThreatIdentityData(f),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ActiveResolutionAdvisories(u16);

impl ActiveResolutionAdvisories {
    pub const fn from_u16_unchecked(word: u16) -> Self {
        Self(word)
    }

    pub const fn from_u16(word: u16) -> Option<Self> {
        if word & 0b1100000000000000 == 0 {
            Some(Self(word))
        }
        else {
            None
        }
    }

    pub fn as_u16(&self) -> u16 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RacsRecord(u8);

impl RacsRecord {
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
pub struct ThreatTypeIndicator(u8);

impl ThreatTypeIndicator {
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ThreatIdentityData(u32);

impl ThreatIdentityData {
    pub const fn from_u32_unchecked(word: u32) -> Self {
        Self(word)
    }

    pub const fn from_u32(word: u32) -> Option<Self> {
        if word & 0b11111100000000000000000000000000 == 0 {
            Some(Self(word))
        }
        else {
            None
        }
    }

    pub fn as_u32(&self) -> u32 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TargetStateAndStatusInformation {
    pub sil_supplement: SilSupplement,
    pub selected_altitude_type: SelectedAltitudeType,
    //pub navigation_accuracy_category_position: NavigationAccuracyCategoryPosition,
    /// Feel free to open a pull request :3
    pub todo: (),
}

impl TargetStateAndStatusInformation {
    pub fn decode<B: Buf>(buffer: &mut B, bit_8: bool) -> Self {
        // page 106
        let sil_supplement = SilSupplement::from_bit(bit_8);
        let bytes: [u8; 6] = buffer.get_bytes();
        let selected_altitude_type = if bytes[0] & 0b1000000 == 0 {
            SelectedAltitudeType::Fms
        }
        else {
            SelectedAltitudeType::McpFcu
        };

        // todo
        Self {
            sil_supplement,
            selected_altitude_type,
            todo: (),
        }
    }
}

/// Probability of exceeding NIC radius of containment is based on
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SilSupplement {
    PerHour,
    PerSample,
}

impl SilSupplement {
    fn from_bit(bit: bool) -> Self {
        if bit { Self::PerSample } else { Self::PerHour }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SelectedAltitudeType {
    McpFcu,
    Fms,
}

/// Aircraft Operational Status ADS-B Message
///
/// 2.2.3.2.7.2 page 116
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AircraftOperationalStatus {
    /// Airborne participants (sub type 0)
    Airborne {
        capability_class: Option<AirborneCapabilityClass>,
        operational_mode: AirborneOperationalMode,
        mops_version: MopsVersion,
        nic_supp_a: bool,
        nac_p: NacP,
        gva: Gva,
        sil: Sil,
        nic_baro: bool,
        hrd: bool,
        sil_supplement: SilSupplement,
        reserved_56: bool,
    },

    /// Surface participants (sub type 1)
    Surface {
        capability_class: Option<SurfaceCapabilityClass>,
        /// Aircraft/Vehicle Length and Width Code
        lw: LwCode,
        operational_mode: SurfaceOperationalMode,
        mops_version: MopsVersion,
        nic_supp_a: bool,
        nac_p: NacP,
        reserved: u8,
        sil: Sil,
        track_heading: bool,
        hrd: bool,
        sil_supplement: SilSupplement,
        reserved_56: bool,
    },

    Reserved {
        sub_type: u8,
        data: [u8; 6],
    },
}

impl AircraftOperationalStatus {
    pub fn decode<B: Buf>(buffer: &mut B, bits_6_to_8: u8) -> Self {
        let sub_type = bits_6_to_8;

        if sub_type == 0 || sub_type == 1 {
            // byte         0        1        2        3        4        5
            // bit   01234567 01234567 01234567 01234567 01234567 01234567
            // field aaaaaaaa aaaaaaaa bbbbbbbb bbbbbbbb cccdeeee ffgghijk

            let a = buffer.get_u16();
            let b = buffer.get_u16();

            let byte_4 = buffer.get_u8();
            let mops_version = MopsVersion(byte_4 >> 5); // c
            let nic_supp_a = byte_4 & 0b00010000 != 0; // d
            let nac_p = NacP(byte_4 & 0b00001111); // e

            let byte_5 = buffer.get_u8();
            let f = byte_5 >> 6;
            let sil = Sil((byte_5 & 0b00110000) >> 4); // g
            let h = byte_5 & 0b00001000 != 0;
            let hrd = byte_5 & 0b00000100 != 0; // i
            let sil_supplement = SilSupplement::from_bit(byte_5 & 0b00000010 != 0);
            let reserved_56 = byte_5 & 0b00000001 != 0;

            if sub_type == 0 {
                Self::Airborne {
                    capability_class: AirborneCapabilityClass::from_u16(a, mops_version),
                    operational_mode: AirborneOperationalMode::from_u16(b),
                    mops_version,
                    nic_supp_a,
                    nac_p,
                    gva: Gva(f),
                    sil,
                    nic_baro: h,
                    hrd,
                    sil_supplement,
                    reserved_56,
                }
            }
            else {
                Self::Surface {
                    capability_class: SurfaceCapabilityClass::from_u16(a >> 4, mops_version),
                    lw: LwCode(u8::try_from(a & 0b0000_1111).unwrap()),
                    operational_mode: SurfaceOperationalMode::from_u16(b),
                    mops_version,
                    nic_supp_a,
                    nac_p,
                    reserved: f,
                    sil,
                    track_heading: h,
                    hrd,
                    sil_supplement,
                    reserved_56,
                }
            }
        }
        else {
            Self::Reserved {
                sub_type,
                data: buffer.get_bytes(),
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AirborneCapabilityClass {
    Version0 {
        zero_9_10: u8,
        not_tcas: bool,
        cdti: bool,
        reserved_13_24: u16,
    },
    Version2 {
        reserved_9_10: u8,
        tcas_operational: bool,
        x1090es_in: bool,
        reserved_13_14: u8,
        arv: bool,
        ts: bool,
        tc: u8, // todo: 2bits
        uat_in: bool,
        /// # Note
        ///
        /// 1090 MOPS specifies this as a 6 bit field from ME bit 20 - 24 (5
        /// bits). Looks like it's 5 bits really, since it wouldn't fit
        /// otherwise.
        reserved_20_24: u8,
    },
}

impl AirborneCapabilityClass {
    pub fn from_u16(word: u16, mops_version: MopsVersion) -> Option<Self> {
        match mops_version.0 {
            0 => {
                Some(Self::Version0 {
                    zero_9_10: u8::try_from(word >> 14).unwrap(),
                    not_tcas: word & 0b0010_0000_0000_0000 != 0,
                    cdti: word & 0b0001_0000_0000_0000 != 0,
                    reserved_13_24: word & 0b0000_1111_1111_1111,
                })
            }
            2 => {
                Some(Self::Version2 {
                    reserved_9_10: u8::try_from(word >> 14).unwrap(),
                    tcas_operational: word & 0b0010_0000_0000_0000 != 0,
                    x1090es_in: word & 0b0001_0000_0000_0000 != 0,
                    reserved_13_14: u8::try_from((word & 0b0000_1100_0000_0000) >> 10).unwrap(),
                    arv: word & 0b0000_0010_0000_0000 != 0,
                    ts: word & 0b0000_0001_0000_0000 != 0,
                    tc: u8::try_from((word & 0b0000_0000_1100_0000) >> 6).unwrap(),
                    uat_in: word & 0b0000_0000_0010_0000 != 0,
                    reserved_20_24: u8::try_from(word & 0b0000_0000_0001_1111).unwrap(),
                })
            }
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SurfaceCapabilityClass {
    Version2 {
        reserved_9_10: u8,
        /// position offset applied
        poa: bool,
        x1090es_in: bool,
        reserved_13_14: u8,
        // Class B ground vehicle is transmitting with less than 70 watts
        b2_low: bool,
        uat_in: bool,
        nac_v: NacV,
        nic_supplement_c: bool,
    },
}

impl SurfaceCapabilityClass {
    pub fn from_u16(word: u16, mops_version: MopsVersion) -> Option<Self> {
        if mops_version.0 == 2 {
            Some(Self::Version2 {
                reserved_9_10: u8::try_from(word >> 6).unwrap(),
                poa: word & 0b0010_0000_0000_ != 0,
                x1090es_in: word & 0b0001_0000_0000 != 0,
                reserved_13_14: u8::try_from((word & 0b0000_1100_0000) >> 10).unwrap(),
                b2_low: word & 0b0000_0010_0000 != 0,
                uat_in: word & 0b0000_0001_0000 != 0,
                nac_v: NacV(u8::try_from((word & 0b0000_0000_1110) >> 1).unwrap()),
                nic_supplement_c: word & 0b0000_0000_0001 != 0,
            })
        }
        else {
            None
        }
    }
}

struct OperationalModeCommon {
    zero_25_26: u8, // 2 bits
    tcas_ra_active: bool,
    ident_switch_active: bool,
    reserved_atc: bool,
    single_antenna_flag: bool,
    system_design_assurance: SystemDesignAssurance,
}

impl OperationalModeCommon {
    pub fn from_byte(byte: u8) -> Self {
        // bit  01234567
        //      00abcdee
        Self {
            zero_25_26: byte >> 6,                                       // 0
            tcas_ra_active: byte & 0b00100000 != 0,                      // a
            ident_switch_active: byte & 0b00010000 != 0,                 // b
            reserved_atc: byte & 0b00001000 != 0,                        // c
            single_antenna_flag: byte & 0b00000100 != 0,                 // d
            system_design_assurance: SystemDesignAssurance(byte & 0b11), // e
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AirborneOperationalMode {
    pub zero_25_26: u8, // 2 bits
    pub tcas_ra_active: bool,
    pub ident_switch_active: bool,
    pub reserved_atc: bool,
    pub single_antenna_flag: bool,
    pub system_design_assurance: SystemDesignAssurance,
    pub reserved_33_40: u8, // 8 bits
}

impl AirborneOperationalMode {
    pub fn from_u16(word: u16) -> Self {
        let [byte_0, byte_1] = word.to_be_bytes();
        let OperationalModeCommon {
            zero_25_26,
            tcas_ra_active,
            ident_switch_active,
            reserved_atc,
            single_antenna_flag,
            system_design_assurance,
        } = OperationalModeCommon::from_byte(byte_0);

        Self {
            zero_25_26,
            tcas_ra_active,
            ident_switch_active,
            reserved_atc,
            single_antenna_flag,
            system_design_assurance,
            reserved_33_40: byte_1,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SurfaceOperationalMode {
    pub zero_25_26: u8, // 2 bits
    pub tcas_ra_active: bool,
    pub ident_switch_active: bool,
    pub reserved_atc: bool,
    pub single_antenna_flag: bool,
    pub system_design_assurance: SystemDesignAssurance,
    pub gps_antenna_offset: GpsAntennaOffset,
}

impl SurfaceOperationalMode {
    pub fn from_u16(word: u16) -> Self {
        let [byte_0, byte_1] = word.to_be_bytes();
        let OperationalModeCommon {
            zero_25_26,
            tcas_ra_active,
            ident_switch_active,
            reserved_atc,
            single_antenna_flag,
            system_design_assurance,
        } = OperationalModeCommon::from_byte(byte_0);

        Self {
            zero_25_26,
            tcas_ra_active,
            ident_switch_active,
            reserved_atc,
            single_antenna_flag,
            system_design_assurance,
            gps_antenna_offset: GpsAntennaOffset(byte_1),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SystemDesignAssurance(u8);

impl SystemDesignAssurance {
    pub const NONE: Self = Self(0);
    pub const D: Self = Self(1);
    pub const C: Self = Self(2);
    pub const B: Self = Self(3);

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

/// 2.2.3.2.7.2.4.7 page 126
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GpsAntennaOffset(pub u8);

impl GpsAntennaOffset {
    /// Create an [`GpsAntennaOffset`] from lateral and longitudinal offsets.
    ///
    /// # Arguments
    ///
    /// - `lateral`: Lateral distance of the GPS antenna from the longitudinal
    ///   axis (roll) of the aircraft in meters. Valid values are even between
    ///   -6 and 6 inclusive. Negative values are left of the aircraft.
    /// - `longitudinal`: Longitudinal distance of the GPS antenna from the nose
    ///   of the aircraft. Valid values are even between 0 and 62 inclusive.
    pub fn new(mut lateral: i8, mut longitudinal: u8) -> Option<Self> {
        if longitudinal & 1 == 0 && longitudinal <= 62 && lateral & 1 == 0 && lateral <= 6 {
            let mut value = 0;

            if lateral > 0 {
                value |= 0b100_00000;
            }
            else if lateral < 0 {
                lateral *= -1;
            }
            let lateral = u8::try_from(lateral >> 1).unwrap();
            longitudinal >>= 1;

            assert!(lateral & 0b11111100 == 0);
            assert!(longitudinal & 0b11100000 == 0);

            value |= lateral << 5;
            value |= longitudinal;

            Some(Self(value))
        }
        else {
            None
        }
    }
}

/// 3 bit ADS-B version
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MopsVersion(u8);

impl MopsVersion {
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

/// Navigation Accuracy Category for Position
#[derive(Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NacP(u8);

impl NacP {
    pub const UNKNOWN: Self = Self(0);
    pub const RNP_10: Self = Self(1);
    pub const RNP_4: Self = Self(2);
    pub const RNP_2: Self = Self(3);
    pub const RNP_1: Self = Self(4);
    pub const RNP_0_5: Self = Self(5);
    pub const RNP_0_3: Self = Self(6);
    pub const RNP_0_1: Self = Self(7);
    pub const GPS_SA_ON: Self = Self(8);
    pub const GPS_SA_OFF: Self = Self(9);
    pub const GPS_WAAS: Self = Self(10);
    pub const GPS_LAAS: Self = Self(11);

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

impl Debug for NacP {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            Self::UNKNOWN => write!(f, "NacP::UNKNOWN"),
            Self::RNP_10 => write!(f, "NacP::RNP_10"),
            Self::RNP_4 => write!(f, "NacP::RNP_4"),
            Self::RNP_2 => write!(f, "NacP::RNP_2"),
            Self::RNP_1 => write!(f, "NacP::RNP_1"),
            Self::RNP_0_5 => write!(f, "NacP::RNP_0_5"),
            Self::RNP_0_3 => write!(f, "NacP::RNP_0_3"),
            Self::RNP_0_1 => write!(f, "NacP::RNP_0_1"),
            Self::GPS_SA_ON => write!(f, "NacP::GPS_SA_ON"),
            Self::GPS_SA_OFF => write!(f, "NacP::GPS_SA_OFF"),
            Self::GPS_WAAS => write!(f, "NacP::GPS_WAAS"),
            Self::GPS_LAAS => write!(f, "NacP::GPS_LAAS"),
            _ => write!(f, "NacP({})", self.0),
        }
    }
}

/// Navigation Accuracy Category for Velocity
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NacV(u8);

impl NacV {
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

/// Geometric Vertical Accuracy
///
/// 2.2.3.2.7.2.8 page 130
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Gva(u8);

impl Gva {
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

/// Source Integrity Supplement
///
/// 2.2.3.2.7.2.9 page 131
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Sil(u8);

impl Sil {
    pub const UNKNOWN: Self = Self(0);

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

/// Aircraft/Vehicle Length and Width Code
///
/// 2.2.3.2.7.2.11 page 133
///
/// # TODO
///
/// Decoding, encoding (and Debug)
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LwCode(u8);

impl LwCode {
    pub const UNKNOWN: Self = Self(0);

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

/////////////////////////

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SurfaceSystemMessage {
    Reserved { sub_type: u8, data: [u8; 6] },
    MultilaterationSystemStatus([u8; 6]),
}

impl SurfaceSystemMessage {
    pub fn decode<B: Buf>(buffer: &mut B, bits_6_to_8: u8) -> Self {
        let sub_type = bits_6_to_8;

        match sub_type {
            1 => Self::MultilaterationSystemStatus(buffer.get_bytes()),
            _ => {
                Self::Reserved {
                    sub_type,
                    data: buffer.get_bytes(),
                }
            }
        }
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

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EncodedCallsign(pub [u8; 6]);

impl EncodedCallsign {
    /// Expands the encoded callsign to 8bit per character.
    pub fn expand(&self) -> [u8; 8] {
        // byte 0        1        2        3        4        5
        // bit  01234567 01234567 01234567 01234567 01234567 01234567
        // char 00000011 11112222 22333333 44444455 55556666 66777777

        let bytes = &self.0;
        [
            bytes[0] >> 2,
            ((bytes[0] & 0b11) << 4) | (bytes[1] >> 4),
            ((bytes[1] & 0b1111) << 2) | (bytes[2] >> 6),
            (bytes[2] & 0b111111),
            bytes[3] >> 2,
            ((bytes[3] & 0b11) << 4) | (bytes[4] >> 4),
            ((bytes[4] & 0b1111) << 2) | (bytes[5] >> 6),
            (bytes[5] & 0b111111),
        ]
    }

    /// Decodes the callsign into a small string
    pub fn decode(&self) -> Result<Callsign, InvalidCallsign> {
        let expanded = self.expand();
        let mut characters = [0; 8];

        // resolve to ascii character
        for (i, byte) in expanded.iter().enumerate() {
            characters[i] = CALLSIGN_ENCODING[*byte as usize];

            if characters[i] == b'#' {
                return Err(InvalidCallsign {
                    expanded,
                    position: i,
                    character: *byte,
                });
            }
        }

        Ok(Callsign { characters })
    }

    // Decodes the callsign into a small string and ignores invalid characters
    pub fn decode_permissive(&self) -> Callsign {
        let mut expanded = self.expand();

        // resolve to ascii character
        for byte in &mut expanded {
            let resolved = CALLSIGN_ENCODING_PERMISSIVE[*byte as usize];
            *byte = resolved;
        }

        Callsign {
            characters: expanded,
        }
    }
}

/// This is the character set MOPS specifies, but readsb uses this one:
///
/// ```plain
/// b"@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\]^_ !\"#$%&'()*+,-./0123456789:;<=>?"
/// ```
///
/// <https://mode-s.org/1090mhz/content/ads-b/2-identification.html>
const CALLSIGN_ENCODING: &'static [u8] =
    b"#ABCDEFGHIJKLMNOPQRSTUVWXYZ##### ###############0123456789######";

/// readsb uses this and calls it an AIS charset.
const CALLSIGN_ENCODING_PERMISSIVE: &'static [u8] =
    b"@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\]^_ !\"#$%&'()*+,-./0123456789:;<=>?";

impl Debug for EncodedCallsign {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "EncodedCallsign(\"{}\")",
            self.decode_permissive().as_str()
        )
    }
}

#[derive(Clone, Copy, Debug, thiserror::Error)]
#[error("Invalid character {character:02x} at position {position}")]
pub struct InvalidCallsign {
    pub expanded: [u8; 8],
    pub position: usize,
    pub character: u8,
}

/// A decoded callsign.
///
/// This is basically a small string (without heap allocation).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Callsign {
    // note: we only ever fill this with valid ASCII characters
    characters: [u8; Self::LENGTH],
}

impl Callsign {
    const LENGTH: usize = 8;

    pub fn as_str(&self) -> &str {
        // we check this, so we might use the unsafe variant here
        std::str::from_utf8(&self.characters).expect("bug: invalid utf-8 in callsign")
    }
}

impl Debug for Callsign {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Callsign(\"{}\")", self.as_str())
    }
}

impl Display for Callsign {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl FromStr for Callsign {
    type Err = CallsignFromStrError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let n = s.len();
        if n > Self::LENGTH {
            return Err(CallsignFromStrError::InvalidLength(n));
        }

        let mut characters = [0u8; Self::LENGTH];
        for (i, c) in s.chars().enumerate() {
            if !valid_callsign_char(c) {
                return Err(CallsignFromStrError::InvalidChar {
                    position: i,
                    character: c,
                });
            }
            characters[i] = c.try_into().unwrap();
        }

        Ok(Self { characters })
    }
}

impl AsRef<[u8]> for Callsign {
    fn as_ref(&self) -> &[u8] {
        &self.characters[..]
    }
}

impl AsRef<[u8; Self::LENGTH]> for Callsign {
    fn as_ref(&self) -> &[u8; Self::LENGTH] {
        &self.characters
    }
}

impl AsRef<str> for Callsign {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

#[derive(Clone, Debug, thiserror::Error)]
pub enum CallsignFromStrError {
    #[error("Invalid character in callsign: '{character}' at position {position}")]
    InvalidChar { position: usize, character: char },
    #[error("Invalid length for callsign: {0}")]
    InvalidLength(usize),
}

pub fn valid_callsign_char(c: char) -> bool {
    c.is_ascii_uppercase() || c.is_ascii_digit() || c == ' '
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Movement(u8);

impl Movement {
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

    /// Decode movement in 1/8th knots
    pub fn decode_as_1_8th_kt(&self) -> Option<u32> {
        let q = MovementQuantization::from_encoded_value(*self);
        match q {
            MovementQuantization::NotAvailable => None,
            MovementQuantization::Stopped => Some(0),
            MovementQuantization::Quantized {
                encoded_base,
                decoded_base,
                decoded_step,
            } => Some(u32::from(self.0 - *encoded_base) * *decoded_step + *decoded_base),
            MovementQuantization::Exceeding175Kt => Some(1400),
            MovementQuantization::Reserved => None,
        }
    }

    /// Decode movement in knots
    pub fn decode(&self) -> Option<f64> {
        self.decode_as_1_8th_kt().map(|speed| speed as f64 * 0.125)
    }
}

impl Debug for Movement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(kt) = self.decode() {
            write!(f, "Movement({} kt)", kt)
        }
        else {
            write!(f, "Movement(None)")
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

#[derive(Clone, Copy, Debug)]
pub enum MovementQuantization {
    NotAvailable,
    Stopped,
    /// The speed in kt is `1/8 * ((encoded - encoded_base) * decoded_step +
    /// decoded_base)`
    Quantized {
        encoded_base: u8,
        /// in 1/8 kt
        decoded_base: u32,
        /// in 1/8 kt
        decoded_step: u32,
    },
    Exceeding175Kt,
    Reserved,
}

impl MovementQuantization {
    /// Returns the quantization of a surface position movement value.
    pub fn from_encoded_value(encoded: Movement) -> &'static Self {
        match encoded.0 {
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
            0 | 9..=18 => Self::Barometric,
            20..=22 => Self::Gnss,
            _ => panic!("invalid type code: {}", type_code),
        }
    }

    pub fn altitude(&self, altitude: i32) -> Altitude {
        Altitude::from_type_and_value(*self, altitude)
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
/// 2.2.3.2.3.4.3 page 58
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AltitudeCode(u16);

impl AltitudeCode {
    pub const fn from_u16_unchecked(word: u16) -> Self {
        Self(word)
    }

    pub const fn from_u16(word: u16) -> Option<Self> {
        if word & 0b1111000000000000 == 0 && word != 0 {
            Some(Self(word))
        }
        else {
            None
        }
    }

    pub fn as_u16(&self) -> u16 {
        self.0
    }

    /// Decodes the altitude into feet.
    pub fn decode(&self) -> i32 {
        // [This][1] says the 12 bits are the height in meters for GNSS.
        // This is unlikely as it can't encode anything above 4095 meters then.
        //
        // MOPS doesn't say this is encoded any different than barometric. It doesn't
        // even mention which unit. So is it encoded just like barometric and in ft?
        // Both adsb_deku and readsb decode it that way.
        //
        // 11 bits altitude with 25 feet resolution and -1000 feet offset gives a
        // max value of 50175, so we need a i32 for the decoded altitude
        //
        // [1]: https://mode-s.org/1090mhz/content/ads-b/3-airborne-position.html

        let q_bit = self.0 & 0b000000010000 != 0;

        if q_bit {
            // the altitude in 25 feet increments (this removes the Q bit)
            // bit  0123456789ab
            //      aaaaaaaqaaaa

            let value = i32::from(((self.0 & 0b1111_1110_0000) >> 1) | (self.0 & 0b1111));
            value * 25 - 1000
        }
        else {
            // encoded using gillham code in 100 foot increments
            let value = decode_gillham_ac12(self.0);
            i32::from(value) * 100 - 1200
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Altitude {
    /// Barometric altitude in feet
    Barometric(i32),
    /// GNSS altitude in feet
    Gnss(i32),
}

impl Altitude {
    pub fn ty(&self) -> AltitudeType {
        match self {
            Altitude::Barometric(_) => AltitudeType::Barometric,
            Altitude::Gnss(_) => AltitudeType::Gnss,
        }
    }

    pub fn barometric(&self) -> Option<i32> {
        match self {
            Altitude::Barometric(altitude) => Some(*altitude),
            Altitude::Gnss(_) => None,
        }
    }

    pub fn gnss(&self) -> Option<i32> {
        match self {
            Altitude::Barometric(_) => None,
            Altitude::Gnss(altitude) => Some(*altitude),
        }
    }

    pub fn any(&self) -> i32 {
        match self {
            Altitude::Barometric(altitude) => *altitude,
            Altitude::Gnss(altitude) => *altitude,
        }
    }

    /// Create [`Altitude`] from [`AltitudeType`] and altitude value in feet.
    pub fn from_type_and_value(altitude_type: AltitudeType, value: i32) -> Self {
        match altitude_type {
            AltitudeType::Barometric => Altitude::Barometric(value),
            AltitudeType::Gnss => Altitude::Gnss(value),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VelocityType {
    GroundSpeed(GroundSpeed),
    Airspeed(Airspeed),
}

#[derive(Clone, Debug, Copy, PartialEq, Eq)]
pub struct GroundSpeed {
    pub direction_east_west: DirectionEastWest,
    pub velocity_east_west: Option<Velocity>,
    pub direction_north_south: DirectionNorthSouth,
    pub velocity_north_south: Option<Velocity>,
}

impl GroundSpeed {
    /// Returns ground speed as x and y components in knots.
    ///
    /// X values go from west to east, Y values go from south to north.
    pub fn components(&self, supersonic: bool) -> Option<[i16; 2]> {
        let v_ew = i16::try_from(self.velocity_east_west?.as_knots(supersonic)).unwrap();
        let v_ns = i16::try_from(self.velocity_north_south?.as_knots(supersonic)).unwrap();
        let s_ew = match self.direction_east_west {
            DirectionEastWest::WestToEast => 1,
            DirectionEastWest::EastToWest => -1,
        };
        let s_ns = match self.direction_north_south {
            DirectionNorthSouth::SouthToNorth => 1,
            DirectionNorthSouth::NorthToSouth => -1,
        };
        Some([v_ew * s_ew, v_ns * s_ns])
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DirectionNorthSouth {
    SouthToNorth,
    NorthToSouth,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DirectionEastWest {
    WestToEast,
    EastToWest,
}

/// A 10-bit velocity value.
///
/// This is used for east-west and north-south ground speed in [`GroundSpeed`]
/// and for the airspeed in [`Airspeed`]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Velocity(u16);

impl Velocity {
    pub const fn from_u16_unchecked(word: u16) -> Self {
        Self(word)
    }

    pub const fn from_u16(word: u16) -> Option<Self> {
        if word & 0b1111110000000000 == 0 && word != 0 {
            Some(Self(word))
        }
        else {
            None
        }
    }

    pub fn as_u16(&self) -> u16 {
        self.0
    }

    pub fn as_knots(&self, supersonic: bool) -> u16 {
        let v = self.0 - 1;
        let v = if supersonic { v * 4 } else { v };
        v
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Airspeed {
    pub magnetic_heading: Option<MagneticHeading>,
    pub airspeed_type: AirspeedType,
    pub airspeed_value: Option<Velocity>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MagneticHeading(u16);

impl MagneticHeading {
    pub const fn from_u16_unchecked(word: u16) -> Self {
        Self(word)
    }

    pub const fn from_u16(word: u16) -> Option<Self> {
        if word & 0b1111110000000000 == 0 {
            Some(Self(word))
        }
        else {
            None
        }
    }

    /// Magnetic heading as 360/1024 of a degree
    ///
    /// Clockwise from true magnetic north.
    pub fn as_u16(&self) -> u16 {
        self.0
    }

    /// Magnetic heading in degrees
    ///
    /// Clockwise from true magnetic north.
    pub fn as_degrees(&self) -> f64 {
        self.0 as f64 * 360.0 / 1024.0
    }

    /// Magnetic heading in radians
    ///
    /// Clockwise from true magnetic north.
    pub fn as_radians(&self) -> f64 {
        self.0 as f64 * TAU / 1024.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AirspeedType {
    Indicated,
    True,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VerticalRate {
    pub source: VerticalRateSource,
    pub sign: VerticalRateSign,
    pub value: Option<VerticalRateValue>,
}

impl VerticalRate {
    /// Decode as ft/min with positive being up.
    pub fn as_ft_per_min(&self) -> Option<i16> {
        let mut v = i16::try_from(self.value?.as_ft_per_min()).unwrap();
        if matches!(self.sign, VerticalRateSign::Down) {
            v = -v;
        }
        Some(v)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum VerticalRateSource {
    Barometric,
    Gnss,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum VerticalRateSign {
    Up,
    Down,
}

// 9 bit vertical rate without sign
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VerticalRateValue(u16);

impl VerticalRateValue {
    pub const fn from_u16_unchecked(word: u16) -> Self {
        Self(word)
    }

    pub const fn from_u16(word: u16) -> Option<Self> {
        if word & 0b1111111000000000 == 0 && word != 0 {
            Some(Self(word))
        }
        else {
            None
        }
    }

    /// The magnetic heading as 360/1024 of a degree
    pub fn as_u16(&self) -> u16 {
        self.0
    }

    pub fn as_ft_per_min(&self) -> u16 {
        (self.0 - 1) * 64
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AltitudeDifferenceSign {
    GnssAboveBarometric,
    GnssBelowBarometric,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AltitudeDifferenceValue(u8);

impl AltitudeDifferenceValue {
    pub const fn from_u8_unchecked(byte: u8) -> Self {
        Self(byte)
    }

    pub const fn from_u8(byte: u8) -> Option<Self> {
        if byte & 0b10000000 == 0 && byte != 0 {
            Some(Self(byte))
        }
        else {
            None
        }
    }

    pub fn as_u8(&self) -> u8 {
        self.0
    }

    pub fn as_ft(&self) -> u8 {
        (self.0 - 1) * 25
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AltitudeDifference {
    pub sign: AltitudeDifferenceSign,
    pub value: Option<AltitudeDifferenceValue>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TurnIndicator(u8);

impl TurnIndicator {
    pub const fn from_u8_unchecked(byte: u8) -> Self {
        Self(byte)
    }

    pub const fn from_u8(byte: u8) -> Option<Self> {
        if byte & 0b11111000 == 0 && byte != 0 {
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

#[cfg(test)]
mod tests {
    use adsbee_api_types::Squawk;
    use approx::assert_abs_diff_eq;

    use crate::source::mode_s::{
        ExtendedSquitter,
        Frame,
        adsb::{
            AircraftStatus,
            AltitudeCode,
            AltitudeDifferenceSign,
            DirectionEastWest,
            DirectionNorthSouth,
            EmergencyPriorityStatus,
            Message,
            NacV,
            TurnIndicator,
            Velocity,
            VelocityType,
            VerticalRateSign,
            VerticalRateSource,
            VerticalRateValue,
            WakeVortexCategory,
            cpr,
        },
        util::gillham::decode_gillham_id13,
    };

    #[test]
    fn it_decodes_ac12() {
        // from example: https://mode-s.org/1090mhz/content/ads-b/3-airborne-position.html

        let ac = AltitudeCode::from_u16(0xc38).expect("invalid AC12");
        assert_eq!(ac.decode(), 38000);
    }

    #[test]
    fn it_decodes_airborne_position_barometric() {
        // from adsb_deku
        // first 12 bit of "\xc3\x82" is 0xc38 which is the altitude code.
        let bytes = b"\x8D\x40\x62\x1D\x58\xC3\x82\xD6\x90\xC8\xAC\x28\x63\xA7";
        let frame = Frame::decode(&mut &bytes[..]).unwrap();
        match frame {
            Frame::ExtendedSquitter(ExtendedSquitter {
                adsb_message: Message::AirbornePosition(position),
                ..
            }) => {
                assert_eq!(
                    position
                        .altitude()
                        .expect("no altitude")
                        .barometric()
                        .expect("not barometric"),
                    38000
                );
                let cpr = position.cpr.expect("no position");
                assert_eq!(cpr.position.latitude.as_u32(), 93000);
                assert_eq!(cpr.position.longitude.as_u32(), 51372);
                assert_eq!(cpr.format, cpr::Format::Even);
            }
            _ => panic!("unexpected frame: {frame:?}"),
        }
    }

    #[test]
    fn it_decodes_airborne_velocity() {
        // byte               0        1        2        3        4        5
        // bit         01234567 01234567 01234567 01234567 01234567 01234567
        // field       abcccdee eeeeeeee fggggggg ggghijjj jjjjjjkk lmmmmmmm
        //    10011001 00100101 00000001 00101001 01111000 00000100 10000100
        // sub_type=1 -> GroundSpeed, supersonic=false
        //

        // from adsb_deku, but fixed
        let bytes = b"\x8d\xa3\xd4\x25\x99\x25\x01\x29\x78\x04\x84\x71\x2c\x50";
        let frame = Frame::decode(&mut &bytes[..]).unwrap();
        match frame {
            Frame::ExtendedSquitter(ExtendedSquitter {
                adsb_message: Message::AirborneVelocity(velocity),
                ..
            }) => {
                assert!(!velocity.supersonic);
                assert!(!velocity.intent_change_flag); // a
                assert!(!velocity.ifr_capability_flag); // b
                assert_eq!(velocity.nac_v, NacV(0b100)); // c

                match velocity.velocity_type {
                    VelocityType::GroundSpeed(ground_speed) => {
                        assert_eq!(
                            ground_speed.direction_east_west,
                            DirectionEastWest::EastToWest
                        ); // d
                        assert_eq!(ground_speed.velocity_east_west, Some(Velocity(1))); // e
                        assert_eq!(
                            ground_speed.direction_north_south,
                            DirectionNorthSouth::SouthToNorth
                        ); // f
                        assert_eq!(ground_speed.velocity_north_south, Some(Velocity(331))); // g

                        assert_eq!(ground_speed.velocity_east_west.unwrap().as_knots(false), 0);
                        assert_eq!(
                            ground_speed.velocity_north_south.unwrap().as_knots(false),
                            330
                        );
                    }
                    _ => panic!("expected ground speed"),
                }

                assert_eq!(
                    velocity.vertical_rate.source,
                    VerticalRateSource::Barometric
                );
                assert_eq!(velocity.vertical_rate.sign, VerticalRateSign::Down);
                assert_eq!(velocity.vertical_rate.value, Some(VerticalRateValue(1)));
                assert_eq!(velocity.vertical_rate.as_ft_per_min().unwrap(), 0);

                assert_eq!(velocity.turn_indicator, TurnIndicator(0));
                assert_eq!(
                    velocity.altitude_difference.sign,
                    AltitudeDifferenceSign::GnssBelowBarometric
                );
            }
            _ => panic!("unexpected frame: {frame:?}"),
        }
    }

    #[test]
    fn it_decodes_aircraft_identification() {
        let bytes = b"\x8d\x40\x74\xb5\x23\x15\xa6\x76\xdd\x13\xa0\x66\x29\x67";
        let frame = Frame::decode(&mut &bytes[..]).unwrap();
        match frame {
            Frame::ExtendedSquitter(ExtendedSquitter {
                adsb_message: Message::AircraftIdentification(ident),
                ..
            }) => {
                assert_eq!(ident.wake_vortex_category, WakeVortexCategory::Medium2);
                assert_eq!(
                    ident.callsign.decode().expect("invalid callsign").as_str(),
                    "EZY67QN "
                );
            }
            _ => panic!("unexpected frame: {frame:?}"),
        }
    }

    #[test]
    fn it_decodes_aircraft_status_mode_a() {
        let bytes = b"\x8d\xa0\xda\xdb\xe1\x02\x8b\x00\x00\x00\x00\xfe\xad\x7b";
        let frame = Frame::decode(&mut &bytes[..]).unwrap();
        match frame {
            Frame::ExtendedSquitter(ExtendedSquitter {
                adsb_message:
                    Message::AircraftStatus(AircraftStatus::EmergencyPriorityStatusAndModeACode(status)),
                ..
            }) => {
                assert_eq!(
                    status.emergency_priority_status,
                    EmergencyPriorityStatus::NO_EMERGENCY
                );
                assert_eq!(status.mode_a_code, Squawk::from_u16_unchecked(0o6604));
                assert_eq!(status.reserved, 0);
            }
            _ => panic!("unexpected frame: {frame:?}"),
        }
    }

    #[test]
    fn it_decodes_aircraft_status_emergency() {
        let bytes = b"\x8c\x8c\x60\x2c\xe1\x65\xe5\xc1\x82\x51\x96\x55\x3f\x11";

        //       01100101 11100101 11000001 10000010 01010001 10010110
        // byte         0        1
        // byte  01234567 01234567
        // bit   aaabbbbb bbbbbbbb ---reserved------------------------

        // emergency = 0b011 minimal fuel
        // squawk = 0b0010111100101

        // we have a test for this, so we don't have to do this manual here :3
        let expected_squawk = Squawk::from_u16_unchecked(decode_gillham_id13(0b0010111100101));

        let frame = Frame::decode(&mut &bytes[..]).unwrap();
        match frame {
            Frame::ExtendedSquitter(ExtendedSquitter {
                adsb_message:
                    Message::AircraftStatus(AircraftStatus::EmergencyPriorityStatusAndModeACode(status)),
                ..
            }) => {
                assert_eq!(
                    status.emergency_priority_status,
                    EmergencyPriorityStatus::MINIMAL_FUEL
                );
                assert_eq!(status.mode_a_code, expected_squawk);

                // the message we used for testing here has some stuff in the reserved bits ????
                assert_eq!(status.reserved, 0xc1825196);
            }
            _ => panic!("unexpected frame: {frame:?}"),
        }
    }

    #[test]
    fn it_decodes_surface_position() {
        // byte       -1        0        1
        // bit  01234567 01234567 01234567
        //      .....aaa aaaabccc ccccdeff ffffffff fffffffg gggggggg gggggggg
        //      00111010 11101101 01110010 00100100 00010010 00010110 10001000
        // rest of bits is cpr

        let bytes = b"\x8c\x4a\xca\x15\x3a\xed\x72\x24\x12\x16\x88\x4a\xa6\x9b";
        let frame = Frame::decode(&mut &bytes[..]).unwrap();

        match frame {
            Frame::ExtendedSquitter(ExtendedSquitter {
                adsb_message: Message::SurfacePosition(position),
                ..
            }) => {
                assert_eq!(position.movement.0, 0b0101110);
                let ground_track = position.ground_track.expect("no ground track");
                assert_eq!(ground_track.0, 0b1010111);
                assert_abs_diff_eq!(ground_track.as_degrees(), 244.6875);
                assert_abs_diff_eq!(ground_track.as_radians(), 4.270602, epsilon = 0.00001);

                assert_eq!(position.cpr.format, cpr::Format::Even);
                assert_eq!(
                    position.cpr.position.latitude.as_u32(),
                    0b1_0001_0010_0000_1001
                );
                assert_eq!(
                    position.cpr.position.longitude.as_u32(),
                    0b0_0001_0110_1000_1000
                );
            }
            _ => panic!("unexpected frame: {frame:?}"),
        }
    }
}
