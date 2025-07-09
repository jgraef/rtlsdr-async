use std::ffi::{
    CStr,
    c_char,
};

use crate::{
    Error,
    RtlSdr,
};

/// Returns an iterator over the available RTL-SDRs.
pub fn devices() -> DeviceIter {
    let device_count = unsafe { rtlsdr_sys::rtlsdr_get_device_count() };

    DeviceIter {
        device_count,
        index: 0,
    }
}

/// Iterator over available devices.
///
/// This yields [`DeviceInfo`]s.
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

            // todo: i think this returns garbage if you don't have permissions for the
            // device. test this.
            let device_name = unsafe { CStr::from_ptr(rtlsdr_sys::rtlsdr_get_device_name(index)) };

            if !device_name.is_empty() {
                let mut manufacturer = [0u8; 256];
                let mut product = [0u8; 256];
                let mut serial = [0u8; 256];

                let ret = unsafe {
                    rtlsdr_sys::rtlsdr_get_device_usb_strings(
                        index,
                        manufacturer.as_mut_ptr() as *mut c_char,
                        product.as_mut_ptr() as *mut c_char,
                        serial.as_mut_ptr() as *mut c_char,
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

/// RTL-SDR device information.
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

    /// Open the device
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
