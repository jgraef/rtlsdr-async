//! <http://www.aeroelectric.com/articles/Altitude_Encoding/modec.htm>
//!
//! # TODO
//!
//! This seems to be the same as `super::mode_s::{IdentityCode, AltitudeCode}`,
//! but we need to test this. Unfortunately the mode A frames in our test data
//! are all zeros.

use adsb_index_api_types::Squawk;

#[derive(Clone, Copy, Debug)]
pub enum ModeAc {
    ModeA(ModeA),
    ModeC(ModeC),
}

impl ModeAc {
    pub fn decode(data: [u8; 2]) -> Self {
        if let Ok(mode_c) = ModeC::decode(data) {
            Self::ModeC(mode_c)
        }
        else {
            Self::ModeA(ModeA::from_bytes(data))
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ModeA {
    pub squawk: Squawk,
    pub ident: bool,
}

impl ModeA {
    pub fn from_bytes(data: [u8; 2]) -> Self {
        // todo: is this big-endian? does this use this "gillham" code?
        Self::from_u16(u16::from_be_bytes(data))
    }

    /// Decode squawk and ident flag from 16bit word.
    pub fn from_u16(word: u16) -> Self {
        // fixme: this is not correct! the bits need to be jumbled lol

        // Mode A:
        // bit:    f e d c b a 9 8 7 6 5 4 3 2 1
        // squawk: a a a 0 b b b 0 c c c 0 d d d -> aaabbbcccddd
        // ident:  0 0 0 0 0 0 0 1 0 0 0 0 0 0 0

        let squawk = ((word & 0x7000) >> 3)
            | ((word & 0x0700) >> 2)
            | ((word & 0x0070) >> 1)
            | (word & 0x0007);
        let ident = word & 0x0080 != 0;

        ModeA {
            squawk: Squawk::from_u16_unchecked(squawk),
            ident,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ModeC {}

impl ModeC {
    pub fn decode(_data: [u8; 2]) -> Result<ModeC, ModeCDecodeError> {
        // look at https://mode-s.org/1090mhz/content/mode-s/3-surveillance.html # Altitude reply
        todo!();
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ModeCDecodeError {}
