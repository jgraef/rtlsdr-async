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
/// 8 bits per component.
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

#[derive(Clone, Copy, Debug)]
pub enum RawFrame {
    ModeAc { data: [u8; 2] },
    ModeSShort { data: [u8; 7] },
    ModeSLong { data: [u8; 14] },
}

impl AsRef<[u8]> for RawFrame {
    fn as_ref(&self) -> &[u8] {
        match self {
            RawFrame::ModeAc { data } => &data[..],
            RawFrame::ModeSShort { data } => &data[..],
            RawFrame::ModeSLong { data } => &data[..],
        }
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
