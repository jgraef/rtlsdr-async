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
use futures_core::Stream;
use pin_project_lite::pin_project;

pub use crate::{
    buffer_queue::Buffer,
    enumerate::{
        DeviceInfo,
        DeviceIter,
        devices,
    },
};
use crate::{
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

    // for now we'll create the receiver the first time poll_read_samples is called.
    // eventually we want RtlSdr to have a stream method which returns a separate IqStream. This
    // way we can also have methods that start direct sampling, which would return a different type
    // of stream. but i'm not sure yet how we would have a unified interface with RtlTcpClient.
    buffer_queue_reader: Option<buffer_queue::Reader>,
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
            buffer_queue_reader: None,
        })
    }

    pub fn get_center_frequency(&self) -> Result<u32, Error> {
        self.control.get_center_frequency()
    }

    pub async fn set_center_frequency(&self, frequency: u32) -> Result<(), Error> {
        self.control.set_center_frequency(frequency).await
    }

    pub fn get_sample_rate(&self) -> Result<u32, Error> {
        self.control.get_sample_rate()
    }

    pub async fn set_sample_rate(&self, sample_rate: u32) -> Result<(), Error> {
        self.control.set_sample_rate(sample_rate).await
    }

    pub fn get_tuner_type(&self) -> TunerType {
        self.control.get_tuner_type()
    }

    pub fn get_tuner_gains(&self) -> &[i32] {
        self.control.get_tuner_gains()
    }

    pub fn get_tuner_gain(&self) -> Result<i32, Error> {
        self.control.get_tuner_gain()
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

    pub fn get_frequency_correction(&self) -> Result<i32, Error> {
        self.control.get_frequency_correction()
    }

    pub async fn set_frequency_correction(&self, ppm: i32) -> Result<(), Error> {
        self.control.set_frequency_correction(ppm).await
    }

    pub fn get_offset_tuning(&self) -> Result<bool, Error> {
        self.control.get_offset_tuning()
    }

    pub async fn set_offset_tuning(&self, enable: bool) -> Result<(), Error> {
        self.control.set_offset_tuning(enable).await
    }

    pub fn get_rtl_xtal(&self) -> Result<u32, Error> {
        self.control.get_rtl_xtal()
    }

    pub async fn set_rtl_xtal(&self, frequency: u32) -> Result<(), Error> {
        self.control.set_rtl_xtal(frequency).await
    }

    pub fn get_tuner_xtal(&self) -> Result<u32, Error> {
        self.control.get_tuner_xtal()
    }

    pub async fn set_tuner_xtal(&self, frequency: u32) -> Result<(), Error> {
        self.control.set_tuner_xtal(frequency).await
    }

    pub async fn set_bias_tee(&self, enable: bool) -> Result<(), Error> {
        self.control.set_bias_tee(enable).await
    }

    fn reader(&mut self) -> &mut buffer_queue::Reader {
        self.buffer_queue_reader
            .get_or_insert_with(|| self.buffer_queue_subscriber.receiver().into())
    }
}

impl AsyncReadSamples for RtlSdr {
    type Error = Error;

    fn poll_read_samples(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &mut [IqSample],
    ) -> Poll<Result<usize, Self::Error>> {
        Pin::new(self.reader())
            .poll_read_samples(cx, buffer)
            .map_err(|error| match error {})
    }
}

// this could just return the buffers without the Result, because the buffer
// queue doesn't return errors, but for future compatibility we will make it
// return Results.
impl Stream for RtlSdr {
    type Item = Result<Buffer, Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.reader().receiver)
            .poll_next(cx)
            .map(|ready| ready.map(Ok))
    }
}

impl Configure for RtlSdr {
    type Error = Error;

    async fn set_center_frequency(&mut self, frequency: u32) -> Result<(), Error> {
        RtlSdr::set_center_frequency(&*self, frequency).await
    }

    async fn set_sample_rate(&mut self, sample_rate: u32) -> Result<(), Error> {
        RtlSdr::set_sample_rate(&*self, sample_rate).await
    }

    async fn set_tuner_gain(&mut self, gain: Gain) -> Result<(), Error> {
        RtlSdr::set_tuner_gain(&*self, gain).await
    }

    async fn set_agc_mode(&mut self, enable: bool) -> Result<(), Error> {
        RtlSdr::set_agc_mode(&*self, enable).await
    }

    async fn set_frequency_correction(&mut self, ppm: i32) -> Result<(), Error> {
        RtlSdr::set_frequency_correction(&*self, ppm).await
    }

    async fn set_tuner_if_gain(&mut self, stage: i16, gain: i16) -> Result<(), Error> {
        RtlSdr::set_tuner_if_gain(&*self, stage.into(), gain.into()).await
    }

    async fn set_offset_tuning(&mut self, enable: bool) -> Result<(), Error> {
        RtlSdr::set_offset_tuning(&*self, enable).await
    }

    async fn set_rtl_xtal(&mut self, frequency: u32) -> Result<(), Error> {
        RtlSdr::set_rtl_xtal(&*self, frequency).await
    }

    async fn set_tuner_xtal(&mut self, frequency: u32) -> Result<(), Error> {
        RtlSdr::set_tuner_xtal(&*self, frequency).await
    }

    async fn set_bias_tee(&mut self, enable: bool) -> Result<(), Error> {
        RtlSdr::set_bias_tee(&*self, enable).await
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
                num_samples_read: self.filled,
            }))
        }
    }
}

/// Error returned by
/// [`read_samples_exact`][AsyncReadSamplesExt::read_samples_exact]
#[derive(Clone, Copy, Debug, thiserror::Error)]
pub enum ReadSamplesExactError<E> {
    /// The stream ended before the buffer could be filled completely.
    #[error("EOF after {num_samples_read} samples")]
    Eof { num_samples_read: usize },

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
