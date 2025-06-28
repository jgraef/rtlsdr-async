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
            Self::ModeA(ModeA::decode(data))
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ModeA {
    pub squawk: Squawk,
    pub ident: bool,
}

impl ModeA {
    pub fn decode(data: [u8; 2]) -> Self {
        // Mode A packet:
        // bit:    f e d c b a 9 8 7 6 5 4 3 2 1
        // squawk: a a a 0 b b b 0 c c c 0 d d d -> aaabbbcccddd
        // ident:  0 0 0 0 0 0 0 1 0 0 0 0 0 0 0

        // todo: is this big-endian?
        let word = u16::from_be_bytes(data);

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
        todo!();
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ModeCDecodeError {}
