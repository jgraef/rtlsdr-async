pub mod gillham;

use bytes::Buf;

use crate::source::mode_s::{
    AltitudeCode,
    DownlinkRequest,
    FlightStatus,
    ReplyInformation,
    SensitivityLevel,
    UtilityMessage,
    VerticalStatus,
    adsb::cpr::{
        Cpr,
        CprCoodinate,
        CprFormat,
        CprPosition,
    },
};

/// Decode fields common in surveillance replies.
///
/// This decodes the data with respect to its alignment in a frame.
///
/// `bits_6_to_8` are the last 3 bits of the first byte.
/// `bytes` are the remaining bytes.
pub fn decode_surveillance_reply_body(
    bits_6_to_8: u8,
    bytes: [u8; 3],
) -> (FlightStatus, DownlinkRequest, UtilityMessage, u16) {
    // bits_6_to_8  bytes [0]      [1]      [2]
    // .....fff     ddddduuu  uuuaaaaa aaaaaaaa
    let flight_status = FlightStatus(bits_6_to_8);
    let downlink_request = DownlinkRequest(bytes[0] >> 3);
    let utility_message =
        UtilityMessage::from_u8_unchecked(((bytes[0] & 0b111) << 3) | (bytes[1] >> 5));
    (
        flight_status,
        downlink_request,
        utility_message,
        decode_frame_aligned_altitude_or_identity_code(&bytes[1..=2]),
    )
}

/// Decode fields common in air air surveillance frames.
///
/// This decodes the data with respect to its alignment in a frame.
///
/// `bits_6_to_8` are the last 3 bits of the first byte.
/// `bytes` are the remaining bytes.
///
/// ```plain
/// bits_6_to_8  bytes [0]      [1]      [2]
/// .....vxx     sssxxrrr  rxxaaaaa aaaaaaaa
/// ```
pub fn decode_air_air_surveillance_common_fields(
    bits_6_to_8: u8,
    bytes: [u8; 3],
) -> (
    VerticalStatus,
    SensitivityLevel,
    ReplyInformation,
    AltitudeCode,
) {
    let vertical_status = if bits_6_to_8 & 0b100 == 0 {
        VerticalStatus::Airborne
    }
    else {
        VerticalStatus::Ground
    };
    let sensitivity_level = SensitivityLevel(bytes[0] >> 6);
    let reply_information = ReplyInformation(((bytes[0] & 0b111) << 1) | (bytes[1] >> 7));
    let altitude_code = AltitudeCode(decode_frame_aligned_altitude_or_identity_code(
        &bytes[1..=2],
    ));
    (
        vertical_status,
        sensitivity_level,
        reply_information,
        altitude_code,
    )
}

/// Extracts the AC13 or ID13.
///
/// This extracts a 13 bit word starting at bit 3:
///
/// ```plain
/// byte         0        1
/// bit   01234567 01234567
/// value ...aaaaa aaaaaaaa
/// ```
///
/// This is useful for decoding surveillance replies and air air surveillance
/// frames.
pub fn decode_frame_aligned_altitude_or_identity_code(bytes: &[u8]) -> u16 {
    (u16::from(bytes[0] & 0b00011111) << 8) | u16::from(bytes[1])
}

/// Decode CPR from a frame.
///
/// This expects the CPR latitude and longitude to start at bit 6 in `bytes[0]`.
/// A total of 5 bytes are required.
///
/// ```plain
/// byte         0        1        2        3        4
/// bit   01234567 01234567 01234567 01234567 01234567
/// value ......aa aaaaaaaa aaaaaaab bbbbbbbb bbbbbbbb
/// ```
pub fn decode_frame_aligned_cpr(bytes: &[u8]) -> Cpr {
    let format = CprFormat::from_bit(bytes[0] & 0b00000100 != 0);
    let position = CprPosition {
        latitude: CprCoodinate::from_u32_unchecked(
            (u32::from(bytes[0] & 0b10) << 15)
                | (u32::from(bytes[1]) << 7)
                | u32::from(bytes[2] >> 1),
        ),
        longitude: CprCoodinate::from_u32_unchecked(
            (u32::from(bytes[2] & 0b1) << 16) | (u32::from(bytes[3]) << 8) | u32::from(bytes[4]),
        ),
    };
    Cpr { format, position }
}

/// CRC algorithm for Mode-S
///
/// <https://www.ll.mit.edu/sites/default/files/publication/doc/2018-12/Gertz_1984_ATC-117_WW-15318.pdf>
pub const CRC_24_MODES: crc::Algorithm<u32> = crc::Algorithm {
    width: 24,
    poly: 0xfff409,
    init: 0,
    refin: false,
    refout: false,
    xorout: 0x000000,
    check: 0x54268,
    residue: 0x000000,
};

/// Wraps a [`Buf`][bytes::Buf] that calculates the CRC checksum of the read
/// data.
pub struct CrcBuf<'a, B> {
    pub inner: B,
    pub digest: crc::Digest<'a, u32>,
}

impl<'a, B: Buf> Buf for CrcBuf<'a, B> {
    fn remaining(&self) -> usize {
        self.inner.remaining()
    }

    fn chunk(&self) -> &[u8] {
        self.inner.chunk()
    }

    fn advance(&mut self, cnt: usize) {
        let chunk = &self.inner.chunk()[..cnt];
        self.digest.update(chunk);
        self.inner.advance(cnt);
    }
}
