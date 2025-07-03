use std::{
    pin::Pin,
    task::{
        Context,
        Poll,
    },
};

use bytemuck::{
    Pod,
    Zeroable,
};

pub mod command;
pub mod demodulator;
pub mod tcp;

//const INPUT_BUFFER_SIZE: usize = 0x800000; // 8 KiB

/// Sample rate: 2 samples/µs
pub const SAMPLE_RATE: u32 = 2_000_000;

/// Mode S downlink frequency: 1090 MHz
pub const DOWNLINK_FREQUENCY: u32 = 1_090_000_000;

/// Mode S uplink frequency: 1030 MHz
pub const UPLINK_FREQUENCY: u32 = 1_030_000_000;

/// 16 bit IQ sample
///
/// 8 bits per component.
#[derive(Clone, Copy, Pod, Zeroable)]
#[repr(C)]
pub struct IqSample {
    /// I: in-phase / real component
    pub i: u8,
    /// Q: quadrature / imaginary component
    pub q: u8,
}

impl IqSample {
    pub fn magnitude(&self) -> u16 {
        #[inline(always)]
        fn abs(x: u8) -> u8 {
            if x >= 127 { x - 127 } else { 127 - x }
        }

        #[inline(always)]
        fn square(x: u8) -> u16 {
            let x = u16::from(abs(x));
            x * x
        }

        square(self.i) + square(self.q)
    }
}

#[derive(Clone, Copy, Debug)]
pub enum RawFrame {
    ModeAc { data: [u8; 2] },
    ModeSShort { data: [u8; 7] },
    ModeSLong { data: [u8; 14] },
}

pub trait AsyncReadSamples {
    type Error;

    fn poll_read_samples(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &mut [IqSample],
    ) -> Poll<Result<usize, Self::Error>>;
}

pub type Magnitude = u16;

#[derive(Clone, Copy, Debug)]
pub struct Cursor<'a> {
    pub samples: &'a [Magnitude],
    pub position: usize,
}

impl<'a> Cursor<'a> {
    #[inline(always)]
    pub fn advance(&mut self, amount: usize) {
        self.position += amount;
    }

    #[inline(always)]
    pub fn advance_to_end(&mut self) {
        self.position = self.samples.len();
    }

    #[inline(always)]
    pub fn remaining(&self) -> &[Magnitude] {
        &self.samples[self.position..]
    }
}
