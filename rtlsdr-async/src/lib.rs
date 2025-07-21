//! # Async bindings for [librtlsdr][1]
//!
//! This crate provides async bindings for the [librtlsdr][1] C library.
//!
//! [1]: https://gitea.osmocom.org/sdr/rtl-sdr

mod buffer_queue;
mod control;
mod enumerate;
mod handle;
mod sampling;

#[cfg(feature = "tcp")]
pub mod rtl_tcp;

use std::{
    fmt::Debug,
    marker::PhantomData,
    ops::{
        Bound,
        RangeBounds,
    },
    pin::Pin,
    sync::Arc,
    task::{
        Context,
        Poll,
    },
};

use bytemuck::{
    Pod,
    Zeroable,
};
use futures_util::Stream;
#[cfg(feature = "num-complex")]
use num_complex::Complex;

pub use crate::enumerate::{
    DeviceInfo,
    DeviceIter,
    devices,
};
use crate::{
    buffer_queue::Buffer,
    control::Control,
    handle::Handle,
    sampling::spawn_reader_thread,
};

/// default buffer size is 16 KiB
///
/// at 2.4 Mhz sample rate this is equivalent to ~ 6.8 ms of samples
const DEFAULT_BUFFER_SIZE: usize = 0x4000; // 16 KiB

/// default queue size
///
/// together with `DEFAULT_BUFFER_SIZE` this makes a total of 1 MiB of buffers,
/// or ~436 ms of samples.
const DEFAULT_QUEUE_SIZE: usize = 64;

/// Errors returned by an [`RtlSdr`]
#[derive(Clone, Debug, thiserror::Error)]
pub enum Error {
    #[error("librtlsdr error: {function} retured {value}")]
    LibRtlSdr { function: &'static str, value: i32 },
    #[error("control handler thread died unexpectedly")]
    ControlThreadDead,
    #[error("reader handler thread died unexpectedly")]
    ReaderThreadDead,
    #[error("can't select gain level, because librtlsdr doesn't report any supported gain levels")]
    NoSupportedGains,
    #[error("unknown tuner")]
    UnknownTuner,
    #[error("operation not supported")]
    Unsupported,
    #[error("invalid gain index: {index}")]
    InvalidGainIndex { index: usize },
}

impl Error {
    pub(crate) fn from_lib(function: &'static str, value: i32) -> Self {
        Self::LibRtlSdr { function, value }
    }
}

/// An RTL-SDR.
///
/// This provides an async interface [`AsyncReadSamples`] to read IQ samples
/// from the device, and several methods to configure it.
///
/// [`RtlSdr`] is cheaply cloneable! All copies will read from the same
/// underlying device.
///
/// # Internals
///
/// Internally this spawns 2 threads:
///
/// 1. A thread that handles slow control commands like
///    [`Self::set_center_frequency`]. There will only ever be one control
///    thread for all devices.
/// 2. A thread that reads IQ samples from the device. Each [`RtlSdr`] will
///    spawn its own reader thread.
#[derive(Debug, Clone)]
pub struct RtlSdr {
    control: Control,

    buffer_queue_subscriber: buffer_queue::Subscriber,
}

impl RtlSdr {
    /// Open an RTL-SDR with the given index.
    ///
    /// You can enumerate the available devices with [`devices`].
    pub fn open(index: u32) -> Result<Self, Error> {
        Self::open_impl(index, DEFAULT_QUEUE_SIZE, DEFAULT_BUFFER_SIZE)
    }

    /// `buffer_size` must be somewhat carefully chosen. from the librtlsdr doc
    /// it seems like it must be at least a multiple of 512, and should really
    /// be a multiple of 16KiB
    fn open_impl(index: u32, queue_size: usize, buffer_size: usize) -> Result<Self, Error> {
        let handle = Arc::new(Handle::open(index)?);

        let control = Control::new(handle.clone());
        let buffer_queue_subscriber = spawn_reader_thread(handle, buffer_size, queue_size);

        Ok(Self {
            control,
            buffer_queue_subscriber,
        })
    }

    pub async fn get_center_frequency(&self) -> Result<u32, Error> {
        self.control.get_center_frequency().await
    }

    pub async fn set_center_frequency(&self, frequency: u32) -> Result<(), Error> {
        self.control.set_center_frequency(frequency).await
    }

    pub async fn get_sample_rate(&self) -> Result<u32, Error> {
        self.control.get_sample_rate().await
    }

    /// Valid sample rates are([source][1]):
    ///
    /// - from 225_001 Hz upto 300_000 Hz
    /// - from 900_001 Hz upto 3_200_000 Hz
    pub async fn set_sample_rate(&self, sample_rate: u32) -> Result<(), Error> {
        self.control.set_sample_rate(sample_rate).await
    }

    pub fn get_tuner_type(&self) -> TunerType {
        self.control.get_tuner_type()
    }

    pub fn get_tuner_gains(&self) -> &[i32] {
        self.control.get_tuner_gains()
    }

    pub async fn get_tuner_gain(&self) -> Result<i32, Error> {
        self.control.get_tuner_gain().await
    }

    pub async fn set_tuner_gain(&self, gain: Gain) -> Result<(), Error> {
        self.control.set_tuner_gain(gain).await
    }

    pub async fn set_tuner_if_gain(&self, stage: i32, gain: i32) -> Result<(), Error> {
        self.control.set_tuner_if_gain(stage, gain).await
    }

    pub async fn set_tuner_bandwidth(&self, bandwidth: u32) -> Result<(), Error> {
        self.control.set_tuner_bandwidth(bandwidth).await
    }

    pub async fn set_agc_mode(&self, enable: bool) -> Result<(), Error> {
        self.control.set_agc_mode(enable).await
    }

    pub async fn get_frequency_correction(&self) -> Result<i32, Error> {
        self.control.get_frequency_correction().await
    }

    pub async fn set_frequency_correction(&self, ppm: i32) -> Result<(), Error> {
        self.control.set_frequency_correction(ppm).await
    }

    pub async fn get_offset_tuning(&self) -> Result<bool, Error> {
        self.control.get_offset_tuning().await
    }

    pub async fn set_offset_tuning(&self, enable: bool) -> Result<(), Error> {
        self.control.set_offset_tuning(enable).await
    }

    pub async fn get_rtl_xtal(&self) -> Result<u32, Error> {
        self.control.get_rtl_xtal().await
    }

    pub async fn set_rtl_xtal(&self, frequency: u32) -> Result<(), Error> {
        self.control.set_rtl_xtal(frequency).await
    }

    pub async fn get_tuner_xtal(&self) -> Result<u32, Error> {
        self.control.get_tuner_xtal().await
    }

    pub async fn set_tuner_xtal(&self, frequency: u32) -> Result<(), Error> {
        self.control.set_tuner_xtal(frequency).await
    }

    pub async fn set_bias_tee(&self, enable: bool) -> Result<(), Error> {
        self.control.set_bias_tee(enable).await
    }

    pub async fn samples(&self) -> Result<Samples<Iq>, Error> {
        self.control.set_direct_sampling(None).await?;
        Ok(Samples {
            receiver: self.buffer_queue_subscriber.receiver(),
            sample_type: SampleType::Iq,
            _phantom: PhantomData,
        })
    }

    pub async fn direct_samples(&self, mode: DirectSamplingMode) -> Result<Samples<u8>, Error> {
        self.control.set_direct_sampling(Some(mode)).await?;
        Ok(Samples {
            receiver: self.buffer_queue_subscriber.receiver(),
            sample_type: mode.into(),
            _phantom: PhantomData,
        })
    }
}

impl Backend for RtlSdr {
    type Error = Error;

    fn dongle_info(&self) -> DongleInfo {
        DongleInfo {
            tuner_type: self.get_tuner_type(),
            tuner_gain_count: self
                .get_tuner_gains()
                .len()
                .try_into()
                .expect("number of tuner gains doesn't fit into an u32"),
        }
    }

    async fn set_center_frequency(&self, frequency: u32) -> Result<(), Error> {
        RtlSdr::set_center_frequency(self, frequency).await
    }

    async fn set_sample_rate(&self, sample_rate: u32) -> Result<(), Error> {
        RtlSdr::set_sample_rate(self, sample_rate).await
    }

    async fn set_tuner_gain(&self, gain: Gain) -> Result<(), Error> {
        RtlSdr::set_tuner_gain(self, gain).await
    }

    async fn set_agc_mode(&self, enable: bool) -> Result<(), Error> {
        RtlSdr::set_agc_mode(self, enable).await
    }

    async fn set_frequency_correction(&self, ppm: i32) -> Result<(), Error> {
        RtlSdr::set_frequency_correction(self, ppm).await
    }

    async fn set_tuner_if_gain(&self, stage: i16, gain: i16) -> Result<(), Error> {
        RtlSdr::set_tuner_if_gain(self, stage.into(), gain.into()).await
    }

    async fn set_offset_tuning(&self, enable: bool) -> Result<(), Error> {
        RtlSdr::set_offset_tuning(self, enable).await
    }

    async fn set_rtl_xtal(&self, frequency: u32) -> Result<(), Error> {
        RtlSdr::set_rtl_xtal(self, frequency).await
    }

    async fn set_tuner_xtal(&self, frequency: u32) -> Result<(), Error> {
        RtlSdr::set_tuner_xtal(self, frequency).await
    }

    async fn set_bias_tee(&self, enable: bool) -> Result<(), Error> {
        RtlSdr::set_bias_tee(self, enable).await
    }

    async fn samples(&self) -> Result<Samples<Iq>, Error> {
        RtlSdr::samples(self).await
    }

    async fn direct_samples(&self, mode: DirectSamplingMode) -> Result<Samples<u8>, Error> {
        RtlSdr::direct_samples(self, mode).await
    }
}

#[derive(Clone, Debug)]
pub struct Samples<T> {
    receiver: buffer_queue::Receiver,
    sample_type: SampleType,
    _phantom: PhantomData<fn() -> T>,
}

// this could just return the buffers without the Result, because the buffer
// queue doesn't return errors, but for future compatibility we will make it
// return Results.
impl<T> Stream for Samples<T> {
    type Item = Result<Chunk<T>, Error>;

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Chunk<T>, Error>>> {
        match Pin::new(&mut self.receiver).poll_next(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(None) => Poll::Ready(None),
            //Poll::Ready(Some(Err(error))) => Poll::Ready(Some(Err(error))),
            Poll::Ready(Some(buffer)) => {
                if self.sample_type == buffer.sample_type {
                    Poll::Ready(Some(Ok(Chunk {
                        buffer,
                        _phantom: PhantomData,
                    })))
                }
                else {
                    // switched sampling mode, so this stream ends
                    Poll::Ready(None)
                }
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct Chunk<T> {
    buffer: Buffer,
    _phantom: PhantomData<fn() -> T>,
}

impl<T: Pod> Chunk<T> {
    #[inline]
    pub fn samples(&self) -> &[T] {
        bytemuck::cast_slice(self.buffer.filled())
    }

    #[inline]
    pub fn iter(&self) -> std::slice::Iter<'_, T> {
        self.samples().iter()
    }
}

impl<T> Chunk<T> {
    #[inline]
    pub fn sample_rate(&self) -> u32 {
        self.buffer.sample_rate
    }

    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        self.buffer.filled()
    }

    #[inline]
    pub fn slice(&mut self, range: impl RangeBounds<usize>) {
        let width = size_of::<T>();
        self.buffer.slice(map_bounds(range, |x| x * width))
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.buffer.len() / size_of::<T>()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }
}

#[inline]
fn map_bound<T, U, F>(bound: Bound<T>, f: F) -> Bound<U>
where
    F: FnOnce(T) -> U,
{
    match bound {
        Bound::Included(bound) => Bound::Included(f(bound)),
        Bound::Excluded(bound) => Bound::Excluded(f(bound)),
        Bound::Unbounded => Bound::Unbounded,
    }
}

#[inline]
fn map_bounds<T, U, F>(bounds: impl RangeBounds<T>, mut f: F) -> (Bound<U>, Bound<U>)
where
    F: FnMut(&T) -> U,
{
    (
        map_bound(bounds.start_bound().clone(), &mut f),
        map_bound(bounds.end_bound().clone(), &mut f),
    )
}

impl<T: Pod> AsRef<[T]> for Chunk<T> {
    fn as_ref(&self) -> &[T] {
        self.samples()
    }
}

/// 16 bit IQ sample
///
/// 8 bits per component, mapped from [-1, 1] to [0, 255]. K3XEC has [a good
/// reference][1] on the format.
///
/// [1]: https://k3xec.com/packrat-processing-iq/
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
#[repr(C)]
pub struct Iq {
    /// I: in-phase / real component
    pub i: u8,
    /// Q: quadrature / imaginary component
    pub q: u8,
}

impl Default for Iq {
    fn default() -> Self {
        Self { i: 128, q: 128 }
    }
}

#[cfg(feature = "num-complex")]
impl From<Iq> for Complex<f32> {
    fn from(value: Iq) -> Self {
        Self {
            re: u8_to_f32(value.i),
            im: u8_to_f32(value.q),
        }
    }
}

#[inline]
fn u8_to_f32(x: u8) -> f32 {
    // map the special rtlsdr encoding to f32
    (x as f32) / 255.0 * 2.0 - 1.0
}

/// RTL-SDR backend.
///
/// This is basically all the methods shared by
/// [`RtlTcpClient`][crate::rtl_tcp::client::RtlTcpClient], and [`RtlSdr`], so
/// that they can be used interchangeably.
pub trait Backend {
    type Error: std::error::Error + Send;

    fn dongle_info(&self) -> DongleInfo;

    /// Set tuner frequency in Hz
    fn set_center_frequency(
        &self,
        frequency: u32,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + Sync;

    /// Set sample rate in Hz
    fn set_sample_rate(
        &self,
        sample_rate: u32,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + Sync;

    /// Set tuner gain, in tenths of a dB
    fn set_tuner_gain(
        &self,
        gain: Gain,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + Sync;

    /// Set the automatic gain correction, a software step to correct the
    /// incoming signal, this is not automatic gain control on the hardware
    /// chip, that is controlled by tuner gain mode.
    fn set_agc_mode(
        &self,
        enable: bool,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + Sync;

    fn set_frequency_correction(
        &self,
        ppm: i32,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + Sync;

    fn set_tuner_if_gain(
        &self,
        stage: i16,
        gain: i16,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + Sync;

    fn set_offset_tuning(
        &self,
        enable: bool,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + Sync;

    fn set_rtl_xtal(
        &self,
        frequency: u32,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + Sync;

    fn set_tuner_xtal(
        &self,
        frequency: u32,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + Sync;

    fn set_bias_tee(
        &self,
        enable: bool,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + Sync;

    fn samples(&self) -> impl Future<Output = Result<Samples<Iq>, Self::Error>> + Send + Sync;

    fn direct_samples(
        &self,
        mode: DirectSamplingMode,
    ) -> impl Future<Output = Result<Samples<u8>, Self::Error>> + Send + Sync;
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum SampleType {
    #[default]
    Iq,
    I,
    Q,
}

impl From<DirectSamplingMode> for SampleType {
    fn from(value: DirectSamplingMode) -> Self {
        match value {
            DirectSamplingMode::I => Self::I,
            DirectSamplingMode::Q => Self::Q,
        }
    }
}

impl From<Option<DirectSamplingMode>> for SampleType {
    fn from(value: Option<DirectSamplingMode>) -> Self {
        value.map(Into::into).unwrap_or(SampleType::Iq)
    }
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

/// Information about the SDR dongle that is sent by the server.
#[derive(Clone, Copy, Debug)]
pub struct DongleInfo {
    /// Tuner type as reported by librtlsdr
    pub tuner_type: TunerType,

    /// Number of gain levels supported by the tuner.
    pub tuner_gain_count: u32,
}
