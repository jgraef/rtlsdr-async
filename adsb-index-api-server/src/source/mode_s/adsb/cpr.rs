//! Compact Position Reporting
//!
//! Latitude and longitude information is reported using two alternating
//! messages (called even and odd). The original position can be recovered using
//! two methods:
//!
//! - global: needs two messages, but might fail if the messages are from
//!   different "zones".
//! - local: needs one message and a recent reference position.
//!   - airborne: reference position needs to be within 180 NM of the actual
//!     position.
//!   - surface: reference position needs to be within 45 NM of the actual
//!     position.
//!
//! <https://mode-s.org/1090mhz/content/ads-b/3-airborne-position.html>

use std::ops::Not;

pub use self::algorithm::Algorithm;
use crate::source::mode_s::VerticalStatus;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cpr {
    pub format: Format,
    pub position: PositionCode,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Format {
    Even,
    Odd,
}

impl Format {
    /// Returns the CPR format from the boolean value of the bit in the
    /// respective fields.
    pub fn from_bit(bit: bool) -> Self {
        if bit { Format::Odd } else { Format::Even }
    }

    /// The returned boolean corresponds to the value of the bit encoded in the
    /// frames.
    pub fn is_even(&self) -> bool {
        match self {
            Format::Even => false,
            Format::Odd => true,
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

impl Not for Format {
    type Output = Self;

    fn not(self) -> Self::Output {
        self.other()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PositionCode {
    pub latitude: CoodinateCode,
    pub longitude: CoodinateCode,
}

/// 17 bit encoded latitude/longitude
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CoodinateCode(u32);

impl CoodinateCode {
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
pub enum DecodeError {
    #[error("messages must be from the same longitude zone")]
    MessagesFromDifferentLongitudeZones { nl_lat_even: f64, nl_lat_odd: f64 },
}

mod algorithm {
    use std::f64::consts::{
        FRAC_PI_2,
        PI,
        TAU,
    };

    use super::{
        CoodinateCode,
        Cpr,
        DecodeError,
        Format,
        Position,
        PositionCode,
    };

    const N_Z: f64 = 15.0;
    const D_LAT_EVEN: f64 = 360.0 / (4.0 * N_Z);
    const D_LAT_ODD: f64 = 360.0 / (4.0 * N_Z - 1.0);

    // floor(x) = x.floor()
    // mod(x, y) = x.rem_euclid(y)
    // arccos(x) = x.acos()

    // note: MOPS says this equation is too slow for real-time. it is fast enough
    // for us lol
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
            let b = (PI * lat.abs() / 180.0).cos().powi(2);
            (TAU / (1.0 - a / b).acos()).floor()
        }
    }

    #[inline(always)]
    fn fix_lat(mut lat: f64) -> f64 {
        if lat >= 270.0 {
            lat -= 360.0;
        }
        lat
    }

    #[inline(always)]
    fn fix_lon(mut lon: f64) -> f64 {
        if lon >= 180.0 {
            lon -= 360.0;
        }
        lon
    }

    #[inline(always)]
    fn i(format: Format) -> f64 {
        match format {
            Format::Even => 0.0,
            Format::Odd => 1.0,
        }
    }

    /// Algorithm for encoding and decoding CPR positions
    ///
    /// A.1.7, page A-55 (905)
    #[derive(Clone, Copy, Debug)]
    pub struct Algorithm {
        /// Number of bits used to encode a position coordinate
        pub nb: u8,

        /// `D_lat`/`D_lon` factor. `1.0` for airborne, `0.25` for latitude
        pub d_factor: f64,
    }

    impl Algorithm {
        pub const AIRBORNE: Self = Self {
            nb: 17,
            d_factor: 1.0,
        };

        pub const SURFACE: Self = Self {
            nb: 19,
            d_factor: 0.25,
        };

        /// # Note
        ///
        /// This only uses [`Format::Even`].
        pub const INTENT: Self = Self {
            nb: 14,
            d_factor: 1.0,
        };

        pub const TISB_COARSE_AIRBORNE: Self = Self {
            nb: 12,
            d_factor: 1.0,
        };
    }

    impl Algorithm {
        #[inline(always)]
        fn pow_2_nb(&self) -> f64 {
            2.0f64.powi(self.nb.into())
        }

        #[inline(always)]
        fn cpr_position_to_yz_xz_scaled(&self, position: PositionCode) -> [f64; 2] {
            let pow_2_nb = self.pow_2_nb();
            let yz = position.latitude.0 as f64;
            let xz = position.longitude.0 as f64;
            [yz / pow_2_nb, xz / pow_2_nb]
        }

        // todo: test and make public
        // note: for Self::INTENT only CprFormat::EVEN is used
        pub(super) fn encode(&self, position: Position, format: Format) -> PositionCode {
            let lat = position.latitude;
            let lon = position.longitude;

            let i = i(format);
            // MOPS doesn't scale D_lat and D_lon differently for surface?
            let d_lat = 360.0 / (4.0 * N_Z - i);

            let pow_2_nb = self.pow_2_nb();
            let yz = (pow_2_nb * lat.rem_euclid(d_lat) / d_lat + 0.5).floor();
            let r_lat = d_lat * (yz / pow_2_nb + (lat / d_lat).floor());

            // MOPS doesn't scale D_lat and D_lon differently for surface?
            let d_lon = 360.0 / (n_l(r_lat) - i).max(1.0);

            let xz = (pow_2_nb * lon.rem_euclid(d_lon) / d_lon + 0.5).floor();

            // does this work? is there a better way?
            let yz = CoodinateCode(yz.rem_euclid(pow_2_nb) as u32);
            let xz = CoodinateCode(xz.rem_euclid(pow_2_nb) as u32);

            PositionCode {
                latitude: yz,
                longitude: xz,
            }
        }

        /// Decode an a single CPR using a reference position.
        ///
        /// This will always work, but the reference position must be close to
        /// the actual position (see module documentation).
        pub fn decode_local(&self, field: Cpr, reference_position: Position) -> Position {
            let i = i(field.format);

            let lat_s = reference_position.latitude;
            let lon_s = reference_position.longitude;

            let [yz, xz] = self.cpr_position_to_yz_xz_scaled(field.position);

            let d_lat = self.d_factor * 360.0 / (4.0 * N_Z - i);

            // latitude zone index
            let j = (lat_s / d_lat).floor() + (0.5 + lat_s.rem_euclid(d_lat) / d_lat - yz).floor();

            let r_lat = d_lat * (j + yz);

            let d_lon = 360.0 / (n_l(r_lat) - i).max(1.0);

            // longitude zone index
            let m = (lon_s / d_lon).floor() + (0.5 + lon_s.rem_euclid(d_lon) / d_lon - xz).floor();

            let r_lon = d_lon * (m + xz);
            let r_lon = fix_lon(r_lon);

            Position {
                latitude: r_lat,
                longitude: r_lon,
            }
        }

        /// Decode an even and and odd CPR.
        ///
        /// This might fail if the CPRs are from different zones. If you don't
        /// have both CPRs or if this function fails, you can use
        /// [`decode_local`][Self::decode_local].
        pub fn decode_global(
            &self,
            cpr_even: PositionCode,
            cpr_odd: PositionCode,
            most_recent: Format,
        ) -> Result<Position, DecodeError> {
            let [yz_even, xz_even] = self.cpr_position_to_yz_xz_scaled(cpr_even);
            let [yz_odd, xz_odd] = self.cpr_position_to_yz_xz_scaled(cpr_odd);

            let d_lat_even = self.d_factor * 360.0 / (4.0 * N_Z);
            let d_lat_odd = self.d_factor * 360.0 / (4.0 * N_Z - 1.0);

            // latitude zone index
            let j = (59.0 * yz_even - 60.0 * yz_odd + 0.5).floor();

            let r_lat_even = d_lat_even * (j.rem_euclid(60.0) + yz_even);
            let r_lat_odd = d_lat_odd * (j.rem_euclid(59.0) + yz_odd);

            let r_lat_even = fix_lat(r_lat_even);
            let r_lat_odd = fix_lat(r_lat_odd);

            let nl_r_lat_even = n_l(r_lat_even);
            let nl_r_lat_odd = n_l(r_lat_odd);

            // nl is a whole number and we only use floats for convenience. the value is
            // floored though, so using `==` should be fine.
            if nl_r_lat_even != nl_r_lat_odd {
                return Err(DecodeError::MessagesFromDifferentLongitudeZones {
                    nl_lat_even: nl_r_lat_even,
                    nl_lat_odd: nl_r_lat_odd,
                });
            }

            // select most recent
            let (r_lat, nl_r_lat, xz, n) = match most_recent {
                Format::Even => (r_lat_even, nl_r_lat_even, xz_even, nl_r_lat_even.max(1.0)),
                Format::Odd => {
                    (
                        r_lat_odd,
                        nl_r_lat_odd,
                        xz_odd,
                        (nl_r_lat_odd - 1.0).max(1.0),
                    )
                }
            };

            let d_lon = 360.0 / n;

            // longitude index
            let m = (xz_even * (nl_r_lat - 1.0) - xz_odd * nl_r_lat + 0.5).floor();

            let r_lon = d_lon * (m.rem_euclid(n) + xz);
            let r_lon = fix_lon(r_lon);

            Ok(Position {
                latitude: r_lat,
                longitude: r_lon,
            })
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct DecoderBin<T> {
    vertical_status: VerticalStatus,
    position: PositionCode,
    time: T,
}

/// CPR decoder
///
/// This is generic over the type of time you use. All `T` needs to support is
/// comparisions (i.e [`Ord`]).
#[derive(Clone, Copy, Debug, Default)]
pub struct Decoder<T> {
    even: Option<DecoderBin<T>>,
    odd: Option<DecoderBin<T>>,
}

impl<T: Ord> Decoder<T> {
    /// Push a CPR value into the decoder.
    ///
    /// This buffers the CPR value and tries to decode it. It will first try to
    /// decode it globally with a buffered CPR value. If that fails, it will
    /// decode it with a reference position, if available.
    ///
    /// The vertical status needs to be provided because the decoding algorithm
    /// depends on it.
    ///
    /// The provided local reference needs to be close to the actual position
    /// (see module documentation).
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
            Format::Even => (&mut self.even, &self.odd),
            Format::Odd => (&mut self.odd, &self.even),
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

        let algorithm = match vertical_status {
            VerticalStatus::Airborne => &Algorithm::AIRBORNE,
            VerticalStatus::Ground => &Algorithm::SURFACE,
        };

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
                        Format::Even => (cpr.position, other_bin.position),
                        Format::Odd => (other_bin.position, cpr.position),
                    };

                    algorithm.decode_global(even, odd, most_recent).ok()
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
                    algorithm.decode_local(cpr, reference)
                })
            })
    }
}

#[cfg(test)]
mod tests {
    use approx::assert_abs_diff_eq;

    use super::{
        Algorithm,
        CoodinateCode,
        Cpr,
        Format,
        Position,
        PositionCode,
    };

    const EXAMPLE_EVEN: PositionCode = PositionCode {
        latitude: CoodinateCode::from_u32_unchecked(0b10110101101001000),
        longitude: CoodinateCode::from_u32_unchecked(0b01100100010101100),
    };
    const EXAMPLE_ODD: PositionCode = PositionCode {
        latitude: CoodinateCode::from_u32_unchecked(0b10010000110101110),
        longitude: CoodinateCode::from_u32_unchecked(0b01100010000010010),
    };
    const EXAMPLE_POSITION: Position = Position {
        latitude: 52.2572,
        longitude: 3.91937,
    };
    const EXAMPLE_REFERENCE: Position = Position {
        latitude: 52.258,
        longitude: 3.918,
    };

    #[test]
    fn decode_globally_unambigious_decoding_example() {
        let position = Algorithm::AIRBORNE
            .decode_global(EXAMPLE_EVEN, EXAMPLE_ODD, Format::Even)
            .unwrap();

        assert_abs_diff_eq!(
            position.latitude,
            EXAMPLE_POSITION.latitude,
            epsilon = 0.001
        );
        assert_abs_diff_eq!(
            position.longitude,
            EXAMPLE_POSITION.longitude,
            epsilon = 0.001
        );
    }

    #[test]
    fn decode_locally_umambiguous_decoding_example() {
        let cpr = Cpr {
            format: Format::Even,
            position: EXAMPLE_EVEN,
        };

        let position = Algorithm::AIRBORNE.decode_local(cpr, EXAMPLE_REFERENCE);

        assert_abs_diff_eq!(
            position.latitude,
            EXAMPLE_POSITION.latitude,
            epsilon = 0.001
        );
        assert_abs_diff_eq!(
            position.longitude,
            EXAMPLE_POSITION.longitude,
            epsilon = 0.001
        );
    }

    const P1: Position = Position {
        latitude: 48.729381,
        longitude: 2.916458,
    };
    const P2: Position = Position {
        latitude: 48.715478,
        longitude: 2.943659,
    };

    #[test]
    fn global_round_trip_airborne() {
        let cpr_even = Algorithm::AIRBORNE.encode(P1, Format::Even);
        let cpr_odd = Algorithm::AIRBORNE.encode(P2, Format::Odd);

        let position = Algorithm::AIRBORNE
            .decode_global(cpr_even, cpr_odd, Format::Odd)
            .unwrap();
        assert_abs_diff_eq!(position.latitude, P2.latitude, epsilon = 0.001);
        assert_abs_diff_eq!(position.longitude, P2.longitude, epsilon = 0.001);
    }

    fn local_round_trip_airborne() {
        let position = Algorithm::AIRBORNE.encode(P1, Format::Even);
        let position = Algorithm::AIRBORNE.decode_local(
            Cpr {
                format: Format::Even,
                position,
            },
            P2,
        );
        assert_abs_diff_eq!(position.latitude, P1.latitude, epsilon = 0.001);
        assert_abs_diff_eq!(position.longitude, P1.longitude, epsilon = 0.001);
    }
}
