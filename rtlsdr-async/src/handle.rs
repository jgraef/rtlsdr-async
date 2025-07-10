use std::{
    ffi::{
        c_int,
        c_void,
    },
    fmt::Debug,
    ptr::null_mut,
};

use parking_lot::{
    Mutex,
    MutexGuard,
};

use crate::{
    DirectSamplingMode,
    Error,
    TunerGainMode,
    TunerType,
};

/// This used to be somewhat unsafe, but now it isn't anymore!
/// All operations on the Handle are synchronized using a Mutex.
///
/// Be aware though that many control operations and of course reads take a bit
/// of time. That's why they're handling in their respective threads.
#[derive(Debug)]
pub(crate) struct Handle {
    // holds some state we want to maintain for this handle. but this should always be used when
    // interacting with the handle. thus this contains the handle.
    locked: Mutex<LockedHandle>,

    pub index: u32,
    pub tuner_type: TunerType,
    pub tuner_gains: TunerGains,
}

unsafe impl Send for Handle {}
unsafe impl Sync for Handle {}

impl Handle {
    pub fn open(index: u32) -> Result<Self, Error> {
        let mut handle: rtlsdr_sys::rtlsdr_dev_t = null_mut();
        let ret =
            unsafe { rtlsdr_sys::rtlsdr_open(&mut handle as *mut rtlsdr_sys::rtlsdr_dev_t, index) };
        tracing::debug!(?index, ?ret, "rtlsdr_open");
        if ret != 0 {
            return Err(Error::from_lib("rtlsdr_open", ret));
        }
        assert!(
            !handle.is_null(),
            "rtlsdr_open returned 0, but handle is still NULL"
        );

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
                // instead of failing we could just allocate.
                tracing::warn!(
                    ?num_gains,
                    capacity = TunerGains::CAPACITY,
                    "bug: number of tuner gains available exceeds capacity."
                );
            }
        }
        tracing::debug!(gains = ?tuner_gains, "rtlsdr_get_tuner_gains");

        // this is needed for reading to work
        // note: only fails if the dev pointer is null, which it is not
        let ret = unsafe { rtlsdr_sys::rtlsdr_reset_buffer(handle) };
        assert_eq!(ret, 0, "rtlsdr_reset_buffer didn't return 0");

        Ok(Handle {
            locked: Mutex::new(LockedHandle {
                handle,
                tuner_gain_mode: None,
            }),
            index,
            tuner_type,
            tuner_gains,
        })
    }

    pub fn lock(&self) -> MutexGuard<'_, LockedHandle> {
        self.locked.lock()
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        tracing::debug!(index = self.index, "rtl_sdr_close");
        let state = self.locked.lock();
        unsafe {
            rtlsdr_sys::rtlsdr_close(state.handle);
        }
    }
}

#[derive(Debug)]
pub(crate) struct LockedHandle {
    handle: rtlsdr_sys::rtlsdr_dev_t,

    /// the tuner gain mode we set previously. we store this so we can skip
    /// setting it if we would set it to the same mode gain. librtlsdr doesn't
    /// do this check. initially we don't know the mode, so this is an Option.
    tuner_gain_mode: Option<TunerGainMode>,
}

impl LockedHandle {
    pub fn get_center_frequency(&mut self) -> Result<u32, Error> {
        let ret = unsafe { rtlsdr_sys::rtlsdr_get_center_freq(self.handle) };
        tracing::debug!(ret, "rtlsdr_get_center_freq");
        if ret == 0 {
            Err(Error::from_lib("rtlsdr_get_center_freq", 0))
        }
        else {
            Ok(ret)
        }
    }

    pub fn set_center_frequency(&mut self, frequency: u32) -> Result<(), Error> {
        let ret = unsafe { rtlsdr_sys::rtlsdr_set_center_freq(self.handle, frequency) };
        tracing::debug!(ret, frequency, "rtlsdr_set_center_freq");
        if ret == 0 {
            Ok(())
        }
        else {
            Err(Error::from_lib("rtlsdr_set_center_freq", ret))
        }
    }

    pub fn get_sample_rate(&mut self) -> Result<u32, Error> {
        // this returns 0 if dev is NULL, but it isn't. otherwise it straight up gives
        // us dev->rate. dev->rate might be 0 if the sample rate hasn't been set
        // yet. so should we return a Result, Option, or just plain u32?
        let ret = unsafe { rtlsdr_sys::rtlsdr_get_sample_rate(self.handle) };
        tracing::trace!(ret, "rtlsdr_get_sample_rate");
        if ret != 0 {
            Ok(ret)
        }
        else {
            Err(Error::from_lib("rtlsdr_get_sample_rate", ret as i32))
        }
    }

    pub fn set_sample_rate(&mut self, sample_rate: u32) -> Result<(), Error> {
        let ret = unsafe { rtlsdr_sys::rtlsdr_set_sample_rate(self.handle, sample_rate) };
        tracing::debug!(ret, sample_rate, "rtlsdr_set_sample_rate");
        if ret == 0 {
            Ok(())
        }
        else {
            Err(Error::from_lib("rtlsdr_set_sample_rate", ret))
        }
    }

    pub fn set_tuner_gain_mode(&mut self, mode: TunerGainMode) -> Result<(), Error> {
        // if the mode is already set, don't set it again
        if self
            .tuner_gain_mode
            .map_or(false, |current| current == mode)
        {
            return Ok(());
        }

        let ret = unsafe {
            rtlsdr_sys::rtlsdr_set_tuner_gain_mode(
                self.handle,
                match mode {
                    TunerGainMode::Manual => 1,
                    TunerGainMode::Auto => 0,
                },
            )
        };
        tracing::debug!(ret, ?mode, "rtlsdr_set_tuner_gain_mode");
        if ret == 0 {
            self.tuner_gain_mode = Some(mode);
            Ok(())
        }
        else {
            Err(Error::from_lib("rtlsdr_set_tuner_gain_mode", ret))
        }
    }

    pub fn get_tuner_gain(&mut self) -> Result<i32, Error> {
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

    pub fn set_tuner_gain(&mut self, gain: i32) -> Result<(), Error> {
        let ret = unsafe { rtlsdr_sys::rtlsdr_set_tuner_gain(self.handle, gain) };
        tracing::debug!(ret, gain, "rtlsdr_set_tuner_gain");
        if ret == 0 {
            Ok(())
        }
        else {
            Err(Error::from_lib("rtlsdr_set_tuner_gain", ret))
        }
    }

    pub fn set_tuner_if_gain(&mut self, stage: i32, gain: i32) -> Result<(), Error> {
        let ret = unsafe { rtlsdr_sys::rtlsdr_set_tuner_if_gain(self.handle, stage, gain) };
        tracing::debug!(ret, gain, "rtlsdr_set_tuner_if_gain");
        if ret == 0 {
            Ok(())
        }
        else {
            Err(Error::from_lib("rtlsdr_set_tuner_if_gain", ret))
        }
    }

    pub fn set_tuner_bandwidth(&mut self, bandwidth: u32) -> Result<(), Error> {
        let ret = unsafe { rtlsdr_sys::rtlsdr_set_tuner_bandwidth(self.handle, bandwidth) };
        tracing::debug!(ret, bandwidth, "rtlsdr_set_tuner_bandwidth");
        if ret == 0 {
            Ok(())
        }
        else {
            Err(Error::from_lib("rtlsdr_set_tuner_bandwidth", ret))
        }
    }

    pub fn set_agc_mode(&mut self, enable: bool) -> Result<(), Error> {
        let ret = unsafe { rtlsdr_sys::rtlsdr_set_agc_mode(self.handle, enable as i32) };
        tracing::debug!(ret, ?enable, "rtlsdr_set_agc_mode");
        if ret == 0 {
            Ok(())
        }
        else {
            Err(Error::from_lib("rtlsdr_set_agc_mode", ret))
        }
    }

    pub fn get_frequency_correction(&mut self) -> Result<i32, Error> {
        let ret = unsafe { rtlsdr_sys::rtlsdr_get_freq_correction(self.handle) };
        tracing::debug!(ret, "rtlsdr_get_freq_correction");
        // note: only returns errors for dev=null, and besides 0 is a valid return value
        Ok(ret)
    }

    pub fn set_frequency_correction(&mut self, ppm: i32) -> Result<(), Error> {
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

    pub fn get_offset_tuning(&mut self) -> Result<bool, Error> {
        let ret = unsafe { rtlsdr_sys::rtlsdr_get_offset_tuning(self.handle) };
        tracing::debug!(ret, "rtlsdr_get_offset_tuning");
        // note: only returns errors for dev=null
        assert!(ret == 0 || ret == 1);
        Ok(ret == 1)
    }

    pub fn set_offset_tuning(&mut self, enable: bool) -> Result<(), Error> {
        let ret = unsafe { rtlsdr_sys::rtlsdr_set_offset_tuning(self.handle, enable as i32) };
        tracing::debug!(ret, ?enable, "rtlsdr_set_offset_tuning");
        if ret == 0 {
            Ok(())
        }
        else {
            Err(Error::from_lib("rtlsdr_set_offset_tuning", ret))
        }
    }

    pub fn get_xtal_frequency(&mut self) -> Result<(u32, u32), Error> {
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

    pub fn set_xtal_frequency(
        &mut self,
        rtl_frequency: u32,
        tuner_frequency: u32,
    ) -> Result<(), Error> {
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

    pub fn set_bias_tee(&mut self, pin: u8, enable: bool) -> Result<(), Error> {
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

    pub fn get_direct_sampling(&mut self) -> Result<Option<DirectSamplingMode>, Error> {
        let ret = unsafe { rtlsdr_sys::rtlsdr_get_direct_sampling(self.handle) };
        tracing::trace!(ret, "rtlsdr_get_direct_sampling");
        match ret {
            0 => Ok(None),
            1 => Ok(Some(DirectSamplingMode::I)),
            2 => Ok(Some(DirectSamplingMode::Q)),
            _ => Err(Error::from_lib("rtlsdr_get_direct_sampling", ret)),
        }
    }

    pub fn set_direct_sampling(&mut self, mode: Option<DirectSamplingMode>) -> Result<(), Error> {
        // librtlsdr doesn't do this
        let current = self.get_direct_sampling()?;
        if current == mode {
            return Ok(());
        }

        let mode_value = match mode {
            None => 0,
            Some(DirectSamplingMode::I) => 1,
            Some(DirectSamplingMode::Q) => 2,
        };
        let ret = unsafe { rtlsdr_sys::rtlsdr_set_direct_sampling(self.handle, mode_value) };
        tracing::debug!(ret, ?mode, "rtlsdr_set_direct_sampling");
        if ret == 0 {
            Ok(())
        }
        else {
            Err(Error::from_lib("rtlsdr_set_direct_sampling", ret))
        }
    }

    /// this is synchronized with the rest of the methods on this.
    /// initially it wasn't to allow for asynchronous control and sampling. but
    /// i believe it's better this was, as this way we know exactly what state
    /// the handle is in when sampling.
    ///
    /// a quick test showed that this usually fills the whole buffer.
    pub fn read_sync(&mut self, buffer: &mut [u8]) -> Result<usize, Error> {
        let mut n_read = 0;

        let ret = unsafe {
            rtlsdr_sys::rtlsdr_read_sync(
                self.handle,
                buffer.as_mut_ptr() as *mut c_void,
                buffer
                    .len()
                    .try_into()
                    .expect("buffer size too large for i32"),
                &mut n_read as *mut c_int,
            )
        };

        if ret == 0 {
            Ok(n_read.try_into().unwrap())
        }
        else {
            Err(Error::from_lib("rtlsdr_read_sync", ret))
        }
    }
}

// todo: we could make this type public
#[derive(Clone, Copy)]
pub(crate) struct TunerGains {
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

    pub fn iter(&self) -> std::slice::Iter<'_, i32> {
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
