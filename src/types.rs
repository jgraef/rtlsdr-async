use std::{
    fmt::{
        Debug,
        Display,
    },
    str::FromStr,
};

use serde::{
    Deserialize,
    Serialize,
};

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct IcaoAddress {
    address: u32,
    non_icao: bool, // todo: how should this be handled correctly?
}

impl IcaoAddress {
    pub const fn from_u32_unchecked(address: u32) -> Self {
        Self {
            address,
            non_icao: false,
        }
    }

    pub fn from_u32(address: u32) -> Option<Self> {
        (address < 0x1000000).then(|| Self::from_u32_unchecked(address))
    }

    pub const fn with_non_icao_flag(self) -> Self {
        Self {
            address: self.address,
            non_icao: true,
        }
    }

    pub fn non_icao(&self) -> bool {
        self.non_icao
    }

    pub fn as_bytes(&self) -> [u8; 3] {
        let b = self.address.to_be_bytes();
        assert!(b[0] == 0);
        [b[1], b[2], b[3]]
    }
}

impl Display for IcaoAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.non_icao {
            write!(f, "~")?;
        }
        write!(f, "{:06x}", self.address)
    }
}

impl Debug for IcaoAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "IcaoAddress({self})")
    }
}

impl FromStr for IcaoAddress {
    type Err = IcaoAddressFromStrError;

    fn from_str(mut s: &str) -> Result<Self, Self::Err> {
        let err = || {
            IcaoAddressFromStrError {
                input: s.to_owned(),
            }
        };
        let mut non_icao = false;
        if s.starts_with('~') {
            non_icao = true;
            s = &s[1..];
        }

        let address = u32::from_str_radix(s, 16).map_err(|_| err())?;
        let mut address = Self::from_u32(address).ok_or_else(err)?;
        address.non_icao = non_icao;
        Ok(address)
    }
}

#[derive(Clone, Debug, thiserror::Error)]
#[error("Invalid ICAO address: {input}")]
pub struct IcaoAddressFromStrError {
    pub input: String,
}

impl From<IcaoAddress> for u32 {
    fn from(value: IcaoAddress) -> Self {
        value.address
    }
}

impl<DB: sqlx::Database> sqlx::Type<DB> for IcaoAddress
where
    i32: sqlx::Type<DB>,
{
    fn type_info() -> DB::TypeInfo {
        <i32 as sqlx::Type<DB>>::type_info()
    }
}

impl<'q, DB: sqlx::Database> sqlx::Encode<'q, DB> for IcaoAddress
where
    i32: sqlx::Encode<'q, DB>,
{
    fn encode_by_ref(
        &self,
        buf: &mut <DB as sqlx::Database>::ArgumentBuffer<'q>,
    ) -> Result<sqlx::encode::IsNull, sqlx::error::BoxDynError> {
        let mut address = self.address as i32;
        if self.non_icao {
            address = -address;
        }
        <i32 as sqlx::Encode<DB>>::encode_by_ref(&(address as i32), buf)
    }
}

impl<'r, DB: sqlx::Database> sqlx::Decode<'r, DB> for IcaoAddress
where
    i32: sqlx::Decode<'r, DB>,
{
    fn decode(
        value: <DB as sqlx::Database>::ValueRef<'r>,
    ) -> Result<Self, sqlx::error::BoxDynError> {
        let address = <i32 as sqlx::Decode<DB>>::decode(value)?;
        let non_icao = address < 0;
        let address = address.abs() as u32;
        Ok(Self { address, non_icao })
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Squawk {
    code: u16,
}

impl Squawk {
    const VFR_STANDARD: Self = Self::from_u16_unchecked(0700);
    const AIRCRAFT_HIJACKING: Self = Self::from_u16_unchecked(07500);
    const RADIO_FAILURE: Self = Self::from_u16_unchecked(07600);
    const EMERGENCY: Self = Self::from_u16_unchecked(07700);

    pub const fn from_u16_unchecked(code: u16) -> Self {
        Self { code }
    }

    pub fn from_u16(code: u16) -> Option<Self> {
        (code < 010000).then(|| Self::from_u16_unchecked(code))
    }
}

impl Display for Squawk {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:04o}", self.code)
    }
}

impl Debug for Squawk {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Squawk({:04o})", self.code)
    }
}

impl FromStr for Squawk {
    type Err = SquawkFromStrError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let err = || {
            SquawkFromStrError {
                input: s.to_owned(),
            }
        };
        let code = u16::from_str_radix(s, 8).map_err(|_| err())?;
        Self::from_u16(code).ok_or_else(err)
    }
}

#[derive(Clone, Debug, thiserror::Error)]
#[error("Invalid squawk code: {input}")]
pub struct SquawkFromStrError {
    pub input: String,
}

impl From<Squawk> for u16 {
    fn from(value: Squawk) -> Self {
        value.code
    }
}

impl<DB: sqlx::Database> sqlx::Type<DB> for Squawk
where
    i16: sqlx::Type<DB>,
{
    fn type_info() -> DB::TypeInfo {
        <i16 as sqlx::Type<DB>>::type_info()
    }
}

impl<'q, DB: sqlx::Database> sqlx::Encode<'q, DB> for Squawk
where
    i16: sqlx::Encode<'q, DB>,
{
    fn encode_by_ref(
        &self,
        buf: &mut <DB as sqlx::Database>::ArgumentBuffer<'q>,
    ) -> Result<sqlx::encode::IsNull, sqlx::error::BoxDynError> {
        <i16 as sqlx::Encode<DB>>::encode_by_ref(&(self.code as i16), buf)
    }
}

impl<'r, DB: sqlx::Database> sqlx::Decode<'r, DB> for Squawk
where
    i16: sqlx::Decode<'r, DB>,
{
    fn decode(
        value: <DB as sqlx::Database>::ValueRef<'r>,
    ) -> Result<Self, sqlx::error::BoxDynError> {
        let code = <i16 as sqlx::Decode<DB>>::decode(value)?;
        Ok(Self::from_u16_unchecked(code as u16))
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Wtc {
    #[serde(rename = "L")]
    Light,
    #[serde(rename = "M")]
    Medium,
    #[serde(rename = "H")]
    Heavy,
    #[serde(rename = "J")]
    Super,
}

impl Wtc {
    pub fn as_char(&self) -> char {
        match self {
            Wtc::Light => 'L',
            Wtc::Medium => 'M',
            Wtc::Heavy => 'H',
            Wtc::Super => 'J',
        }
    }

    pub fn from_char(c: char) -> Option<Self> {
        match c {
            'L' | 'l' => Some(Self::Light),
            'M' | 'm' => Some(Self::Medium),
            'H' | 'h' => Some(Self::Heavy),
            'J' | 'j' => Some(Self::Super),
            _ => None,
        }
    }
}

impl Display for Wtc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_char())
    }
}

impl Debug for Wtc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Wtc({})", self.as_char())
    }
}

impl FromStr for Wtc {
    type Err = WtcFromStrError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let err = || {
            WtcFromStrError {
                input: s.to_owned(),
            }
        };
        let mut chars = s.chars();
        let c = chars.next().ok_or_else(err)?;
        if chars.next().is_some() {
            return Err(err());
        }
        Self::from_char(c).ok_or_else(err)
    }
}

#[derive(Clone, Debug, thiserror::Error)]
#[error("Invalid WTC: {input}")]
pub struct WtcFromStrError {
    pub input: String,
}

impl From<Wtc> for char {
    fn from(value: Wtc) -> Self {
        value.as_char()
    }
}

impl TryFrom<char> for Wtc {
    type Error = ();

    fn try_from(value: char) -> Result<Self, Self::Error> {
        Self::from_char(value).ok_or(())
    }
}

impl<DB: sqlx::Database> sqlx::Type<DB> for Wtc
where
    i8: sqlx::Type<DB>,
{
    fn type_info() -> DB::TypeInfo {
        <i8 as sqlx::Type<DB>>::type_info()
    }
}

impl<'q, DB: sqlx::Database> sqlx::Encode<'q, DB> for Wtc
where
    i8: sqlx::Encode<'q, DB>,
{
    fn encode_by_ref(
        &self,
        buf: &mut <DB as sqlx::Database>::ArgumentBuffer<'q>,
    ) -> Result<sqlx::encode::IsNull, sqlx::error::BoxDynError> {
        <i8 as sqlx::Encode<DB>>::encode_by_ref(&(self.as_char() as i8), buf)
    }
}

impl<'r, DB: sqlx::Database> sqlx::Decode<'r, DB> for Wtc
where
    i8: sqlx::Decode<'r, DB>,
{
    fn decode(
        value: <DB as sqlx::Database>::ValueRef<'r>,
    ) -> Result<Self, sqlx::error::BoxDynError> {
        let c = <i8 as sqlx::Decode<DB>>::decode(value)?;
        Ok(Self::from_char(c as u8 as char).unwrap_or_else(|| panic!("invalid wtc: {c}")))
    }
}
