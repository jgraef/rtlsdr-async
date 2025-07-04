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

pub mod flights;
pub mod live;
pub(crate) mod util;

#[cfg(feature = "sqlx")]
mod sqlx;

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

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Bbox {
    pub south_of: f32,
    pub north_of: f32,
    pub east_of: f32,
    pub west_of: f32,
}
