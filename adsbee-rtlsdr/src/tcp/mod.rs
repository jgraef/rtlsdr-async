use bytes::{
    Buf,
    BufMut,
};

use crate::{
    DirectSamplingMode,
    TunerGainMode,
    TunerType,
    util::BufReadBytesExt,
};

pub mod client;
pub mod server;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    SetCenterFrequency { frequency: u32 },
    SetSampleRate { sample_rate: u32 },
    SetTunerGainMode { mode: TunerGainMode },
    SetTunerGain { gain: i32 },
    SetFrequencyCorrection { ppm: i32 },
    SetTunerIfGain { stage: i16, gain: i16 },
    SetTestMode { enable: bool },
    SetAgcMode { enable: bool },
    SetDirectSampling { mode: Option<DirectSamplingMode> },
    SetOffsetTuning { enable: bool },
    SetRtlXtal { frequency: u32 },
    SetTunerXtal { frequency: u32 },
    SetTunerGainIndex { index: u32 },
    SetBiasT { enable: bool },
}

impl Command {
    pub fn decode<B: Buf>(mut buffer: B) -> Result<Self, InvalidCommand> {
        match buffer.get_u8() {
            0x01 => {
                Ok(Self::SetCenterFrequency {
                    frequency: buffer.get_u32(),
                })
            }
            0x02 => {
                Ok(Self::SetSampleRate {
                    sample_rate: buffer.get_u32(),
                })
            }
            0x03 => {
                Ok(Self::SetTunerGainMode {
                    mode: if buffer.get_u32() == 0 {
                        TunerGainMode::Auto
                    }
                    else {
                        TunerGainMode::Manual
                    },
                })
            }
            0x04 => {
                Ok(Self::SetTunerGain {
                    gain: buffer.get_i32(),
                })
            }
            0x05 => {
                Ok(Self::SetFrequencyCorrection {
                    ppm: buffer.get_i32(),
                })
            }
            0x06 => {
                Ok(Self::SetTunerIfGain {
                    stage: buffer.get_i16(),
                    gain: buffer.get_i16(),
                })
            }
            0x07 => {
                Ok(Self::SetTestMode {
                    enable: buffer.get_u32() != 0,
                })
            }
            0x08 => {
                Ok(Self::SetAgcMode {
                    enable: buffer.get_u32() != 0,
                })
            }
            0x09 => {
                Ok(Self::SetDirectSampling {
                    mode: match buffer.get_u32() {
                        1 => Some(DirectSamplingMode::I),
                        2 => Some(DirectSamplingMode::Q),
                        _ => None,
                    },
                })
            }
            0x0a => {
                Ok(Self::SetOffsetTuning {
                    enable: buffer.get_u32() != 0,
                })
            }
            0x0b => {
                Ok(Self::SetRtlXtal {
                    frequency: buffer.get_u32(),
                })
            }
            0x0c => {
                Ok(Self::SetTunerXtal {
                    frequency: buffer.get_u32(),
                })
            }
            0x0d => {
                Ok(Self::SetTunerGainIndex {
                    index: buffer.get_u32(),
                })
            }
            0x0e => {
                Ok(Self::SetBiasT {
                    enable: buffer.get_u32() != 0,
                })
            }
            command => {
                Err(InvalidCommand {
                    command,
                    arguments: buffer.get_bytes(),
                })
            }
        }
    }

    pub fn encode<B: BufMut>(&self, mut buffer: B) {
        match self {
            Self::SetCenterFrequency { frequency } => {
                buffer.put_u8(0x01);
                buffer.put_u32(*frequency);
            }
            Self::SetSampleRate { sample_rate } => {
                buffer.put_u8(0x02);
                buffer.put_u32(*sample_rate);
            }
            Self::SetTunerGainMode { mode } => {
                buffer.put_u8(0x03);
                buffer.put_u32(match mode {
                    TunerGainMode::Auto => 0,
                    TunerGainMode::Manual => 1,
                });
            }
            Self::SetTunerGain { gain } => {
                buffer.put_u8(0x04);
                buffer.put_i32(*gain);
            }
            Self::SetFrequencyCorrection { ppm } => {
                buffer.put_u8(0x05);
                buffer.put_i32(*ppm);
            }
            Self::SetTunerIfGain { stage, gain } => {
                buffer.put_u8(0x06);
                buffer.put_i16(*stage);
                buffer.put_i16(*gain);
            }
            Self::SetTestMode { enable } => {
                buffer.put_u8(0x07);
                buffer.put_u32(*enable as u32);
            }
            Self::SetAgcMode { enable } => {
                buffer.put_u8(0x08);
                buffer.put_u32(*enable as u32);
            }
            Self::SetDirectSampling { mode } => {
                buffer.put_u8(0x09);
                buffer.put_u32(match mode {
                    None => 0,
                    Some(DirectSamplingMode::I) => 1,
                    Some(DirectSamplingMode::Q) => 2,
                });
            }
            Self::SetOffsetTuning { enable } => {
                buffer.put_u8(0x0a);
                buffer.put_u32(*enable as u32);
            }
            Self::SetRtlXtal { frequency } => {
                buffer.put_u8(0x0b);
                buffer.put_u32(*frequency);
            }
            Self::SetTunerXtal { frequency } => {
                buffer.put_u8(0x0c);
                buffer.put_u32(*frequency);
            }
            Self::SetTunerGainIndex { index } => {
                buffer.put_u8(0x0d);
                buffer.put_u32(*index);
            }
            Self::SetBiasT { enable } => {
                buffer.put_u8(0x0e);
                buffer.put_u32(*enable as u32);
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, thiserror::Error)]
#[error("Invalid rtl_tcp command: 0x{command:02} (arguments: {arguments:?})")]
pub struct InvalidCommand {
    pub command: u8,
    pub arguments: [u8; 4],
}

#[derive(Clone, Copy, Debug)]
pub struct DongleInfo {
    pub tuner_type: TunerType,
    pub tuner_gain_type: u32,
}

pub const HEADER_LENGTH: usize = 12;
pub const COMMAND_LENGTH: usize = 5;
pub const MAGIC: &'static [u8; 4] = b"RTL0";
