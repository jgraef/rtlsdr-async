use bytes::Buf;

use crate::source::mode_s::DecodeError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Message {}

impl Message {
    pub fn decode<B: Buf>(buffer: &mut B) -> Result<Self, DecodeError> {
        todo!();
    }
}
