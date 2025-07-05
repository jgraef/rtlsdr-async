use std::{
    collections::VecDeque,
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
    sync::Arc,
    task::{
        Context,
        Poll,
        Waker,
    },
    thread,
};

use parking_lot::Mutex;
use rtlsdr_sys::rtlsdr_read_sync;

use crate::{
    AsyncReadSamples,
    Configure,
    Gain,
    IqSample,
};

const DEFAULT_BUFFER_SIZE: usize = 0x4000; // 16 KiB
const DEFAULT_QUEUE_SIZE: usize = 64; // total of 1 MiB buffers

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("librtlsdr error: {0}")]
    LibRtlSdr(i32),
    #[error("device handler thread died unexpectedly")]
    DeviceThreadDead,
}

impl Error {
    pub fn from_lib(value: i32) -> Self {
        Self::LibRtlSdr(value)
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
struct Handle(rtlsdr_sys::rtlsdr_dev_t);

unsafe impl Send for Handle {}
unsafe impl Sync for Handle {}

impl Handle {
    fn open(index: u32) -> Result<Self, Error> {
        let mut handle: rtlsdr_sys::rtlsdr_dev_t = null_mut();
        let ret =
            unsafe { rtlsdr_sys::rtlsdr_open(&mut handle as *mut rtlsdr_sys::rtlsdr_dev_t, index) };
        if ret == 0 {
            Ok(Handle(handle))
        }
        else {
            Err(Error::from_lib(ret))
        }
    }

    fn set_center_frequency(&self, frequency: u32) -> Result<(), Error> {
        let ret = unsafe { rtlsdr_sys::rtlsdr_set_center_freq(self.0, frequency) };
        if ret == 0 {
            Ok(())
        }
        else {
            Err(Error::from_lib(ret))
        }
    }

    fn set_sample_rate(&self, sample_rate: u32) -> Result<(), Error> {
        let ret = unsafe { rtlsdr_sys::rtlsdr_set_sample_rate(self.0, sample_rate) };
        if ret == 0 {
            Ok(())
        }
        else {
            Err(Error::from_lib(ret))
        }
    }

    fn set_tuner_gain_mode(&self, manual: bool) -> Result<(), Error> {
        let ret = unsafe { rtlsdr_sys::rtlsdr_set_tuner_gain_mode(self.0, manual as i32) };
        if ret == 0 {
            Ok(())
        }
        else {
            Err(Error::from_lib(ret))
        }
    }

    fn set_tuner_gain(&self, gain: u32) -> Result<(), Error> {
        let ret = unsafe { rtlsdr_sys::rtlsdr_set_tuner_gain(self.0, gain as i32) };
        if ret == 0 {
            Ok(())
        }
        else {
            Err(Error::from_lib(ret))
        }
    }

    fn set_agc_mode(&self, enabled: bool) -> Result<(), Error> {
        let ret = unsafe { rtlsdr_sys::rtlsdr_set_agc_mode(self.0, enabled as i32) };
        if ret == 0 {
            Ok(())
        }
        else {
            Err(Error::from_lib(ret))
        }
    }

    fn read_sync(&self, buffer: &mut [u8]) -> Result<usize, Error> {
        let mut n_read: i32 = 0;

        let ret = unsafe {
            rtlsdr_read_sync(
                self.0,
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
            Err(Error::from_lib(ret))
        }
    }

    fn reset_buffer(&self) {
        // note: only fails if the dev pointer is null, which it is not
        let ret = unsafe { rtlsdr_sys::rtlsdr_reset_buffer(self.0) };
        assert_eq!(ret, 0, "rtlsdr_reset_buffer didn't return 0");
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            rtlsdr_sys::rtlsdr_close(self.0);
        }
    }
}

impl Debug for Handle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Handle").finish_non_exhaustive()
    }
}

struct Shared {
    /// mutex for control handle
    /// not used by reader thread
    control_lock: Mutex<()>,

    /// rtlsdr device handle
    handle: Handle,

    /// queue that holds buffers that has been read by the reader thread
    queue: Mutex<BufferQueue>,
}

#[derive(Clone)]
pub struct RtlSdr {
    /// shared state
    shared: Arc<Shared>,

    /// our read position in the buffer queue
    queue_read_pos: usize,

    /// the buffer if we currently have one
    buffer: Option<Buffer>,

    // read position in buffer
    buffer_pos: usize,
}

impl RtlSdr {
    pub fn open(index: u32) -> Result<Self, Error> {
        Self::open_impl(index, DEFAULT_QUEUE_SIZE, DEFAULT_BUFFER_SIZE)
    }

    fn open_impl(index: u32, queue_size: usize, buffer_size: usize) -> Result<Self, Error> {
        let handle = Handle::open(index)?;
        handle.reset_buffer();

        let shared = Arc::new(Shared {
            control_lock: Mutex::new(()),
            handle,
            queue: Mutex::new(BufferQueue::new(queue_size)),
        });

        thread::spawn({
            let shared = shared.clone();
            move || {
                reader_thread(shared, buffer_size);
            }
        });

        Ok(Self {
            shared,
            queue_read_pos: 0,
            buffer: None,
            buffer_pos: 0,
        })
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

                let mut queue = this.shared.queue.lock();

                match queue.poll_read(&mut this.queue_read_pos, cx) {
                    Poll::Pending => return Poll::Pending,
                    Poll::Ready(buffer) => {
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
        let _guard = self.shared.control_lock.lock();
        self.shared.handle.set_center_frequency(frequency)?;
        Ok(())
    }

    async fn set_sample_rate(&mut self, sample_rate: u32) -> Result<(), Error> {
        let _guard = self.shared.control_lock.lock();
        self.shared.handle.set_sample_rate(sample_rate)?;
        Ok(())
    }

    async fn set_gain(&mut self, gain: Gain) -> Result<(), Error> {
        let _guard = self.shared.control_lock.lock();
        match gain {
            Gain::Manual(gain) => {
                self.shared.handle.set_tuner_gain_mode(true)?;
                self.shared.handle.set_tuner_gain(gain)?;
            }
            Gain::Auto => {
                self.shared.handle.set_tuner_gain_mode(false)?;
            }
        }
        Ok(())
    }

    async fn set_agc_mode(&mut self, enabled: bool) -> Result<(), Error> {
        let _guard = self.shared.control_lock.lock();
        self.shared.handle.set_agc_mode(enabled)?;
        Ok(())
    }
}

fn reader_thread(shared: Arc<Shared>, buffer_size: usize) {
    loop {
        let mut queue = shared.queue.lock();

        let mut buffer = queue
            .pop_buffer()
            .unwrap_or_else(|| Buffer::new(buffer_size));
        let buffer_mut = Arc::make_mut(&mut buffer.data);
        let buffer_mut = bytemuck::cast_slice_mut(buffer_mut);
        drop(queue);

        loop {
            match shared.handle.read_sync(buffer_mut) {
                Ok(n_read) => {
                    if n_read > 0 {
                        assert!(n_read & 1 == 0, "not an even amount of samples :sobbing:");
                        buffer.filled = n_read >> 1;
                        break;
                    }
                }
                Err(error) => todo!("handle errors: {error:?}"),
            }
        }

        let mut queue = shared.queue.lock();
        queue.push_buffer(buffer);
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

/// This is the central queue that passes buffers from the reader thread
/// (producer) to the AsyncReadSamples impl (consumer).
///
/// The items in the VecDeque are numbered head_pos..tail_pos from head to tail.
/// Consumers have a read_pos that is relative to that numbering, so they'll
/// know if they're lagging behind.
struct BufferQueue {
    slots: VecDeque<Buffer>,
    tail_pos: usize,
    head_pos: usize,
    capacity: usize,
    wakers: Vec<Waker>,
}

impl BufferQueue {
    fn new(capacity: usize) -> Self {
        assert!(capacity > 0);

        Self {
            slots: VecDeque::with_capacity(capacity),
            tail_pos: 0,
            head_pos: 0,
            capacity,
            wakers: vec![],
        }
    }

    fn poll_read(&mut self, read_pos: &mut usize, cx: &mut Context<'_>) -> Poll<Buffer> {
        let queue_index = if *read_pos < self.head_pos {
            *read_pos = self.head_pos;
            0
        }
        else {
            *read_pos - self.head_pos
        };

        if *read_pos < self.tail_pos {
            *read_pos += 1;
            Poll::Ready(self.slots[queue_index].clone())
        }
        else {
            self.wakers.push(cx.waker().clone());
            Poll::Pending
        }
    }

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
