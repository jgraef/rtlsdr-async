//! Compact Position Reporting
//!
//! Latitude and longitude information is reported using two alternating
//! messages (called even and odd). The original position can be recovered using
//! two methods:
//!
//! - global: needs two messages, but might fail if the messages are from
//!   different "zones".
//! - local: needs one message and a recent reference position.
//!  - airborne: reference position needs to be within 180 NM of the actual
//!    position.
//!  - surface: reference position needs to be within 45 NM of the actual
//!    position.
//!
//! A.1.7 page A-55(905)
//!
//! <https://mode-s.org/1090mhz/content/ads-b/3-airborne-position.html>

use std::ops::Not;

pub use self::decode::{
    decode_globally_unambigious_airborne,
    decode_locally_umambiguous_airborne,
};
use crate::source::mode_s::VerticalStatus;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cpr {
    pub format: CprFormat,
    pub position: CprPosition,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CprFormat {
    Even,
    Odd,
}

impl CprFormat {
    /// Returns the CPR format from the boolean value of the bit in the
    /// respective fields.
    pub fn from_bit(bit: bool) -> Self {
        if bit { CprFormat::Odd } else { CprFormat::Even }
    }

    /// The returned boolean corresponds to the value of the bit encoded in the
    /// frames.
    pub fn is_even(&self) -> bool {
        match self {
            CprFormat::Even => false,
            CprFormat::Odd => true,
        }
    }

    #[inline(always)]
    pub fn is_odd(&self) -> bool {
        !self.is_even()
    }

    /// If this is even, returns odd. If this is odd, returns even.
    pub fn other(&self) -> Self {
        match self {
            Self::Even => Self::Odd,
            Self::Odd => Self::Even,
        }
    }
}

impl Not for CprFormat {
    type Output = Self;

    fn not(self) -> Self::Output {
        self.other()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CprPosition {
    pub latitude: CprValue,
    pub longitude: CprValue,
}

/// 17 bit encoded latitude/longitude
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CprValue(u32);

impl CprValue {
    pub const fn from_u32_unchecked(word: u32) -> Self {
        Self(word)
    }

    pub const fn from_u32(word: u32) -> Option<Self> {
        if word & 0xfffe0000 == 0 {
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

#[derive(Clone, Copy, Debug)]
pub struct Position {
    pub latitude: f64,
    pub longitude: f64,
}

#[derive(Debug, thiserror::Error)]
pub enum CprDecodeError {
    #[error("messages must be from the same longitude zone")]
    MessagesFromDifferentLongitudeZones { nl_lat_even: f64, nl_lat_odd: f64 },
}

mod decode {
    use std::f64::consts::{
        FRAC_PI_2,
        PI,
        TAU,
    };

    use super::{
        Cpr,
        CprDecodeError,
        CprFormat,
        CprPosition,
        CprValue,
        Position,
    };

    const N_Z: f64 = 15.0;
    const D_LAT_EVEN: f64 = 360.0 / (4.0 * N_Z);
    const D_LAT_ODD: f64 = 360.0 / (4.0 * N_Z - 1.0);

    // floor(x) = x.floor()
    // mod(x, y) = x.rem_euclid(y)
    // arccos(x) = x.acos()

    pub fn n_l(lat: f64) -> f64 {
        if lat == 0.0 {
            59.0
        }
        else if lat == 87.0 || lat == -87.0 {
            2.0
        }
        else if lat > 87.0 || lat < -87.0 {
            1.0
        }
        else {
            let a = 1.0 - (FRAC_PI_2 / N_Z).cos();
            let b = (PI * lat / 180.0).cos().powi(2);
            (TAU / (1.0 - a / b).acos()).floor()
        }
    }

    /// scale cpr latitude longitude to a fraction [0, 1]
    #[inline(always)]
    fn lat_lon_cpr(x: CprValue) -> f64 {
        (x.0 as f64) / 131072.0
    }

    #[inline(always)]
    fn adjust_lat(lat: &mut f64) {
        if *lat >= 270.0 {
            *lat -= 360.0;
        }
    }

    /// Decode an even and and odd CPR into latitude and longitude in degrees.
    ///
    /// This might fail if the CPRs are from different zones. If you don't have
    /// both CPRs or if this function fails, you can use
    /// [`decode_locally_umambiguous`].
    pub fn decode_globally_unambigious_airborne(
        cpr_even: CprPosition,
        cpr_odd: CprPosition,
        most_recent: CprFormat,
    ) -> Result<Position, CprDecodeError> {
        let lat_cpr_even = lat_lon_cpr(cpr_even.latitude);
        let lat_cpr_odd = lat_lon_cpr(cpr_odd.latitude);

        // latitude zone index
        let j = (59.0 * lat_cpr_even - 60.0 * lat_cpr_odd + 0.5).floor();

        let mut lat_even = D_LAT_EVEN * (j.rem_euclid(60.0) + lat_cpr_even);
        let mut lat_odd = D_LAT_ODD * (j.rem_euclid(59.0) + lat_cpr_odd);

        adjust_lat(&mut lat_even);
        adjust_lat(&mut lat_odd);

        let nl_lat_even = n_l(lat_even);
        let nl_lat_odd = n_l(lat_odd);

        if nl_lat_even != nl_lat_odd {
            return Err(CprDecodeError::MessagesFromDifferentLongitudeZones {
                nl_lat_even,
                nl_lat_odd,
            });
        }

        let (lat, nl_lat) = match most_recent {
            CprFormat::Even => (lat_even, nl_lat_even),
            CprFormat::Odd => (lat_odd, nl_lat_odd),
        };

        let lon_cpr_even = lat_lon_cpr(cpr_even.longitude);
        let lon_cpr_odd = lat_lon_cpr(cpr_odd.longitude);

        // longitude index
        let m = (lon_cpr_even * (nl_lat - 1.0) - lon_cpr_odd * nl_lat + 0.5).floor();

        // number of longitude zones
        let n_even = nl_lat.max(1.0);
        let n_odd = n_l(lat - 1.0).max(1.0);

        // size of longitude zones
        let d_lon_even = 360.0 / n_even;
        let d_lon_odd = 360.0 / n_odd;

        let lon_even = d_lon_even * (m.rem_euclid(n_even) + lon_cpr_even);
        let lon_odd = d_lon_odd * (m.rem_euclid(n_odd) + lon_cpr_odd);

        let mut lon = match most_recent {
            CprFormat::Even => lon_even,
            CprFormat::Odd => lon_odd,
        };

        if lon >= 180.0 {
            lon -= 360.0;
        }

        Ok(Position {
            latitude: lat,
            longitude: lon,
        })
    }

    /// Decode an a single CPR using a reference position.
    ///
    /// This will always work, but the reference position should be recent.
    pub fn decode_locally_umambiguous_airborne(
        field: Cpr,
        reference_position: &Position,
    ) -> Position {
        let i = match field.format {
            CprFormat::Even => 0.0,
            CprFormat::Odd => 1.0,
        };

        let lat_ref = reference_position.latitude;
        let lon_ref = reference_position.longitude;

        let lat_cpr = lat_lon_cpr(field.position.latitude);
        let lon_cpr = lat_lon_cpr(field.position.longitude);

        let d_lat = 360.0 / (4.0 * N_Z - i);

        // latitude zone index
        let j =
            (lat_ref / d_lat).floor() + (lat_ref.rem_euclid(d_lat) / d_lat - lat_cpr + 0.5).floor();

        let lat = d_lat * (j + lat_cpr);

        let d_lon = 360.0 / (n_l(lat) - i).max(1.0);

        // longitude zone index
        let m =
            (lon_ref / d_lon).floor() + (lon_ref.rem_euclid(d_lon) / d_lon - lon_cpr + 0.5).floor();

        let lon = d_lon * (m + lon_cpr);

        Position {
            latitude: lat,
            longitude: lon,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct DecoderBin<T> {
    vertical_status: VerticalStatus,
    position: CprPosition,
    time: T,
}

/// CPR decoder
///
/// This is generic over the type of time you use. All `T` needs to support is
/// comparisions (i.e [`Ord`][std::cmp::Ord]).
#[derive(Clone, Copy, Debug, Default)]
pub struct CprDecoder<T> {
    even: Option<DecoderBin<T>>,
    odd: Option<DecoderBin<T>>,
}

impl<T: Ord> CprDecoder<T> {
    /// Push a CPR value into the decoder.
    ///
    /// This buffers the CPR value and tries to decode it. It will first try to
    /// decode it globally with a buffered CPR value. If that fails, it will
    /// decode it with a reference position, if available.
    pub fn push(
        &mut self,
        cpr: Cpr,
        vertical_status: VerticalStatus,
        time: T,
        reference: Option<Position>,
    ) -> Option<Position> {
        // fixme: this uses the airborne decoding

        // first we get the bin for this CPR, and the other one
        let (mut this_bin, other_bin) = match cpr.format {
            CprFormat::Even => (&mut self.even, &self.odd),
            CprFormat::Odd => (&mut self.odd, &self.even),
        };

        // if we have another bin, check which one is more recent
        let other_bin_and_most_recent = other_bin.as_ref().map(|other_bin| {
            let most_recent = if time > other_bin.time {
                cpr.format
            }
            else {
                cpr.format.other()
            };
            (other_bin, most_recent)
        });

        // write into the bin for the new CPR
        if let Some(bin) = &mut this_bin {
            if time > bin.time {
                bin.vertical_status = vertical_status;
                bin.position = cpr.position;
                bin.time = time;
            }
            else {
                // if the CPR data is outdated, we can just return
                return None;
            }
        }
        else {
            *this_bin = Some(DecoderBin {
                vertical_status,
                position: cpr.position,
                time,
            });
        }

        // now we can decode :)
        // note: we don't filter stale CPRs here, since the decoding will just fail if
        // its from different zones.
        other_bin_and_most_recent
            .and_then(|(other_bin, most_recent)| {
                // if we have both even and odd position frames, we can try to determine the
                // position without a local reference

                // first check if both CPRs are either airborne or surface
                if other_bin.vertical_status == vertical_status {
                    let (even, odd) = match cpr.format {
                        CprFormat::Even => (cpr.position, other_bin.position),
                        CprFormat::Odd => (other_bin.position, cpr.position),
                    };

                    decode_globally_unambigious_airborne(even, odd, most_recent).ok()
                }
                else {
                    None
                }
            })
            .or_else(|| {
                // either we don't have both even and odd, or the global decode failed
                // (different zones or vertical status)
                reference.map(|reference| {
                    // decode with reference
                    decode_locally_umambiguous_airborne(cpr, &reference)
                })
            })
    }
}

#[cfg(test)]
mod tests {
    use approx::assert_abs_diff_eq;

    use super::{
        Cpr,
        CprFormat,
        CprPosition,
        CprValue,
        Position,
        decode::decode_locally_umambiguous_airborne,
        decode_globally_unambigious_airborne,
    };

    #[test]
    fn decode_globally_unambigious_decoding_example() {
        let cpr_even = CprPosition {
            latitude: CprValue::from_u32_unchecked(0b10110101101001000),
            longitude: CprValue::from_u32_unchecked(0b01100100010101100),
        };
        let cpr_odd = CprPosition {
            latitude: CprValue::from_u32_unchecked(0b10010000110101110),
            longitude: CprValue::from_u32_unchecked(0b01100010000010010),
        };

        let Position {
            latitude,
            longitude,
        } = decode_globally_unambigious_airborne(cpr_even, cpr_odd, CprFormat::Even).unwrap();

        assert_abs_diff_eq!(latitude, 52.2572, epsilon = 0.001);
        assert_abs_diff_eq!(longitude, 3.91937, epsilon = 0.001);
    }

    #[test]
    fn decode_locally_umambiguous_decoding_example() {
        let cpr = Cpr {
            format: CprFormat::Even,
            position: CprPosition {
                latitude: CprValue::from_u32_unchecked(0b10110101101001000),
                longitude: CprValue::from_u32_unchecked(0b01100100010101100),
            },
        };

        let reference_position = Position {
            latitude: 52.258,
            longitude: 3.918,
        };

        let Position {
            latitude,
            longitude,
        } = decode_locally_umambiguous_airborne(cpr, &reference_position);

        assert_abs_diff_eq!(latitude, 52.2572, epsilon = 0.001);
        assert_abs_diff_eq!(longitude, 3.91937, epsilon = 0.001);
    }
}
