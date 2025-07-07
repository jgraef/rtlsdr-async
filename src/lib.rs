//! # Async bindings for [librtlsdr][1]
//!
//! This crate provides async bindings for the [librtlsdr][1] C library.
//!
//! [1]: https://gitea.osmocom.org/sdr/rtl-sdr

mod bindings;
#[cfg(feature = "tcp")]
pub mod rtl_tcp;

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
    Error,
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
/// 8 bits per component, mapped from [-1, 1] to [0, 255]. K3XEC has [a good
/// reference][1] on the format.
///
/// [1]: https://k3xec.com/packrat-processing-iq/
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
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

/// Trait for async reading of samples.
///
/// This works pretty much like futures [`AsyncRead`][1],
/// except it works with 16 bit [`IqSample`]s instead of single bytes.
///
/// [1]: https://docs.rs/futures/latest/futures/io/trait.AsyncRead.html
pub trait AsyncReadSamples {
    /// Error that might occur when reading the IQ stream.
    type Error;

    /// Poll the stream to fill a buffer with IQ samples.
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

/// Extension trait for [`AsyncReadSamples`] with some useful methods.
pub trait AsyncReadSamplesExt: AsyncReadSamples {
    /// Read IQ samples into a buffer.
    ///
    /// This will call
    /// [`poll_read_samples`][AsyncReadSamples::poll_read_samples] exactly once,
    /// and return the number of bytes read. This is cancellation-safe.
    fn read_samples<'a>(&'a mut self, buffer: &'a mut [IqSample]) -> ReadSamples<'a, Self>
    where
        Self: Unpin,
    {
        ReadSamples {
            stream: self,
            buffer,
        }
    }

    /// Read IQ samples into a buffer until the buffer is full.
    ///
    /// This might call
    /// [`poll_read_samples`][AsyncReadSamples::poll_read_samples] multiple
    /// times, and thus is not cancellation-safe.
    fn read_samples_exact<'a>(
        &'a mut self,
        buffer: &'a mut [IqSample],
    ) -> ReadSamplesExact<'a, Self>
    where
        Self: Unpin,
    {
        ReadSamplesExact {
            stream: self,
            buffer,
            filled: 0,
        }
    }

    /// Maps any errors returned by the underlying stream with the provided
    /// closure.
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

/// Future that reads samples into a buffer.
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

/// Future that tries to read an exact amount of samples.
#[derive(Debug)]
pub struct ReadSamplesExact<'a, S: ?Sized> {
    stream: &'a mut S,
    buffer: &'a mut [IqSample],
    filled: usize,
}

impl<'a, 'b, S: AsyncReadSamples + Unpin + ?Sized> Future for ReadSamplesExact<'a, S> {
    type Output = Result<(), ReadSamplesExactError<S::Error>>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        while self.filled < self.buffer.len() {
            let this = &mut *self;
            match Pin::new(&mut *this.stream).poll_read_samples(cx, &mut this.buffer[this.filled..])
            {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(error)) => {
                    return Poll::Ready(Err(ReadSamplesExactError::Other(error)));
                }
                Poll::Ready(Ok(num_samples_read)) => {
                    if num_samples_read == 0 {
                        break;
                    }
                    else {
                        this.filled += num_samples_read;
                    }
                }
            }
        }

        if self.filled == self.buffer.len() {
            Poll::Ready(Ok(()))
        }
        else {
            Poll::Ready(Err(ReadSamplesExactError::Eof {
                num_bytes_read: self.filled,
            }))
        }
    }
}

/// Error returned by
/// [`read_samples_exact`][AsyncReadSamplesExt::read_samples_exact]
#[derive(Clone, Copy, Debug, thiserror::Error)]
pub enum ReadSamplesExactError<E> {
    /// The stream ended before the buffer could be filled completely.
    #[error("EOF after {num_bytes_read} bytes")]
    Eof { num_bytes_read: usize },

    /// The underlying stream produced an error.
    #[error("{0}")]
    Other(#[from] E),
}

pin_project! {
    /// Stream wrapper that maps the error type.
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

/// Trait for IQ streams that accept configuration options.
///
/// This is basically all the methods shared by
/// [`RtlTcpClient`][crate::rtl_tcp::client::RtlTcpClient], and [`RtlSdr`], so
/// that they can be used interchangeably.
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
        T::set_frequency_correction(*self, ppm)
    }

    fn set_tuner_if_gain(
        &mut self,
        stage: i16,
        gain: i16,
    ) -> impl Future<Output = Result<(), Self::Error>> {
        T::set_tuner_if_gain(*self, stage, gain)
    }

    fn set_offset_tuning(&mut self, enable: bool) -> impl Future<Output = Result<(), Self::Error>> {
        T::set_offset_tuning(*self, enable)
    }

    fn set_rtl_xtal(&mut self, frequency: u32) -> impl Future<Output = Result<(), Self::Error>> {
        T::set_rtl_xtal(*self, frequency)
    }

    fn set_tuner_xtal(&mut self, frequency: u32) -> impl Future<Output = Result<(), Self::Error>> {
        T::set_tuner_xtal(*self, frequency)
    }

    fn set_bias_tee(&mut self, enable: bool) -> impl Future<Output = Result<(), Self::Error>> {
        T::set_bias_tee(*self, enable)
    }
}

/// Tuner gain
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gain {
    /// Gain tenths of a dB
    ManualValue(i32),
    /// Tuner gain index specific to the tuner.
    ///
    /// Tuner gain values can be queries with [`RtlSdr::get_tuner_gains`].
    ManualIndex(usize),
    /// Auto gain control
    Auto,
}

/// Tuner gain mode
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TunerGainMode {
    /// Tuner gain is set manually
    Manual,
    /// Tuner gain is set automatically by the tuner.
    Auto,
}

/// Direct sampling mode
///
/// Direct sampling is not yet supported by [`RtlSdr`], but it can be used with
/// [`rtl_tcp`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DirectSamplingMode {
    /// Direct sampling of I branch
    I,
    /// Direct sampling of Q branch
    Q,
}

/// The type of tuner in a [`RtlSdr`].
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
