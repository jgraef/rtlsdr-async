pub use crate::source::mode_s::cpr::decode::decode_globally_unambigious;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CprFormat {
    Even,
    Odd,
}

impl CprFormat {
    pub fn from_bool(bit: bool) -> Self {
        if bit { CprFormat::Odd } else { CprFormat::Even }
    }

    pub fn as_bool(&self) -> bool {
        match self {
            CprFormat::Even => false,
            CprFormat::Odd => true,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cpr {
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
pub struct DecodedPosition {
    pub latitude: f64,
    pub longitude: f64,
}

#[derive(Debug, thiserror::Error)]
pub enum CprDecodeError {
    #[error("messages must be from the same longitude zone")]
    MessagesFromDifferentLongitudeZones { nl_lat_even: f64, nl_lat_odd: f64 },
}

mod decode {
    //! https://mode-s.org/1090mhz/content/ads-b/3-airborne-position.html

    use std::f64::consts::{
        FRAC_PI_2,
        PI,
        TAU,
    };

    use crate::source::mode_s::cpr::{
        Cpr,
        CprDecodeError,
        CprFormat,
        CprValue,
        DecodedPosition,
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
    fn cpr(x: CprValue) -> f64 {
        (x.0 as f64) / 131072.0
    }

    #[inline(always)]
    fn adjust_lat(lat: &mut f64) {
        if *lat >= 270.0 {
            *lat -= 360.0;
        }
    }

    pub fn decode_globally_unambigious(
        cpr_even: Cpr,
        cpr_odd: Cpr,
        most_recent: CprFormat,
    ) -> Result<DecodedPosition, CprDecodeError> {
        let lat_cpr_even = cpr(cpr_even.latitude);
        let lat_cpr_odd = cpr(cpr_odd.latitude);

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

        let lon_cpr_even = cpr(cpr_even.longitude);
        let lon_cpr_odd = cpr(cpr_odd.longitude);

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

        Ok(DecodedPosition {
            latitude: lat,
            longitude: lon,
        })
    }
}

#[cfg(test)]
mod tests {
    use approx::assert_abs_diff_eq;

    use crate::source::mode_s::cpr::{
        Cpr,
        CprFormat,
        CprValue,
        DecodedPosition,
        decode_globally_unambigious,
    };

    #[test]
    fn decode_globally_unambigious_decoding_example() {
        let cpr_even = Cpr {
            latitude: CprValue::from_u32_unchecked(0b10110101101001000),
            longitude: CprValue::from_u32_unchecked(0b01100100010101100),
        };
        let cpr_odd = Cpr {
            latitude: CprValue::from_u32_unchecked(0b10010000110101110),
            longitude: CprValue::from_u32_unchecked(0b01100010000010010),
        };

        let DecodedPosition {
            latitude,
            longitude,
        } = decode_globally_unambigious(cpr_even, cpr_odd, CprFormat::Even).unwrap();

        assert_abs_diff_eq!(latitude, 52.2572, epsilon = 0.001);
        assert_abs_diff_eq!(longitude, 3.91937, epsilon = 0.001);
    }
}
