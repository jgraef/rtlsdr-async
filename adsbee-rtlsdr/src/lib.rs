mod bindings;
#[cfg(feature = "command")]
pub mod command;
pub mod demodulator;
pub mod tcp;
pub(crate) mod util;

use std::{
    pin::Pin,
    task::{
        Context,
        Poll,
    },
};

pub use bindings::{
    DeviceInfo,
    RtlSdr,
    list_devices,
};
use bytemuck::{
    Pod,
    Zeroable,
};
use pin_project_lite::pin_project;

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

pub trait AsyncReadSamples {
    type Error;

    fn poll_read_samples(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &mut [IqSample],
    ) -> Poll<Result<usize, Self::Error>>;
}

pub trait AsyncReadSamplesExt: AsyncReadSamples {
    fn map_err<E, F>(self, f: F) -> MapErr<Self, F>
    where
        F: FnMut(Self::Error) -> E,
        Self: Sized;
}

pin_project! {
    #[derive(Clone, Copy, Debug)]
    pub struct MapErr<S, F> {
        #[pin]
        inner: S,
        map_err: F,
    }
}

impl<S, E, F> AsyncReadSamples for MapErr<S, F>
where
    S: AsyncReadSamples,
    F: FnMut(S::Error) -> E,
{
    type Error = E;
    fn poll_read_samples(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &mut [IqSample],
    ) -> Poll<Result<usize, Self::Error>> {
        let this = self.project();
        this.inner
            .poll_read_samples(cx, buffer)
            .map_err(this.map_err)
    }
}

pub trait Configure {
    type Error;

    /// Set tuner frequency in Hz
    fn set_center_frequency(
        &mut self,
        frequency: u32,
    ) -> impl Future<Output = Result<(), Self::Error>>;

    /// Set sample rate in Hz
    fn set_sample_rate(
        &mut self,
        sample_rate: u32,
    ) -> impl Future<Output = Result<(), Self::Error>>;

    /// Set tuner gain, in tenths of a dB
    fn set_gain(&mut self, gain: Gain) -> impl Future<Output = Result<(), Self::Error>>;

    /// Set the automatic gain correction, a software step to correct the
    /// incoming signal, this is not automatic gain control on the hardware
    /// chip, that is controlled by tuner gain mode.
    fn set_agc_mode(&mut self, enabled: bool) -> impl Future<Output = Result<(), Self::Error>>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gain {
    /// Gain tenths of a dB
    Manual(u32),
    /// Auto gain control
    Auto,
}
