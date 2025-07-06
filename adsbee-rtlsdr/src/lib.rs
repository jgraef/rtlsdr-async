mod bindings;
#[cfg(feature = "command")]
pub mod command;
pub mod demodulator;
#[cfg(feature = "tcp")]
pub mod tcp;
pub(crate) mod util;

use std::{
    fmt::Debug,
    pin::Pin,
    task::{
        Context,
        Poll,
    },
};

pub use bindings::{
    DeviceInfo,
    DeviceIter,
    RtlSdr,
    devices,
};
use bytemuck::{
    Pod,
    Zeroable,
};
use pin_project_lite::pin_project;

/// 16 bit IQ sample
///
/// 8 bits per component, mapped from [-128, 127] to [0, 255]
#[derive(Clone, Copy, Pod, Zeroable)]
#[repr(C)]
pub struct IqSample {
    /// I: in-phase / real component
    pub i: u8,
    /// Q: quadrature / imaginary component
    pub q: u8,
}

impl Default for IqSample {
    fn default() -> Self {
        Self { i: 128, q: 128 }
    }
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

impl<T: ?Sized + AsyncReadSamples + Unpin> AsyncReadSamples for &mut T {
    type Error = <T as AsyncReadSamples>::Error;

    fn poll_read_samples(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &mut [IqSample],
    ) -> Poll<Result<usize, Self::Error>> {
        Pin::new(&mut **self).poll_read_samples(cx, buffer)
    }
}

pub trait AsyncReadSamplesExt: AsyncReadSamples {
    fn read_samples<'a>(&'a mut self, buffer: &'a mut [IqSample]) -> ReadSamples<'a, Self>
    where
        Self: Unpin,
    {
        ReadSamples {
            stream: self,
            buffer,
        }
    }

    fn map_err<E, F>(self, f: F) -> MapErr<Self, F>
    where
        F: FnMut(Self::Error) -> E,
        Self: Sized,
    {
        MapErr {
            inner: self,
            map_err: f,
        }
    }
}

impl<T: AsyncReadSamples> AsyncReadSamplesExt for T {}

pub struct ReadSamples<'a, S: ?Sized> {
    stream: &'a mut S,
    buffer: &'a mut [IqSample],
}

impl<'a, 'b, S: AsyncReadSamples + Unpin + ?Sized> Future for ReadSamples<'a, S> {
    type Output = Result<usize, S::Error>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = &mut *self;
        Pin::new(&mut *this.stream).poll_read_samples(cx, this.buffer)
    }
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
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + Sync;

    /// Set sample rate in Hz
    fn set_sample_rate(
        &mut self,
        sample_rate: u32,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + Sync;

    /// Set tuner gain, in tenths of a dB
    fn set_tuner_gain(
        &mut self,
        gain: Gain,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + Sync;

    /// Set the automatic gain correction, a software step to correct the
    /// incoming signal, this is not automatic gain control on the hardware
    /// chip, that is controlled by tuner gain mode.
    fn set_agc_mode(
        &mut self,
        enable: bool,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + Sync;

    fn set_frequency_correction(
        &mut self,
        ppm: i32,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + Sync;

    fn set_tuner_if_gain(
        &mut self,
        stage: i16,
        gain: i16,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + Sync;

    fn set_offset_tuning(
        &mut self,
        enable: bool,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + Sync;

    fn set_rtl_xtal(
        &mut self,
        frequency: u32,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + Sync;

    fn set_tuner_xtal(
        &mut self,
        frequency: u32,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + Sync;

    fn set_bias_tee(
        &mut self,
        enable: bool,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + Sync;
}

impl<T: ?Sized + Unpin + Configure> Configure for &mut T {
    type Error = <T as Configure>::Error;

    fn set_center_frequency(
        &mut self,
        frequency: u32,
    ) -> impl Future<Output = Result<(), Self::Error>> {
        T::set_center_frequency(*self, frequency)
    }

    fn set_sample_rate(
        &mut self,
        sample_rate: u32,
    ) -> impl Future<Output = Result<(), Self::Error>> {
        T::set_sample_rate(*self, sample_rate)
    }

    fn set_tuner_gain(&mut self, gain: Gain) -> impl Future<Output = Result<(), Self::Error>> {
        T::set_tuner_gain(*self, gain)
    }

    fn set_agc_mode(&mut self, enable: bool) -> impl Future<Output = Result<(), Self::Error>> {
        T::set_agc_mode(*self, enable)
    }

    fn set_frequency_correction(
        &mut self,
        ppm: i32,
    ) -> impl Future<Output = Result<(), Self::Error>> {
        T::set_frequency_correction(self, ppm)
    }

    fn set_tuner_if_gain(
        &mut self,
        stage: i16,
        gain: i16,
    ) -> impl Future<Output = Result<(), Self::Error>> {
        T::set_tuner_if_gain(self, stage, gain)
    }

    fn set_offset_tuning(&mut self, enable: bool) -> impl Future<Output = Result<(), Self::Error>> {
        T::set_offset_tuning(self, enable)
    }

    fn set_rtl_xtal(&mut self, frequency: u32) -> impl Future<Output = Result<(), Self::Error>> {
        T::set_rtl_xtal(self, frequency)
    }

    fn set_tuner_xtal(&mut self, frequency: u32) -> impl Future<Output = Result<(), Self::Error>> {
        T::set_tuner_xtal(self, frequency)
    }

    fn set_bias_tee(&mut self, enable: bool) -> impl Future<Output = Result<(), Self::Error>> {
        T::set_bias_tee(self, enable)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gain {
    /// Gain tenths of a dB
    Manual(i32),
    /// Auto gain control
    Auto,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TunerGainMode {
    Manual,
    Auto,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DirectSamplingMode {
    I,
    Q,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TunerType(pub u32);

impl TunerType {
    pub const UNKNOWN: Self = Self(0);
    pub const E4000: Self = Self(1);
    pub const FC0012: Self = Self(2);
    pub const FC0013: Self = Self(3);
    pub const FC2580: Self = Self(4);
    pub const R820T: Self = Self(5);
    pub const R828D: Self = Self(6);
}

impl TunerType {
    pub fn is_r82xx(&self) -> bool {
        matches!(*self, TunerType::R828D | TunerType::R820T)
    }
}

impl Debug for TunerType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            Self::UNKNOWN => write!(f, "TunerType::UNKNOWN"),
            Self::E4000 => write!(f, "TunerType::E4000"),
            Self::FC0012 => write!(f, "TunerType::FC0012"),
            Self::FC0013 => write!(f, "TunerType::FC0013"),
            Self::FC2580 => write!(f, "TunerType::FC2580"),
            Self::R820T => write!(f, "TunerType::R820T"),
            Self::R828D => write!(f, "TunerType::R828D"),
            _ => write!(f, "TunerType({})", self.0),
        }
    }
}
