use bytes::Buf;

use crate::{
    source::mode_s::DecodeError,
    util::BufReadBytesExt,
};

/// # TODO
///
/// Feel free to open a PR :3
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Message {
    pub data: [u8; 7],
}

impl Message {
    pub fn decode<B: Buf>(buffer: &mut B) -> Result<Self, DecodeError> {
        Ok(Self {
            data: buffer.get_bytes(),
        })
    }
}
