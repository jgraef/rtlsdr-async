//! BEAST output format decoder

use std::{
    pin::Pin,
    task::{
        Context,
        Poll,
    },
};

use bytes::Buf;
use futures_util::Stream;
use pin_project_lite::pin_project;
use tokio::io::{
    AsyncRead,
    ReadBuf,
};
use uuid::Uuid;

use crate::{
    source::beast::{
        ESCAPE,
        Error,
        MlatTimestamp,
        PacketDecode,
        PacketType,
        SignalLevel,
    },
    util::BufReadBytesExt,
};

/// this can be larger for more efficient reads, although the underlying reader
/// is probably buffered.
const RECEIVE_BUFFER_SIZE: usize = 512;

/// this only needs to be able to hold any packet we decode
const PACKET_BUFFER_SIZE: usize = 64;

/// Standard are 1, 2, 3, 4 (see [doc](https://wiki.jetvision.de/wiki/Mode-S_Beast:Data_Output_Formats))
///
/// 1 conflicts:
///  config? <https://github.com/wiedehopf/readsb/blob/75decb53c0e66f4c12cf24127578a3fe7d919219/net_io.c#L3889>
///  heartbeat: <https://github.com/wiedehopf/readsb/blob/75decb53c0e66f4c12cf24127578a3fe7d919219/net_io.c#L108>
///  i think this references the commands a Mode-S BEAST device accepts:
///  <https://wiki.jetvision.de/wiki/Mode-S_Beast:Data_Input_Formats>
///
/// 4: 1 byte - [DIP switches](https://wiki.jetvision.de/wiki/Mode-S_Beast:Data_Input_Formats)
///
/// 5: unknown
///   <https://github.com/wiedehopf/readsb/blob/75decb53c0e66f4c12cf24127578a3fe7d919219/net_io.c#L5043>
///
/// H: HULC extension
///  <https://static.avionix-tech.com/statics/cms/2023-11-21/GNS5894T_ADSB_Module_datasheet_V1.1.pdf> (page 17)
///  0x1A : 0x48 : ID : LEN : DATA
///  is this worth implementing?
///
/// P: ping. code comments reference a 'p as well, but can't find actual code.
///  <https://github.com/wiedehopf/readsb/blob/75decb53c0e66f4c12cf24127578a3fe7d919219/net_io.c#L5050>
///  <https://github.com/wiedehopf/readsb/blob/75decb53c0e66f4c12cf24127578a3fe7d919219/net_io.c#L3889>
///
/// W:
///  looks like config. is this output?. 1 byte data:
///  <https://github.com/wiedehopf/readsb/blob/75decb53c0e66f4c12cf24127578a3fe7d919219/net_io.c#L447>
///  <https://github.com/wiedehopf/readsb/blob/75decb53c0e66f4c12cf24127578a3fe7d919219/net_io.c#L1481>
///  <https://github.com/wiedehopf/readsb/blob/75decb53c0e66f4c12cf24127578a3fe7d919219/net_io.c#L3889>
///
/// 0xe3: <https://github.com/wiedehopf/readsb/blob/75decb53c0e66f4c12cf24127578a3fe7d919219/net_io.c#L4982>
///
/// 0xe4:
///  this seems to be a string encoded uuid. purpose unknown.
///  <https://github.com/wiedehopf/readsb/blob/75decb53c0e66f4c12cf24127578a3fe7d919219/net_io.c#L433>
///  <https://github.com/wiedehopf/readsb/blob/75decb53c0e66f4c12cf24127578a3fe7d919219/net_io.c#L5045>
///
/// 0xe8: <https://github.com/wiedehopf/readsb/blob/75decb53c0e66f4c12cf24127578a3fe7d919219/net_io.c#L4920>
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputPacketType {
    ModeAc,
    ModeSShort,
    ModeSLong,
    DipSwitches,
    Ping,
    ReceiverId,
    Unknown(u8),
}

impl PacketType for OutputPacketType {
    type Packet = OutputPacket;

    fn from_byte(byte: u8) -> Self {
        match byte {
            b'1' => Self::ModeAc,
            b'2' => Self::ModeSShort,
            b'3' => Self::ModeSLong,
            b'4' => Self::DipSwitches,
            //b'P' => Self::Ping, // todo: uppercase P is an input command
            //0xe3 => Self::ReceiverId,
            _ => {
                todo!("unknown packet type: {:02x}", byte);
                //Self::Unknown(byte)
            }
        }
    }

    fn expected_length(&self) -> Option<usize> {
        match self {
            OutputPacketType::ModeAc => Some(9),
            OutputPacketType::ModeSShort => Some(14),
            OutputPacketType::ModeSLong => Some(21),
            OutputPacketType::DipSwitches => Some(1),
            OutputPacketType::Ping => Some(3),
            OutputPacketType::ReceiverId => Some(8),
            _ => None,
        }
    }

    fn is_known(&self) -> bool {
        match self {
            Self::Unknown(_) => false,
            _ => true,
        }
    }
}

impl PacketDecode for OutputPacketType {
    fn decode<B: Buf>(&self, buffer: &mut B) -> Option<OutputPacket> {
        match self {
            Self::ModeAc => {
                Some(OutputPacket::ModeAc {
                    timestamp: MlatTimestamp(buffer.get_bytes()),
                    signal_level: SignalLevel(buffer.get_u8()),
                    data: buffer.get_bytes(),
                })
            }
            Self::ModeSShort => {
                Some(OutputPacket::ModeSShort {
                    timestamp: MlatTimestamp(buffer.get_bytes()),
                    signal_level: SignalLevel(buffer.get_u8()),
                    data: buffer.get_bytes(),
                })
            }
            Self::ModeSLong => {
                Some(OutputPacket::ModeSLong {
                    timestamp: MlatTimestamp(buffer.get_bytes()),
                    signal_level: SignalLevel(buffer.get_u8()),
                    data: buffer.get_bytes(),
                })
            }
            Self::DipSwitches => Some(OutputPacket::DipSwitches(buffer.get_u8())),
            Self::Ping => {
                Some(OutputPacket::Ping {
                    data: buffer.get_bytes(),
                })
            }
            Self::ReceiverId => {
                Some(OutputPacket::ReceiverId {
                    receiver_id: Uuid::from_bytes(buffer.get_bytes()),
                })
            }
            Self::Unknown(byte) => {
                // todo: during development we want to know all packet types we receive.
                // normally this should return None
                todo!("beast: unknown packet type: 0x{byte:02x}");
            }
        }
    }
}

#[derive(Clone, Debug)]
pub enum OutputPacket {
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
    DipSwitches(u8),
    Ping {
        // todo: is this a timestamp?
        data: [u8; 3],
    },
    ReceiverId {
        receiver_id: Uuid,
    },
}

pin_project! {
    #[derive(Debug)]
    pub struct Reader<R> {
        #[pin]
        reader: R,
        receive_buffer: ReceiveBuffer,
        decoder: PacketDecoder,
    }
}

impl<R> Reader<R> {
    pub fn new(reader: R) -> Self {
        Self {
            reader,
            receive_buffer: Default::default(),
            decoder: Default::default(),
        }
    }
}

impl<R: AsyncRead> Stream for Reader<R> {
    type Item = Result<OutputPacket, Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        loop {
            let mut this = self.as_mut().project();

            if this.receive_buffer.has_data() {
                if let Some(packet) = this.decoder.decode_next(&mut this.receive_buffer)? {
                    return Poll::Ready(Some(Ok(packet)));
                }
            }
            else {
                // if there is no data in the receiver buffer, we need to receive some
                this.receive_buffer.reset();

                let mut read_buf = ReadBuf::new(&mut this.receive_buffer.buffer);
                match this.reader.poll_read(cx, &mut read_buf) {
                    Poll::Pending => return Poll::Pending,
                    Poll::Ready(Err(error)) => return Poll::Ready(Some(Err(error.into()))),
                    Poll::Ready(Ok(())) => {
                        let num_bytes_read = read_buf.filled().len();

                        // if no data was received, the underlying reader reached EOF
                        if num_bytes_read == 0 {
                            return Poll::Ready(None);
                        }

                        this.receive_buffer.write_pos = num_bytes_read;
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
    write_pos: usize,
}

impl Default for ReceiveBuffer {
    fn default() -> Self {
        Self {
            buffer: [0; RECEIVE_BUFFER_SIZE],
            read_pos: 0,
            write_pos: 0,
        }
    }
}

impl ReceiveBuffer {
    #[inline(always)]
    fn has_data(&self) -> bool {
        self.read_pos < self.write_pos
    }

    #[inline(always)]
    fn reset(&mut self) {
        self.read_pos = 0;
        self.write_pos = 0;
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
    packet_type: Option<OutputPacketType>,
    buffer: [u8; PACKET_BUFFER_SIZE],
    buffer_write_pos: usize,
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
    /// invocation of this method will resume decoding.
    fn decode_next(
        &mut self,
        receive_buffer: &mut ReceiveBuffer,
    ) -> Result<Option<OutputPacket>, Error> {
        while let Some(mut byte) = receive_buffer.next_byte() {
            if self.leading_escape_read {
                // we already read the packet escape

                if let Some(packet_type) = self.packet_type {
                    // we already read the packet type

                    if self.read_incomplete_escape {
                        // we read an escape before, but we don't know what follows yet.
                        // note: this whole block only handles the case that we read an escape at
                        // the end of the buffer earlier.
                        self.read_incomplete_escape = false;

                        if byte == ESCAPE {
                            // double escape
                            if packet_type.is_known() {
                                self.push_byte(ESCAPE);
                            }
                        }
                        else {
                            // the escape we read was the start of a new packet
                            self.leading_escape_read = true;

                            if let Some(packet) =
                                self.emit_packet(Some(OutputPacketType::from_byte(byte)))?
                            {
                                return Ok(Some(packet));
                            }
                        }
                    }
                    else {
                        if byte == ESCAPE {
                            if let Some(next_byte) = receive_buffer.next_byte() {
                                byte = next_byte;
                            }
                            else {
                                // we read an escape, but the buffer is drained, so we need to
                                // remember this
                                self.read_incomplete_escape = true;
                                break;
                            }
                        }

                        // payload byte
                        if packet_type.is_known() {
                            self.push_byte(byte);
                        }
                    }

                    if let Some(expected_length) = packet_type.expected_length() {
                        assert!(self.buffer_write_pos <= expected_length);

                        if self.buffer_write_pos == expected_length {
                            if let Some(packet) = self.emit_packet(None)? {
                                return Ok(Some(packet));
                            }
                        }
                    }
                }
                else {
                    // if we read an escape here, this is a double escape, meaning we're reading
                    // garbage
                    if byte == ESCAPE {
                        todo!("expected packet type, but read escape -> garbage");
                    }

                    // we didn't read the packet type yet, so this byte is it.
                    self.packet_type = Some(OutputPacketType::from_byte(byte));
                }
            }
            else if byte == ESCAPE {
                // we didn't receive a packet escape yet, but the current byte is one.
                self.leading_escape_read = true;
                self.packet_type = None;
                self.buffer_write_pos = 0;
            }
            else {
                // we didn't receive a packet escape yet, and the current byte isn't one.
                // this is a protocol error.
                todo!("garbage");
                // todo: we might want to return a specific (recoverable) error
            }
        }

        Ok(None)
    }

    #[inline(always)]
    fn push_byte(&mut self, byte: u8) {
        self.buffer[self.buffer_write_pos] = byte;
        self.buffer_write_pos += 1;
    }

    #[inline(always)]
    fn emit_packet(
        &mut self,
        next_packet_type: Option<OutputPacketType>,
    ) -> Result<Option<OutputPacket>, Error> {
        assert!(!self.read_incomplete_escape);
        assert!(self.leading_escape_read);

        if let Some(packet_type) = self.packet_type {
            if let Some(expected_length) = packet_type.expected_length() {
                if self.buffer_write_pos != expected_length {
                    todo!("error, invalid length");
                }
            }

            let mut buffer = &self.buffer[..self.buffer_write_pos];
            tracing::trace!(?buffer, len = buffer.len(), "decode packet");
            let packet = packet_type.decode(&mut buffer);

            // reset decoder state
            self.leading_escape_read = false;
            self.packet_type = next_packet_type;
            self.buffer_write_pos = 0;

            Ok(packet)
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
            packet_type: None,
            buffer: [0; PACKET_BUFFER_SIZE],
            buffer_write_pos: 0,
            read_incomplete_escape: false,
        }
    }
}

pin_project! {
    /// Keeps track of the receiver ID.
    ///
    /// This will consume any [`OutputPacket::ReceiverId`] packets from the
    /// underlying stream and yield all other packets together with the last
    /// received receiver ID.
    #[derive(Debug)]
    pub struct ReaderWithReceiverId<R> {
        #[pin]
        reader: Reader<R>,
        receiver_id: Option<Uuid>,
    }
}

impl<R> ReaderWithReceiverId<R> {
    pub fn new(reader: R) -> Self {
        Self {
            reader: Reader::new(reader),
            receiver_id: None,
        }
    }
}

impl<R: AsyncRead> Stream for ReaderWithReceiverId<R> {
    type Item = Result<(Option<Uuid>, OutputPacket), Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        loop {
            let this = self.as_mut().project();
            match this.reader.poll_next(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Some(Err(error))) => return Poll::Ready(Some(Err(error))),
                Poll::Ready(None) => return Poll::Ready(None),
                Poll::Ready(Some(Ok(OutputPacket::ReceiverId { receiver_id: id }))) => {
                    *this.receiver_id = Some(id);
                }
                Poll::Ready(Some(Ok(packet))) => {
                    return Poll::Ready(Some(Ok((self.receiver_id, packet))));
                }
            }
        }
    }
}
