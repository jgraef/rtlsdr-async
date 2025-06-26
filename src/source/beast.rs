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

use bytes::{
    Bytes,
    BytesMut,
};
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
        state: State,
        //payload_buffer: BytesMut,
    }
}

impl<R> BeastReader<R> {
    pub fn new(reader: R) -> Self {
        Self {
            reader,
            state: Default::default(), //payload_buffer: BytesMut::new(),
        }
    }
}

impl<R: AsyncRead> BeastReader<R> {
    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<Option<Packet>, Error>> {
        loop {
            let this = self.as_mut().project();
            let state = this.state;

            assert!(state.receive_buffer_read_pos <= state.receive_buffer_write_pos);

            // we got an EOF while reading earlier
            if state.eof_reached {
                return Poll::Ready(Ok(None));
            }

            // first consume all bytes we already received from the underlying reader
            while let Some(byte) = state.next_received_byte() {
                if state.packet_escape_read {
                    // we already read the packet escape

                    if let Some(packet_type) = state.packet_type {
                        // we already read the packet type

                        while let Some(byte) = state.next_received_byte() {}
                    }
                    else {
                        // we didn't read the packet type yet, so this byte is it.
                        state.set_packet_type(byte);
                    }
                }
                else if byte == ESCAPE {
                    // we didn't receive a packet escape yet, but the current byte is one.
                    state.new_packet();
                }
                else {
                    // we didn't receive a packet escape yet, and the current byte isn't one. this
                    // is a protocol error.
                    todo!("garbage");
                }
            }

            let mut read_buf = ReadBuf::new(&mut state.receive_buffer);
            match this.reader.poll_read(cx, &mut read_buf) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(error)) => return Poll::Ready(Err(error.into())),
                Poll::Ready(Ok(())) => {
                    let n_read = read_buf.filled().len();
                    if n_read == 0 {
                        state.eof_reached = true;
                        continue;
                    }
                    state.receive_buffer_write_pos = n_read;
                }
            }
        }

        todo!()
    }
}

#[derive(Debug)]
struct State {
    eof_reached: bool,
    receive_buffer: [u8; RECEIVE_BUFFER_SIZE],
    receive_buffer_read_pos: usize,
    receive_buffer_write_pos: usize,
    packet_escape_read: bool,
    packet_type: Option<u8>,
    packet_buffer: [u8; PACKET_BUFFER_SIZE],
    packet_buffer_write_pos: usize,
    packet_expected_length: Option<usize>,
}

impl State {
    #[inline(always)]
    fn next_received_byte(&mut self) -> Option<u8> {
        if self.receive_buffer_read_pos < self.receive_buffer_write_pos {
            let byte = self.receive_buffer[self.receive_buffer_read_pos];
            self.receive_buffer_read_pos += 1;
            Some(byte)
        }
        else {
            None
        }
    }

    #[inline(always)]
    fn new_packet(&mut self) {
        self.packet_escape_read = true;
        self.packet_type = None;
        self.packet_buffer_write_pos = 0;
    }

    #[inline(always)]
    fn set_packet_type(&mut self, ty: u8) {
        self.packet_type = Some(ty);
        self.packet_expected_length = match ty {
            b'1' => Some(9),
            b'2' => Some(14),
            b'3' => Some(21),
            _ => None,
        };
    }
}

impl Default for State {
    fn default() -> Self {
        Self {
            eof_reached: false,
            receive_buffer: [0; RECEIVE_BUFFER_SIZE],
            receive_buffer_read_pos: 0,
            receive_buffer_write_pos: 0,
            packet_escape_read: false,
            packet_type: None,
            packet_buffer: [0; PACKET_BUFFER_SIZE],
            packet_buffer_write_pos: 0,
            packet_expected_length: None,
        }
    }
}

#[derive(Clone, Debug)]
pub enum Packet {
    ModeAc {
        timestamp: MlatTimestamp,
        signal_level: SignalLevel,
    },
}

pub type MlatTimestamp = [u8; 6];
pub type SignalLevel = u8;
