//! # Format
//!
//! https://wiki.jetvision.de/wiki/Mode-S_Beast:Data_Output_Formats
//! https://github.com/wiedehopf/readsb/blob/75decb53c0e66f4c12cf24127578a3fe7d919219/net_io.c#L4885
//! 0xe3: https://github.com/wiedehopf/readsb/blob/75decb53c0e66f4c12cf24127578a3fe7d919219/net_io.c#L1754
//!
//! # decoding ADS-B
//!
//! https://docs.rs/adsb_deku/latest/adsb_deku/

use std::{
    pin::Pin,
    task::{
        Context,
        Poll,
    },
};

use bytes::Buf;
use pin_project_lite::pin_project;
use tokio::io::{
    AsyncRead,
    ReadBuf,
};

// todo: make this the max size of any packets we can recognize
const RECEIVE_BUFFER_SIZE: usize = 512;
const PACKET_BUFFER_SIZE: usize = 512;
const ESCAPE: u8 = 0x1a;

#[derive(Debug, thiserror::Error)]
#[error("beast decode error")]
pub enum Error {
    Io(#[from] std::io::Error),
}

pin_project! {
    #[derive(Debug)]
    pub struct BeastReader<R> {
        #[pin]
        reader: R,
        receive_buffer: ReceiveBuffer,
        decoder: PacketDecoder,
    }
}

impl<R> BeastReader<R> {
    pub fn new(reader: R) -> Self {
        Self {
            reader,
            receive_buffer: Default::default(),
            decoder: Default::default(),
        }
    }
}

impl<R: AsyncRead> BeastReader<R> {
    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<Option<Packet>, Error>> {
        loop {
            let mut this = self.as_mut().project();

            if this.receive_buffer.has_data() {
                if let Some(packet) = this.decoder.decode_next(&mut this.receive_buffer)? {
                    return Poll::Ready(Ok(Some(packet)));
                }
            }
            else {
                // if there is no data in the receiver buffer, we need to receive some
                this.receive_buffer.reset();

                let mut read_buf = ReadBuf::new(&mut this.receive_buffer.buffer);
                match this.reader.poll_read(cx, &mut read_buf) {
                    Poll::Pending => return Poll::Pending,
                    Poll::Ready(Err(error)) => return Poll::Ready(Err(error.into())),
                    Poll::Ready(Ok(())) => {
                        let num_bytes_read = read_buf.filled().len();

                        // if no data was received, the underlying reader reached EOF
                        if num_bytes_read == 0 {
                            return Poll::Ready(Ok(None));
                        }

                        this.receive_buffer.num_bytes = num_bytes_read;
                    }
                }
            }
        }
    }
}

#[derive(Debug)]
struct ReceiveBuffer {
    buffer: [u8; RECEIVE_BUFFER_SIZE],
    read_pos: usize,
    num_bytes: usize,
}

impl Default for ReceiveBuffer {
    fn default() -> Self {
        Self {
            buffer: [0; RECEIVE_BUFFER_SIZE],
            read_pos: 0,
            num_bytes: 0,
        }
    }
}

impl ReceiveBuffer {
    #[inline(always)]
    fn has_data(&self) -> bool {
        self.read_pos < self.num_bytes
    }

    #[inline(always)]
    fn reset(&mut self) {
        self.read_pos = 0;
        self.num_bytes = 0;
    }

    #[inline(always)]
    fn next_byte(&mut self) -> Option<u8> {
        self.has_data().then(|| {
            let byte = self.buffer[self.read_pos];
            self.read_pos += 1;
            byte
        })
    }
}

#[derive(Debug)]
struct PacketDecoder {
    leading_escape_read: bool,
    tag: Option<u8>,
    is_known_tag: bool,
    buffer: [u8; PACKET_BUFFER_SIZE],
    buffer_write_pos: usize,
    expected_length: Option<usize>,
    read_incomplete_escape: bool,
}

impl PacketDecoder {
    /// Reads from the receiver_buffer as much as needed to decode one packet.
    /// if there isn't enough data in the buffer, returns None.
    ///
    /// This will read as much as possible. So if None is returned all data has
    /// been read and the receive_buffer can be cleared.
    ///
    /// The decoder keeps track of any partially decoded packets and the next
    /// invokation of this method will resume decoding.
    fn decode_next(&mut self, receive_buffer: &mut ReceiveBuffer) -> Result<Option<Packet>, Error> {
        while let Some(byte) = receive_buffer.next_byte() {
            if self.leading_escape_read {
                // we already read the packet escape

                if self.tag.is_some() {
                    // we already read the packet type

                    let mut emit_packet = false;
                    if self.read_incomplete_escape {
                        // we read an escape before, but we don't know what follows yet.
                        if byte == ESCAPE {
                            // double escape
                            self.push_byte(ESCAPE);
                            self.read_incomplete_escape = false;
                        }
                        else {
                            // the escape we read was the start of a new packet
                            self.leading_escape_read = true;
                            self.set_packet_type(byte);
                            emit_packet = true;
                        }
                    }
                    else {
                        // payload byte
                        self.push_byte(byte);
                    }

                    if let Some(expected_length) = self.expected_length {
                        if self.buffer_write_pos == expected_length {
                            emit_packet = true;
                        }
                    }

                    if emit_packet {
                        if let Some(packet) = self.emit_packet()? {
                            return Ok(Some(packet));
                        }
                    }
                }
                else {
                    // if we read an escape here something is messed up
                    if byte == ESCAPE {
                        todo!("error");
                    }

                    // we didn't read the packet type yet, so this byte is it.
                    self.set_packet_type(byte);
                }
            }
            else if byte == ESCAPE {
                // we didn't receive a packet escape yet, but the current byte is one.
                self.new_packet();
            }
            else {
                // we didn't receive a packet escape yet, and the current byte isn't one.
                // this is a protocol error.
                todo!("garbage");
            }
        }

        Ok(None)
    }

    #[inline(always)]
    fn new_packet(&mut self) {
        self.leading_escape_read = true;
        self.tag = None;
        self.buffer_write_pos = 0;
    }

    #[inline(always)]
    fn set_packet_type(&mut self, tag: u8) {
        self.tag = Some(tag);
        match tag {
            b'1' => {
                self.expected_length = Some(9);
                self.is_known_tag = true;
            }
            b'2' => {
                self.expected_length = Some(14);
                self.is_known_tag = true;
            }
            b'3' => {
                self.expected_length = Some(21);
                self.is_known_tag = true;
            }
            _ => {}
        }
    }

    #[inline(always)]
    fn push_byte(&mut self, byte: u8) {
        if self.is_known_tag {
            self.buffer[self.buffer_write_pos] = byte;
            self.buffer_write_pos += 1;
        }
    }

    #[inline(always)]
    fn emit_packet(&mut self) -> Result<Option<Packet>, Error> {
        assert!(!self.read_incomplete_escape);
        assert!(self.leading_escape_read);

        let tag = self.tag.expect("emitting packet without having read a tag");

        if self.is_known_tag {
            if let Some(expected_length) = self.expected_length {
                if self.buffer_write_pos != expected_length {
                    todo!("error, invalid length");
                }
            }

            fn read_bytes<B: Buf, const N: usize>(buffer: &mut B) -> [u8; N] {
                let mut data: [u8; N] = [0; N];
                buffer.copy_to_slice(&mut data[..]);
                data
            }

            let mut buffer = &self.buffer[..self.buffer_write_pos];
            let packet = match tag {
                b'1' => {
                    Packet::ModeAc {
                        timestamp: read_bytes(&mut buffer),
                        signal_level: buffer.get_u8(),
                        data: read_bytes(&mut buffer),
                    }
                }
                b'2' => {
                    Packet::ModeSShort {
                        timestamp: read_bytes(&mut buffer),
                        signal_level: buffer.get_u8(),
                        data: read_bytes(&mut buffer),
                    }
                }
                b'3' => {
                    Packet::ModeSLong {
                        timestamp: read_bytes(&mut buffer),
                        signal_level: buffer.get_u8(),
                        data: read_bytes(&mut buffer),
                    }
                }
                _ => unreachable!("is_known_tag is set, but tag was not matched"),
            };

            // reset decoder state
            self.leading_escape_read = false;
            self.tag = None;
            self.is_known_tag = false;
            self.buffer_write_pos = 0;
            self.expected_length = None;

            Ok(Some(packet))
        }
        else {
            Ok(None)
        }
    }
}

impl Default for PacketDecoder {
    fn default() -> Self {
        Self {
            leading_escape_read: false,
            tag: None,
            is_known_tag: false,
            buffer: [0; PACKET_BUFFER_SIZE],
            buffer_write_pos: 0,
            expected_length: None,
            read_incomplete_escape: false,
        }
    }
}

#[derive(Clone, Debug)]
pub enum Packet {
    ModeAc {
        timestamp: MlatTimestamp,
        signal_level: SignalLevel,
        data: [u8; 2],
    },
    ModeSShort {
        timestamp: MlatTimestamp,
        signal_level: SignalLevel,
        data: [u8; 7],
    },
    ModeSLong {
        timestamp: MlatTimestamp,
        signal_level: SignalLevel,
        data: [u8; 14],
    },
}

pub type MlatTimestamp = [u8; 6];
pub type SignalLevel = u8;
