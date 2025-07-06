use std::{
    ffi::{
        CStr,
        c_void,
    },
    fmt::Debug,
    ops::{
        Deref,
        DerefMut,
    },
    pin::Pin,
    ptr::null_mut,
    sync::{
        Arc,
        OnceLock,
    },
    task::{
        Context,
        Poll,
    },
    thread::{
        self,
    },
};

use parking_lot::Mutex;
use rtlsdr_sys::{
    rtlsdr_get_center_freq,
    rtlsdr_read_sync,
};
use tokio::sync::{
    mpsc,
    oneshot,
};

use crate::{
    AsyncReadSamples,
    Configure,
    Gain,
    IqSample,
    TunerType,
};

const DEFAULT_BUFFER_SIZE: usize = 0x4000; // 16 KiB
const DEFAULT_QUEUE_SIZE: usize = 64; // total of 1 MiB buffers

#[derive(Debug, thiserror::Error)]
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
}

impl Error {
    pub fn from_lib(function: &'static str, value: i32) -> Self {
        Self::LibRtlSdr { function, value }
    }
}

pub fn devices() -> DeviceIter {
    let device_count = unsafe { rtlsdr_sys::rtlsdr_get_device_count() };

    DeviceIter {
        device_count,
        index: 0,
    }
}

#[derive(Clone, Copy, Debug)]
pub struct DeviceIter {
    device_count: u32,
    index: u32,
}

impl Iterator for DeviceIter {
    type Item = DeviceInfo;

    fn next(&mut self) -> Option<Self::Item> {
        while self.index < self.device_count {
            let index = self.index;
            self.index += 1;

            let device_name = unsafe { CStr::from_ptr(rtlsdr_sys::rtlsdr_get_device_name(index)) };

            if !device_name.is_empty() {
                let mut manufacturer = [0u8; 256];
                let mut product = [0u8; 256];
                let mut serial = [0u8; 256];

                let ret = unsafe {
                    rtlsdr_sys::rtlsdr_get_device_usb_strings(
                        index,
                        manufacturer.as_mut_ptr() as *mut i8,
                        product.as_mut_ptr() as *mut i8,
                        serial.as_mut_ptr() as *mut i8,
                    )
                };

                let usb_strings = (ret == 0).then(|| {
                    UsbStrings {
                        manufacturer: UsbString::new(manufacturer),
                        product: UsbString::new(product),
                        serial: UsbString::new(serial),
                    }
                });

                return Some(DeviceInfo {
                    index,
                    device_name,
                    usb_strings,
                });
            }
        }

        None
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let n = self.device_count - self.index;
        (0, Some(n.try_into().unwrap()))
    }
}

#[derive(Clone, Copy, Debug)]
pub struct DeviceInfo {
    index: u32,
    device_name: &'static CStr,
    usb_strings: Option<UsbStrings>,
}

impl DeviceInfo {
    pub fn index(&self) -> u32 {
        self.index
    }

    pub fn device_name(&self) -> Option<&str> {
        self.device_name.to_str().ok()
    }

    pub fn manufacturer(&self) -> Option<&str> {
        self.usb_strings
            .as_ref()
            .and_then(|s| s.manufacturer.as_str())
    }

    pub fn product(&self) -> Option<&str> {
        self.usb_strings.as_ref().and_then(|s| s.product.as_str())
    }

    pub fn serial(&self) -> Option<&str> {
        self.usb_strings.as_ref().and_then(|s| s.serial.as_str())
    }

    pub fn open(&self) -> Result<RtlSdr, Error> {
        RtlSdr::open(self.index)
    }
}

#[derive(Clone, Copy, Debug)]
struct UsbStrings {
    manufacturer: UsbString,
    product: UsbString,
    serial: UsbString,
}

#[derive(Clone, Copy, Debug)]
struct UsbString {
    bytes: [u8; Self::BUFFER_SIZE],
    length: usize,
}

impl UsbString {
    const BUFFER_SIZE: usize = 256;

    pub fn new(bytes: [u8; Self::BUFFER_SIZE]) -> Self {
        let length = bytes
            .iter()
            .position(|b| *b == 0)
            .expect("string not nul-terminated");
        Self { bytes, length }
    }

    pub fn as_str(&self) -> Option<&str> {
        str::from_utf8(&self.bytes[..self.length]).ok()
    }
}

/// This whole thing is so unsafe!
///
/// So basically the only way to use librtlsdr is with multiple threads, but its
/// not thread-safe at all! rtl_tcp et al. do it this way: have one thread read
/// the data with rtlsdr_read_async and have another thread set the tuner
/// frequency etc. We'll do the same, but we need to share the device handle for
/// that. Therefore this wrapper is Send + Sync. It also makes sure to close the
/// device when dropped, and adds convenient methods for the functions we want
/// to call.
#[derive(Debug)]
struct Handle {
    handle: rtlsdr_sys::rtlsdr_dev_t,

    // the mutex is separate because it's only used to synchronize control operations (everything
    // that isn't a read).
    control_lock: Mutex<()>,

    tuner_type: TunerType,

    tuner_gains: TunerGains,
}

unsafe impl Send for Handle {}
unsafe impl Sync for Handle {}

impl Handle {
    fn open(index: u32) -> Result<Self, Error> {
        let mut handle: rtlsdr_sys::rtlsdr_dev_t = null_mut();
        let ret =
            unsafe { rtlsdr_sys::rtlsdr_open(&mut handle as *mut rtlsdr_sys::rtlsdr_dev_t, index) };
        if ret != 0 {
            return Err(Error::from_lib("rtlsdr_open", ret));
        }

        // get the tuner type.
        let ret: u32 = unsafe { rtlsdr_sys::rtlsdr_get_tuner_type(handle) } as u32;
        tracing::debug!(ret, "rtlsdr_get_tuner_type");
        if ret == 0 {
            return Err(Error::UnknownTuner);
        }
        let tuner_type = TunerType(ret);

        // get the tuner gains now, so we can hand them out as a slice later. this way
        // we don't need to allocate a Vec everytime get_tuner_gains is called.
        // furthermore the arrays returned by librtlsdr are fixed and as of writing
        // don't exceed 29 entries.
        let ret = unsafe { rtlsdr_sys::rtlsdr_get_tuner_gains(handle, null_mut()) };
        tracing::debug!(ret, "rtlsdr_get_tuner_gains");
        let mut tuner_gains = TunerGains::default();
        if let Ok(num_gains) = ret.try_into() {
            if num_gains < TunerGains::CAPACITY {
                tuner_gains.length = num_gains;
                let ret2 = unsafe {
                    rtlsdr_sys::rtlsdr_get_tuner_gains(handle, tuner_gains.values.as_mut_ptr())
                };
                assert_eq!(
                    ret, ret2,
                    "rtlsdr_get_tuner_gains returned 2 different lengths"
                );
            }
            else {
                tracing::warn!(
                    ?num_gains,
                    capacity = TunerGains::CAPACITY,
                    "number of tuner gains available exceeds capacity"
                );
            }
        }
        tracing::debug!(gains = ?tuner_gains, "rtlsdr_get_tuner_gains");

        Ok(Handle {
            handle,
            control_lock: Mutex::new(()),
            tuner_type,
            tuner_gains,
        })
    }

    fn get_center_frequency(&self) -> Result<u32, Error> {
        let _guard = self.control_lock.lock();
        let ret = unsafe { rtlsdr_get_center_freq(self.handle) };
        tracing::debug!(ret, "rtlsdr_get_center_freq");
        if ret == 0 {
            Err(Error::from_lib("rtlsdr_get_center_freq", 0))
        }
        else {
            Ok(ret)
        }
    }

    fn set_center_frequency(&self, frequency: u32) -> Result<(), Error> {
        let _guard = self.control_lock.lock();
        let ret = unsafe { rtlsdr_sys::rtlsdr_set_center_freq(self.handle, frequency) };
        tracing::debug!(ret, frequency, "rtlsdr_set_center_freq");
        if ret == 0 {
            Ok(())
        }
        else {
            Err(Error::from_lib("rtlsdr_set_center_freq", ret))
        }
    }

    fn get_sample_rate(&self) -> Result<u32, Error> {
        let _guard = self.control_lock.lock();
        let ret = unsafe { rtlsdr_sys::rtlsdr_get_sample_rate(self.handle) };
        tracing::debug!(ret, "rtlsdr_get_sample_rate");
        if ret == 0 {
            Ok(ret)
        }
        else {
            Err(Error::from_lib("rtlsdr_get_sample_rate", ret as i32))
        }
    }

    fn set_sample_rate(&self, sample_rate: u32) -> Result<(), Error> {
        let _guard = self.control_lock.lock();
        let ret = unsafe { rtlsdr_sys::rtlsdr_set_sample_rate(self.handle, sample_rate) };
        tracing::debug!(ret, sample_rate, "rtlsdr_set_sample_rate");
        if ret == 0 {
            Ok(())
        }
        else {
            Err(Error::from_lib("rtlsdr_set_sample_rate", ret))
        }
    }

    fn set_tuner_gain_mode(&self, manual: bool) -> Result<(), Error> {
        let _guard = self.control_lock.lock();
        let ret = unsafe { rtlsdr_sys::rtlsdr_set_tuner_gain_mode(self.handle, manual as i32) };
        tracing::debug!(ret, ?manual, "rtlsdr_set_tuner_gain_mode");
        if ret == 0 {
            Ok(())
        }
        else {
            Err(Error::from_lib("rtlsdr_set_tuner_gain_mode", ret))
        }
    }

    fn get_tuner_gain(&self) -> Result<i32, Error> {
        let _guard = self.control_lock.lock();
        let ret = unsafe { rtlsdr_sys::rtlsdr_get_tuner_gain(self.handle) };
        tracing::debug!(ret, "rtlsdr_get_tuner_gain");
        // note: looking at the librtlsdr source it looks like that 0 is also a valid
        // gain value. rtlsdr_get_tuner_gain only fails if the provided dev
        // handle is null, which it isn't. but it will set dev->gain (the value
        // returrned here) to 0, if set_gain failed
        if ret == 0 {
            Err(Error::from_lib("rtlsdr_get_tuner_gain", ret))
        }
        else {
            Ok(ret)
        }
    }

    fn set_tuner_gain(&self, gain: i32) -> Result<(), Error> {
        let _guard = self.control_lock.lock();
        let ret = unsafe { rtlsdr_sys::rtlsdr_set_tuner_gain(self.handle, gain) };
        tracing::debug!(ret, gain, "rtlsdr_set_tuner_gain");
        if ret == 0 {
            Ok(())
        }
        else {
            Err(Error::from_lib("rtlsdr_set_tuner_gain", ret))
        }
    }

    fn set_tuner_if_gain(&self, stage: i32, gain: i32) -> Result<(), Error> {
        let _guard = self.control_lock.lock();
        let ret = unsafe { rtlsdr_sys::rtlsdr_set_tuner_if_gain(self.handle, stage, gain) };
        tracing::debug!(ret, gain, "rtlsdr_set_tuner_if_gain");
        if ret == 0 {
            Ok(())
        }
        else {
            Err(Error::from_lib("rtlsdr_set_tuner_if_gain", ret))
        }
    }

    fn set_tuner_bandwidth(&self, bandwidth: u32) -> Result<(), Error> {
        let _guard = self.control_lock.lock();
        let ret = unsafe { rtlsdr_sys::rtlsdr_set_tuner_bandwidth(self.handle, bandwidth) };
        tracing::debug!(ret, bandwidth, "rtlsdr_set_tuner_bandwidth");
        if ret == 0 {
            Ok(())
        }
        else {
            Err(Error::from_lib("rtlsdr_set_tuner_bandwidth", ret))
        }
    }

    fn set_agc_mode(&self, enable: bool) -> Result<(), Error> {
        let _guard = self.control_lock.lock();
        let ret = unsafe { rtlsdr_sys::rtlsdr_set_agc_mode(self.handle, enable as i32) };
        tracing::debug!(ret, ?enable, "rtlsdr_set_agc_mode");
        if ret == 0 {
            Ok(())
        }
        else {
            Err(Error::from_lib("rtlsdr_set_agc_mode", ret))
        }
    }

    fn get_frequency_correction(&self) -> Result<i32, Error> {
        let _guard = self.control_lock.lock();
        let ret = unsafe { rtlsdr_sys::rtlsdr_get_freq_correction(self.handle) };
        tracing::debug!(ret, "rtlsdr_get_freq_correction");
        // note: only returns errors for dev=null, and besides 0 is a valid return value
        Ok(ret)
    }

    fn set_frequency_correction(&self, ppm: i32) -> Result<(), Error> {
        let _guard = self.control_lock.lock();
        let ret = unsafe { rtlsdr_sys::rtlsdr_set_freq_correction(self.handle, ppm) };
        tracing::debug!(ret, ?ppm, "rtlsdr_set_freq_correction");
        // -2 means that this value is already set, so not really an error
        if ret == 0 || ret == -2 {
            Ok(())
        }
        else {
            Err(Error::from_lib("rtlsdr_set_freq_correction", ret))
        }
    }

    fn get_offset_tuning(&self) -> Result<bool, Error> {
        let _guard = self.control_lock.lock();
        let ret = unsafe { rtlsdr_sys::rtlsdr_get_offset_tuning(self.handle) };
        tracing::debug!(ret, "rtlsdr_get_offset_tuning");
        // note: only returns errors for dev=null
        assert!(ret == 0 || ret == 1);
        Ok(ret == 1)
    }

    fn set_offset_tuning(&self, enable: bool) -> Result<(), Error> {
        // from the rtlsdr_set_offset_tuning code:
        //
        // ```c
        // if ((dev->tuner_type == RTLSDR_TUNER_R820T) ||
        //   (dev->tuner_type == RTLSDR_TUNER_R828D)) {
        //   /* RTL-SDR-BLOG Hack, enables us to turn on the bias tee by
        //    * clicking on "offset tuning" in software that doesn't have
        //    * specified bias tee support. Offset tuning is not used for
        //    *R820T devices so it is no problem.
        //    */
        //    rtlsdr_set_bias_tee(dev, on);
        //    return -2;
        // }
        // ```
        //
        // we will return Error::Unsupported if anyone tries to **enable** Bias-T on an
        // R8xx. If we decided that we want to allow this hack, we need to also accept
        // -2 as a ok return value.
        if self.tuner_type.is_r82xx() {
            return Err(Error::Unsupported);
        }

        let _guard = self.control_lock.lock();
        let ret = unsafe { rtlsdr_sys::rtlsdr_set_offset_tuning(self.handle, enable as i32) };
        tracing::debug!(ret, ?enable, "rtlsdr_set_offset_tuning");
        if ret == 0 {
            Ok(())
        }
        else {
            Err(Error::from_lib("rtlsdr_set_offset_tuning", ret))
        }
    }

    fn get_xtal_frequency(&self) -> Result<(u32, u32), Error> {
        let _guard = self.control_lock.lock();
        let mut rtl_frequency = 0;
        let mut tuner_frequency = 0;
        let ret = unsafe {
            rtlsdr_sys::rtlsdr_get_xtal_freq(
                self.handle,
                &mut rtl_frequency as *mut u32,
                &mut tuner_frequency as *mut u32,
            )
        };
        tracing::debug!(
            ret,
            ?rtl_frequency,
            ?tuner_frequency,
            "rtlsdr_get_xtal_freq"
        );
        // note: only returns errors for dev=null
        assert_eq!(ret, 0);
        Ok((rtl_frequency, tuner_frequency))
    }

    fn set_xtal_frequency(&self, rtl_frequency: u32, tuner_frequency: u32) -> Result<(), Error> {
        let _guard = self.control_lock.lock();
        let ret = unsafe {
            rtlsdr_sys::rtlsdr_set_xtal_freq(self.handle, rtl_frequency, tuner_frequency)
        };
        tracing::debug!(
            ret,
            ?rtl_frequency,
            ?tuner_frequency,
            "rtlsdr_set_xtal_freq"
        );
        if ret == 0 {
            Ok(())
        }
        else {
            Err(Error::from_lib("rtlsdr_set_xtal_freq", ret))
        }
    }

    fn set_bias_tee(&self, pin: u8, enable: bool) -> Result<(), Error> {
        // todo: missing from ffi bindings

        /*let _guard = self.control_lock.lock();
        let ret = unsafe { rtlsdr_sys::rtlsdr_set_bias_tee_gpio(self.handle, pin.into(), enable as i32) };
        tracing::debug!(ret, ?rtl_frequency, ?tuner_frequency, "rtlsdr_set_bias_tee_gpio");
        if ret == 0 {
            Ok(())
        }
        else {
            Err(Error::from_lib("rtlsdr_set_bias_tee_gpio", ret))
        }*/

        tracing::warn!(?pin, ?enable, "todo: rtlsdr_set_bias_tee_gpio");

        // we will just return Ok(()) and pretend we set it
        Ok(())
    }

    // not synchronized! this must only be used in the reader_thread
    fn read_sync(&self, buffer: &mut [u8]) -> Result<usize, Error> {
        let mut n_read: i32 = 0;

        let ret = unsafe {
            rtlsdr_read_sync(
                self.handle,
                buffer.as_mut_ptr() as *mut c_void,
                buffer
                    .len()
                    .try_into()
                    .expect("buffer size too large for i32"),
                &mut n_read as *mut i32,
            )
        };

        if ret == 0 {
            Ok(n_read.try_into().unwrap())
        }
        else {
            Err(Error::from_lib("rtlsdr_read_sync", ret))
        }
    }

    fn reset_buffer(&self) {
        // note: only fails if the dev pointer is null, which it is not
        let ret = unsafe { rtlsdr_sys::rtlsdr_reset_buffer(self.handle) };
        assert_eq!(ret, 0, "rtlsdr_reset_buffer didn't return 0");
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            rtlsdr_sys::rtlsdr_close(self.handle);
        }
    }
}

#[derive(Clone, Copy)]
struct TunerGains {
    values: [i32; Self::CAPACITY],
    length: usize,
}

impl Default for TunerGains {
    fn default() -> Self {
        Self {
            values: [0; Self::CAPACITY],
            length: 0,
        }
    }
}

impl TunerGains {
    const CAPACITY: usize = 64;

    fn iter(&self) -> std::slice::Iter<'_, i32> {
        self.values[..self.length].iter()
    }
}

impl AsRef<[i32]> for TunerGains {
    fn as_ref(&self) -> &[i32] {
        &self.values[..self.length]
    }
}

impl Debug for TunerGains {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}

#[derive(Clone)]
pub struct RtlSdr {
    /// the handle for the rtlsdr. this also provides convenient methods. all
    /// methods except reads are synchronized.
    handle: Arc<Handle>,

    /// sender to send commands to control thread for slow control commands
    control_queue_sender: mpsc::Sender<ControlMessage>,

    /// reader for the buffer broadcast queue.
    buffer_queue_reader: buffer_queue::Reader,

    /// the buffer if we currently have one. this must be read first, before
    /// fetching a new one from the queue
    buffer: Option<Buffer>,

    // read position in buffer
    buffer_pos: usize,
}

impl RtlSdr {
    pub fn open(index: u32) -> Result<Self, Error> {
        Self::open_impl(index, DEFAULT_QUEUE_SIZE, DEFAULT_BUFFER_SIZE)
    }

    fn open_impl(index: u32, queue_size: usize, buffer_size: usize) -> Result<Self, Error> {
        let handle = Arc::new(Handle::open(index)?);

        // this is needed for reads to work
        handle.reset_buffer();

        let control_queue_sender = get_control_queue_sender();

        let (buffer_queue_writer, buffer_queue_reader) =
            buffer_queue::channel(queue_size, buffer_size);

        thread::spawn({
            let handle = handle.clone();
            move || {
                reader_thread(buffer_queue_writer, handle);
            }
        });

        Ok(Self {
            handle,
            control_queue_sender,
            buffer_queue_reader,
            buffer: None,
            buffer_pos: 0,
        })
    }

    pub fn get_center_frequency(&self) -> Result<u32, Error> {
        self.handle.get_center_frequency()
    }

    pub async fn set_center_frequency(&self, frequency: u32) -> Result<(), Error> {
        let (result_sender, result_receiver) = oneshot::channel();
        self.control_queue_sender
            .send(ControlMessage::SetCenterFrequency {
                handle: self.handle.clone(),
                frequency,
                result_sender,
            })
            .await
            .map_err(|_| Error::ControlThreadDead)?;
        result_receiver
            .await
            .map_err(|_| Error::ControlThreadDead)?
    }

    pub fn get_sample_rate(&self) -> Result<u32, Error> {
        self.handle.get_sample_rate()
    }

    pub async fn set_sample_rate(&self, sample_rate: u32) -> Result<(), Error> {
        let (result_sender, result_receiver) = oneshot::channel();
        self.control_queue_sender
            .send(ControlMessage::SetSampleRate {
                handle: self.handle.clone(),
                sample_rate,
                result_sender,
            })
            .await
            .map_err(|_| Error::ControlThreadDead)?;
        result_receiver
            .await
            .map_err(|_| Error::ControlThreadDead)?
    }

    pub fn get_tuner_type(&self) -> TunerType {
        self.handle.tuner_type
    }

    pub fn get_tuner_gains(&self) -> &[i32] {
        self.handle.tuner_gains.as_ref()
    }

    pub fn get_tuner_gain(&self) -> Result<i32, Error> {
        self.handle.get_tuner_gain()
    }

    pub async fn set_tuner_gain(&self, gain: Gain) -> Result<(), Error> {
        let (result_sender, result_receiver) = oneshot::channel();
        self.control_queue_sender
            .send(ControlMessage::SetTunerGain {
                handle: self.handle.clone(),
                gain,
                result_sender,
            })
            .await
            .map_err(|_| Error::ControlThreadDead)?;
        result_receiver
            .await
            .map_err(|_| Error::ControlThreadDead)?
    }

    pub async fn set_tuner_if_gain(&self, stage: i32, gain: i32) -> Result<(), Error> {
        let (result_sender, result_receiver) = oneshot::channel();
        self.control_queue_sender
            .send(ControlMessage::SetTunerIfGain {
                handle: self.handle.clone(),
                stage,
                gain,
                result_sender,
            })
            .await
            .map_err(|_| Error::ControlThreadDead)?;
        result_receiver
            .await
            .map_err(|_| Error::ControlThreadDead)?
    }

    pub async fn set_tuner_bandwidth(&self, bandwidth: u32) -> Result<(), Error> {
        let (result_sender, result_receiver) = oneshot::channel();
        self.control_queue_sender
            .send(ControlMessage::SetTunerBandwidth {
                handle: self.handle.clone(),
                bandwidth,
                result_sender,
            })
            .await
            .map_err(|_| Error::ControlThreadDead)?;
        result_receiver
            .await
            .map_err(|_| Error::ControlThreadDead)?
    }

    pub async fn set_agc_mode(&self, enable: bool) -> Result<(), Error> {
        let (result_sender, result_receiver) = oneshot::channel();
        self.control_queue_sender
            .send(ControlMessage::SetAgcMode {
                handle: self.handle.clone(),
                enable,
                result_sender,
            })
            .await
            .map_err(|_| Error::ControlThreadDead)?;
        result_receiver
            .await
            .map_err(|_| Error::ControlThreadDead)?
    }

    pub fn get_frequency_correction(&self) -> Result<i32, Error> {
        self.handle.get_frequency_correction()
    }

    pub async fn set_frequency_correction(&self, ppm: i32) -> Result<(), Error> {
        let (result_sender, result_receiver) = oneshot::channel();
        self.control_queue_sender
            .send(ControlMessage::SetFrequencyCorrection {
                handle: self.handle.clone(),
                ppm,
                result_sender,
            })
            .await
            .map_err(|_| Error::ControlThreadDead)?;
        result_receiver
            .await
            .map_err(|_| Error::ControlThreadDead)?
    }

    pub fn get_offset_tuning(&self) -> Result<bool, Error> {
        self.handle.get_offset_tuning()
    }

    pub async fn set_offset_tuning(&self, enable: bool) -> Result<(), Error> {
        let (result_sender, result_receiver) = oneshot::channel();
        self.control_queue_sender
            .send(ControlMessage::SetOffsetTuning {
                handle: self.handle.clone(),
                enable,
                result_sender,
            })
            .await
            .map_err(|_| Error::ControlThreadDead)?;
        result_receiver
            .await
            .map_err(|_| Error::ControlThreadDead)?
    }

    pub fn get_rtl_xtal(&self) -> Result<u32, Error> {
        let (rtl_frequency, _tuner_frequency) = self.handle.get_xtal_frequency()?;
        Ok(rtl_frequency)
    }

    async fn set_xtal_frequency(
        &self,
        rtl_xtal_frequency: Option<u32>,
        tuner_xtal_frequency: Option<u32>,
    ) -> Result<(), Error> {
        let (result_sender, result_receiver) = oneshot::channel();
        self.control_queue_sender
            .send(ControlMessage::SetXtalFrequency {
                handle: self.handle.clone(),
                rtl_xtal_frequency,
                tuner_xtal_frequency,
                result_sender,
            })
            .await
            .map_err(|_| Error::ControlThreadDead)?;
        result_receiver
            .await
            .map_err(|_| Error::ControlThreadDead)?
    }

    pub async fn set_rtl_xtal(&self, frequency: u32) -> Result<(), Error> {
        self.set_xtal_frequency(Some(frequency), None).await
    }

    pub fn get_tuner_xtal(&self) -> Result<u32, Error> {
        let (_rtl_frequency, tuner_frequency) = self.handle.get_xtal_frequency()?;
        Ok(tuner_frequency)
    }

    pub async fn set_tuner_xtal(&self, frequency: u32) -> Result<(), Error> {
        self.set_xtal_frequency(None, Some(frequency)).await
    }

    pub async fn set_bias_tee(&self, enable: bool) -> Result<(), Error> {
        let (result_sender, result_receiver) = oneshot::channel();
        self.control_queue_sender
            .send(ControlMessage::SetBiasTee {
                handle: self.handle.clone(),
                pin: 0,
                enable,
                result_sender,
            })
            .await
            .map_err(|_| Error::ControlThreadDead)?;
        result_receiver
            .await
            .map_err(|_| Error::ControlThreadDead)?
    }
}

impl AsyncReadSamples for RtlSdr {
    type Error = Error;

    fn poll_read_samples(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &mut [IqSample],
    ) -> Poll<Result<usize, Self::Error>> {
        let buffer_out = buffer;

        loop {
            let this = self.deref_mut();

            if let Some(buffer_in) = &this.buffer {
                assert!(this.buffer_pos < buffer_in.len());

                if buffer_out.is_empty() {
                    return Poll::Ready(Ok(0));
                }

                let copy_amount = buffer_out.len().min(buffer_in.len() - this.buffer_pos);

                buffer_out[..copy_amount]
                    .copy_from_slice(&buffer_in[this.buffer_pos..][..copy_amount]);
                this.buffer_pos += copy_amount;

                if this.buffer_pos == buffer_in.len() {
                    this.buffer_pos = 0;
                    this.buffer = None;
                }

                return Poll::Ready(Ok(copy_amount));
            }
            else {
                assert_eq!(this.buffer_pos, 0);
                assert!(this.buffer.is_none());

                match this.buffer_queue_reader.poll_next(cx) {
                    Poll::Pending => return Poll::Pending,
                    Poll::Ready(None) => {
                        return Poll::Ready(Ok(0));
                    }
                    Poll::Ready(Some(buffer)) => {
                        this.buffer = Some(buffer);
                    }
                }
            }
        }
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

/// a call to rtlsdr_set_center_freq takes about 50ms. that's too long for async
/// - especially since we're shoving millions of samples per second at the same
/// time.
///
/// therefore we use a separate thread to run all the slow control commands.
/// we'll use one thread for all RtlSdr objects though.
fn control_thread(mut control_queue_receiver: mpsc::Receiver<ControlMessage>) {
    fn set_tuner_gain(handle: &Handle, gain: Gain) -> Result<(), Error> {
        match gain {
            Gain::Manual(gain) => {
                // manual gain mode must be enabled
                handle.set_tuner_gain_mode(true)?;

                // we need to find a supported gain value
                if let Some(gain) = handle
                    .tuner_gains
                    .iter()
                    .min_by_key(|supported| (**supported - gain).abs())
                {
                    handle.set_tuner_gain(*gain)?;
                    Ok(())
                }
                else {
                    Err(Error::NoSupportedGains)
                }
            }
            Gain::Auto => {
                handle.set_tuner_gain_mode(false)?;
                Ok(())
            }
        }
    }

    fn set_xtal_frequency(
        handle: &Handle,
        rtl_xtal_frequency: Option<u32>,
        tuner_xtal_frequency: Option<u32>,
    ) -> Result<(), Error> {
        let current = handle.get_xtal_frequency()?;
        handle.set_xtal_frequency(
            rtl_xtal_frequency.unwrap_or(current.0),
            tuner_xtal_frequency.unwrap_or(current.1),
        )
    }

    while let Some(command) = control_queue_receiver.blocking_recv() {
        match command {
            ControlMessage::SetCenterFrequency {
                handle,
                frequency,
                result_sender,
            } => {
                let result = handle.set_center_frequency(frequency);
                let _ = result_sender.send(result);
            }
            ControlMessage::SetSampleRate {
                handle,
                sample_rate,
                result_sender,
            } => {
                let result = handle.set_sample_rate(sample_rate);
                let _ = result_sender.send(result);
            }
            ControlMessage::SetTunerGain {
                handle,
                gain,
                result_sender,
            } => {
                let result = set_tuner_gain(&handle, gain);
                let _ = result_sender.send(result);
            }
            ControlMessage::SetTunerIfGain {
                handle,
                stage,
                gain,
                result_sender,
            } => {
                let result = handle.set_tuner_if_gain(stage, gain);
                let _ = result_sender.send(result);
            }
            ControlMessage::SetTunerBandwidth {
                handle,
                bandwidth,
                result_sender,
            } => {
                let result = handle.set_tuner_bandwidth(bandwidth);
                let _ = result_sender.send(result);
            }
            ControlMessage::SetAgcMode {
                handle,
                enable,
                result_sender,
            } => {
                let result = handle.set_agc_mode(enable);
                let _ = result_sender.send(result);
            }
            ControlMessage::SetFrequencyCorrection {
                handle,
                ppm,
                result_sender,
            } => {
                let result = handle.set_frequency_correction(ppm);
                let _ = result_sender.send(result);
            }
            ControlMessage::SetOffsetTuning {
                handle,
                enable,
                result_sender,
            } => {
                let result = handle.set_offset_tuning(enable);
                let _ = result_sender.send(result);
            }
            ControlMessage::SetXtalFrequency {
                handle,
                rtl_xtal_frequency,
                tuner_xtal_frequency,
                result_sender,
            } => {
                let result = set_xtal_frequency(&handle, rtl_xtal_frequency, tuner_xtal_frequency);
                let _ = result_sender.send(result);
            }
            ControlMessage::SetBiasTee {
                handle,
                pin,
                enable,
                result_sender,
            } => {
                let result = handle.set_bias_tee(pin, enable);
                let _ = result_sender.send(result);
            }
        }
    }
}

/// returns a sender to send commands to the control handler thread.
fn get_control_queue_sender() -> mpsc::Sender<ControlMessage> {
    const CONTROL_QUEUE_SIZE: usize = 128;

    static CONTROL_QUEUE_SENDER: OnceLock<mpsc::Sender<ControlMessage>> = OnceLock::new();
    let control_queue_sender = CONTROL_QUEUE_SENDER.get_or_init(|| {
        tracing::debug!("spawning control thread");

        let (control_queue_sender, control_queue_receiver) = mpsc::channel(CONTROL_QUEUE_SIZE);

        thread::spawn(move || {
            control_thread(control_queue_receiver);
        });

        control_queue_sender
    });

    control_queue_sender.clone()
}

enum ControlMessage {
    SetCenterFrequency {
        handle: Arc<Handle>,
        frequency: u32,
        result_sender: oneshot::Sender<Result<(), Error>>,
    },
    SetSampleRate {
        handle: Arc<Handle>,
        sample_rate: u32,
        result_sender: oneshot::Sender<Result<(), Error>>,
    },
    SetTunerGain {
        handle: Arc<Handle>,
        gain: Gain,
        result_sender: oneshot::Sender<Result<(), Error>>,
    },
    SetTunerIfGain {
        handle: Arc<Handle>,
        stage: i32,
        gain: i32,
        result_sender: oneshot::Sender<Result<(), Error>>,
    },
    SetTunerBandwidth {
        handle: Arc<Handle>,
        bandwidth: u32,
        result_sender: oneshot::Sender<Result<(), Error>>,
    },
    SetAgcMode {
        handle: Arc<Handle>,
        enable: bool,
        result_sender: oneshot::Sender<Result<(), Error>>,
    },
    SetFrequencyCorrection {
        handle: Arc<Handle>,
        ppm: i32,
        result_sender: oneshot::Sender<Result<(), Error>>,
    },
    SetOffsetTuning {
        handle: Arc<Handle>,
        enable: bool,
        result_sender: oneshot::Sender<Result<(), Error>>,
    },
    SetXtalFrequency {
        handle: Arc<Handle>,
        rtl_xtal_frequency: Option<u32>,
        tuner_xtal_frequency: Option<u32>,
        result_sender: oneshot::Sender<Result<(), Error>>,
    },
    SetBiasTee {
        handle: Arc<Handle>,
        pin: u8,
        enable: bool,
        result_sender: oneshot::Sender<Result<(), Error>>,
    },
}

fn reader_thread(mut buffer_queue_writer: buffer_queue::Writer, handle: Arc<Handle>) {
    // when we are reading to the buffer we don't hold the queue lock, so once we're
    // done we need to acquire the lock to add the buffer to the queue.
    // but we also need the queue lock to get a new free buffer. we can combine both
    // steps into one lock-holding code section at the start of the loop. All we
    // need to do is remember the buffer we want to push.
    let mut push_buffer = None;

    tracing::debug!("reader thread spawned");

    'outer: loop {
        let Some(mut buffer) = buffer_queue_writer.swap_buffers(push_buffer)
        else {
            // all readers dropped
            tracing::debug!("all readers dropped. exiting reader thread");
            break;
        };

        // this will clone, i.e. make a new buffer, if we can't get unique ownership of
        // it.
        let buffer_mut = Arc::make_mut(&mut buffer.data);
        let buffer_mut = bytemuck::cast_slice_mut(buffer_mut);

        loop {
            match handle.read_sync(buffer_mut) {
                Ok(n_read) => {
                    if n_read > 0 {
                        assert!(n_read & 1 == 0, "not an even amount of bytes :sobbing:");
                        buffer.filled = n_read >> 1;
                        push_buffer = Some(buffer);
                        break;
                    }
                    else {
                        tracing::debug!("rtlsdr_read_sync returned 0. exiting");
                        break 'outer;
                    }
                }
                Err(error) => {
                    tracing::error!(?error, "rtlsdr reader thread error");
                    break 'outer;
                }
            }
        }
    }
}

#[derive(Clone)]
struct Buffer {
    data: Arc<[IqSample]>,
    filled: usize,
}

impl Buffer {
    pub fn new(capacity: usize) -> Self {
        let data = std::iter::repeat_n(IqSample::default(), capacity).collect();
        Self { data, filled: 0 }
    }
}

impl Debug for Buffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Buffer").finish_non_exhaustive()
    }
}

impl Deref for Buffer {
    type Target = [IqSample];

    fn deref(&self) -> &Self::Target {
        &self.data[..self.filled]
    }
}

mod buffer_queue {
    use std::{
        collections::VecDeque,
        sync::Arc,
        task::{
            Context,
            Poll,
            Waker,
        },
    };

    use parking_lot::Mutex;

    use crate::bindings::Buffer;

    /// This is the central queue that passes buffers from the reader thread
    /// (producer) to the AsyncReadSamples impl (consumer).
    ///
    /// The items in the VecDeque are numbered head_pos..tail_pos from head to
    /// tail. Consumers have a read_pos that is relative to that numbering,
    /// so they'll know if they're lagging behind.
    struct Shared {
        num_writers: usize,
        num_readers: usize,
        slots: VecDeque<Buffer>,
        tail_pos: usize,
        head_pos: usize,
        capacity: usize,
        wakers: Vec<Waker>,
    }

    impl Shared {
        fn pop_buffer(&mut self) -> Option<Buffer> {
            if self.slots.len() == self.capacity {
                let buffer = self
                    .slots
                    .pop_front()
                    .expect("empty queue, but is at capacity");
                self.head_pos += 1;
                Some(buffer)
            }
            else {
                None
            }
        }

        fn push_buffer(&mut self, buffer: Buffer) {
            assert!(
                self.slots.len() < self.capacity,
                "expecting buffer queue to be below capacity when pushing"
            );
            self.slots.push_back(buffer);
            self.tail_pos += 1;
            for waker in self.wakers.drain(..) {
                waker.wake();
            }
        }
    }

    #[derive(derive_more::Debug)]
    pub struct Reader {
        #[debug(skip)]
        shared: Arc<Mutex<Shared>>,
        read_pos: usize,
    }

    impl Clone for Reader {
        fn clone(&self) -> Self {
            {
                let mut queue = self.shared.lock();
                queue.num_readers += 1;
            }

            Self {
                shared: self.shared.clone(),
                read_pos: self.read_pos,
            }
        }
    }

    impl Drop for Reader {
        fn drop(&mut self) {
            let mut queue = self.shared.lock();
            queue.num_readers -= 1;
        }
    }

    impl Reader {
        pub fn poll_next(&mut self, cx: &mut Context<'_>) -> Poll<Option<Buffer>> {
            let mut queue = self.shared.lock();

            if queue.num_writers == 0 {
                Poll::Ready(None)
            }
            else {
                let queue_index = if self.read_pos < queue.head_pos {
                    self.read_pos = queue.head_pos;
                    0
                }
                else {
                    self.read_pos - queue.head_pos
                };

                if self.read_pos < queue.tail_pos {
                    self.read_pos += 1;
                    Poll::Ready(Some(queue.slots[queue_index].clone()))
                }
                else {
                    queue.wakers.push(cx.waker().clone());
                    Poll::Pending
                }
            }
        }
    }

    #[derive(derive_more::Debug)]
    pub struct Writer {
        #[debug(skip)]
        shared: Arc<Mutex<Shared>>,
        buffer_size: usize,
    }

    impl Drop for Writer {
        fn drop(&mut self) {
            let mut queue = self.shared.lock();
            queue.num_writers -= 1;
        }
    }

    impl Writer {
        /// Returns a buffer to be filled with data. You can also pass in a
        /// buffer that you just filled. Returns None if all readers
        /// dropped.
        pub fn swap_buffers(&mut self, push_buffer: Option<Buffer>) -> Option<Buffer> {
            let mut queue = self.shared.lock();

            if queue.num_readers == 0 {
                None
            }
            else {
                // first push the buffer we filled in the last loop iteration
                if let Some(buffer) = push_buffer {
                    queue.push_buffer(buffer);
                }

                // get a free buffer from the queue, or make a new one
                let buffer = queue
                    .pop_buffer()
                    .unwrap_or_else(|| Buffer::new(self.buffer_size));

                Some(buffer)
            }
        }
    }

    pub fn channel(num_buffers: usize, buffer_size: usize) -> (Writer, Reader) {
        assert!(num_buffers > 0);
        assert!(buffer_size > 0);

        let shared = Arc::new(Mutex::new(Shared {
            num_readers: 1,
            num_writers: 1,
            slots: VecDeque::with_capacity(num_buffers),
            tail_pos: 0,
            head_pos: 0,
            capacity: num_buffers,
            wakers: vec![],
        }));

        (
            Writer {
                shared: shared.clone(),
                buffer_size,
            },
            Reader {
                shared,
                read_pos: 0,
            },
        )
    }
}
