//!
//! - [readsb encoder][1]
//! - [Reference][2] ([archived][3])
//!
//! [1]: https://github.com/wiedehopf/readsb/blob/75decb53c0e66f4c12cf24127578a3fe7d919219/net_io.c#L3318
//! [2]: http://www.homepages.mcb.net/bones/SBS/Article/Barebones42_Socket_Data.htm
//! [3]: https://web.archive.org/web/20150107063617/http://www.homepages.mcb.net/bones/SBS/Article/Barebones42_Socket_Data.htm

use std::{
    pin::Pin,
    str::{
        FromStr,
        Utf8Error,
    },
    task::{
        Context,
        Poll,
    },
};

use adsb_index_api_types::{
    IcaoAddress,
    Squawk,
    SquawkFromStrError,
};
use chrono::{
    DateTime,
    NaiveDate,
    NaiveTime,
    Utc,
};
use futures_util::Stream;
use pin_project_lite::pin_project;
use tokio::io::{
    AsyncRead,
    ReadBuf,
};

const RECEIVE_BUFFER_SIZE: usize = 1024;

#[derive(Debug, thiserror::Error)]
#[error("sbs decode error")]
pub enum Error {
    Io(#[from] std::io::Error),
    MaxLineLengthExceeded,
    InvalidEncoding(#[from] Utf8Error),
    InvalidMessage(#[from] MessageFromStrError),
}

pin_project! {
    #[derive(Debug)]
    pub struct Reader<R> {
        #[pin]
        reader: R,
        receive_buffer: ReceiveBuffer,
    }
}

impl<R: AsyncRead> Reader<R> {
    pub fn new(reader: R) -> Self {
        Self {
            reader,
            receive_buffer: ReceiveBuffer::default(),
        }
    }
}

impl<R: AsyncRead> Stream for Reader<R> {
    type Item = Result<Message, Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        loop {
            let this = self.as_mut().project();

            if let Some(line) = this.receive_buffer.next_line() {
                // note: SBS seems to use `\r\n` for newlines, but we split lines at either. so
                // we accept `\r` or `\n`, and `\r\n` will produce an empty line, which we
                // ignore.
                //
                // note: readsb also sends empty lines as heartbeat messages
                // https://github.com/wiedehopf/readsb/blob/75decb53c0e66f4c12cf24127578a3fe7d919219/net_io.c#L110
                if !line.is_empty() {
                    match str::from_utf8(line) {
                        Ok(line) => {
                            match line.parse() {
                                Ok(message) => {
                                    return Poll::Ready(Some(Ok(message)));
                                }
                                Err(error) => return Poll::Ready(Some(Err(error.into()))),
                            }
                        }
                        Err(error) => {
                            return Poll::Ready(Some(Err(error.into())));
                        }
                    }
                }
            }
            else {
                this.receive_buffer.prepare_read();
                let mut read_buf =
                    ReadBuf::new(&mut this.receive_buffer.buffer[this.receive_buffer.write_pos..]);
                match this.reader.poll_read(cx, &mut read_buf) {
                    Poll::Pending => return Poll::Pending,
                    Poll::Ready(Err(error)) => return Poll::Ready(Some(Err(error.into()))),
                    Poll::Ready(Ok(())) => {
                        let num_bytes_read = read_buf.filled().len();
                        if num_bytes_read == 0 {
                            return Poll::Ready(None);
                        }

                        this.receive_buffer.write_pos += num_bytes_read;
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
    no_newline_until: usize,
}

impl ReceiveBuffer {
    fn has_data(&self) -> bool {
        self.no_newline_until < self.write_pos
    }

    fn scan_for_newline(&mut self) -> Option<usize> {
        println!(
            "read_pos={}, write_pos={}, no_newline_until={}",
            self.read_pos, self.write_pos, self.no_newline_until
        );

        if let Some(index) = self.buffer[self.no_newline_until..self.write_pos]
            .iter()
            .position(|byte| *byte == b'\r' || *byte == b'\n')
        {
            let index = index + self.no_newline_until;
            self.no_newline_until = index;
            Some(index)
        }
        else {
            self.no_newline_until = self.write_pos;
            None
        }
    }

    fn next_line(&mut self) -> Option<&[u8]> {
        if let Some(newline) = self.scan_for_newline() {
            let start = self.read_pos;
            self.read_pos = newline + 1;
            self.no_newline_until = self.read_pos;
            Some(&self.buffer[start..newline])
        }
        else {
            None
        }
    }

    fn prepare_read(&mut self) {
        if self.read_pos < self.write_pos && self.read_pos > 0 {
            // move data
            self.buffer.copy_within(self.read_pos..self.write_pos, 0);
            self.write_pos -= self.read_pos;
            self.no_newline_until -= self.read_pos;
            self.read_pos = 0;
        }
    }
}

impl Default for ReceiveBuffer {
    fn default() -> Self {
        Self {
            buffer: [0; RECEIVE_BUFFER_SIZE],
            read_pos: 0,
            write_pos: 0,
            no_newline_until: 0,
        }
    }
}

#[derive(Clone, Debug)]
pub enum Message {
    SelectionChange {
        session_id: u32,
        aircraft_id: u32,
        hex_ident: IcaoAddress,
        flight_id: u32,
        time_generated: DateTime<Utc>,
        time_logged: DateTime<Utc>,
        callsign: String,
    },
    NewId {
        session_id: u32,
        aircraft_id: u32,
        hex_ident: IcaoAddress,
        flight_id: u32,
        time_generated: DateTime<Utc>,
        time_logged: DateTime<Utc>,
        callsign: String,
    },
    NewAircraft {
        session_id: u32,
        aircraft_id: u32,
        hex_ident: IcaoAddress,
        flight_id: u32,
        time_generated: DateTime<Utc>,
        time_logged: DateTime<Utc>,
    },
    StatusChange {
        session_id: u32,
        aircraft_id: u32,
        hex_ident: IcaoAddress,
        flight_id: u32,
        time_generated: DateTime<Utc>,
        time_logged: DateTime<Utc>,
        status_change: StatusChange,
    },
    Click {
        session_id: u32,
        time_generated: DateTime<Utc>,
        time_logged: DateTime<Utc>,
    },
    Transmission {
        session_id: u32,
        aircraft_id: u32,
        hex_ident: IcaoAddress,
        flight_id: u32,
        time_generated: DateTime<Utc>,
        time_logged: DateTime<Utc>,
        transmission: Transmission,
    },
}

impl Message {
    pub fn session_id(&self) -> u32 {
        match self {
            Message::SelectionChange { session_id, .. } => *session_id,
            Message::NewId { session_id, .. } => *session_id,
            Message::NewAircraft { session_id, .. } => *session_id,
            Message::StatusChange { session_id, .. } => *session_id,
            Message::Click { session_id, .. } => *session_id,
            Message::Transmission { session_id, .. } => *session_id,
        }
    }

    pub fn time_generated(&self) -> DateTime<Utc> {
        match self {
            Message::SelectionChange { time_generated, .. } => *time_generated,
            Message::NewId { time_generated, .. } => *time_generated,
            Message::NewAircraft { time_generated, .. } => *time_generated,
            Message::StatusChange { time_generated, .. } => *time_generated,
            Message::Click { time_generated, .. } => *time_generated,
            Message::Transmission { time_generated, .. } => *time_generated,
        }
    }

    pub fn time_logged(&self) -> DateTime<Utc> {
        match self {
            Message::SelectionChange { time_logged, .. } => *time_logged,
            Message::NewId { time_logged, .. } => *time_logged,
            Message::NewAircraft { time_logged, .. } => *time_logged,
            Message::StatusChange { time_logged, .. } => *time_logged,
            Message::Click { time_logged, .. } => *time_logged,
            Message::Transmission { time_logged, .. } => *time_logged,
        }
    }
}

impl FromStr for Message {
    type Err = MessageFromStrError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut fields = s.split(',');

        // these fields are always present (might be empty though)
        let message_type = fields.next().ok_or(MessageFromStrError::Truncated)?;
        let transmission_type = fields.next().ok_or(MessageFromStrError::Truncated)?;
        let session_id = fields.next().ok_or(MessageFromStrError::Truncated)?;
        let aircraft_id = fields.next().ok_or(MessageFromStrError::Truncated)?;
        let hex_ident = fields.next().ok_or(MessageFromStrError::Truncated)?;
        let flight_id = fields.next().ok_or(MessageFromStrError::Truncated)?;
        let date_generated = fields.next().ok_or(MessageFromStrError::Truncated)?;
        let time_generated = fields.next().ok_or(MessageFromStrError::Truncated)?;
        let date_logged = fields.next().ok_or(MessageFromStrError::Truncated)?;
        let time_logged = fields.next().ok_or(MessageFromStrError::Truncated)?;

        // the following fields are always populated, so we parse them now.
        let session_id = session_id.parse::<u32>().map_err(|_| {
            MessageFromStrError::InvalidSessionId {
                value: session_id.to_owned(),
            }
        })?;

        const DATE_FORMAT: &'static str = "%Y/%m/%d";
        const TIME_FORMAT: &'static str = "%H:%M:%S%.3f";

        let time_generated = NaiveDate::parse_from_str(date_generated, DATE_FORMAT)
            .map_err(|_e| {
                MessageFromStrError::InvalidDateMessageGenerated {
                    value: date_generated.to_owned(),
                }
            })?
            .and_time(
                NaiveTime::parse_from_str(time_generated, TIME_FORMAT).map_err(|_e| {
                    MessageFromStrError::InvalidTimeMessageGenerated {
                        value: time_generated.to_owned(),
                    }
                })?,
            )
            .and_utc();

        let time_logged = NaiveDate::parse_from_str(date_logged, DATE_FORMAT)
            .map_err(|_e| {
                MessageFromStrError::InvalidDateMessageLogged {
                    value: date_logged.to_owned(),
                }
            })?
            .and_time(
                NaiveTime::parse_from_str(time_logged, TIME_FORMAT).map_err(|_e| {
                    MessageFromStrError::InvalidTimeMessageLogged {
                        value: time_logged.to_owned(),
                    }
                })?,
            )
            .and_utc();

        // these values are not always available, so we provide parsers for them
        let aircraft_id = || {
            aircraft_id.parse::<u32>().map_err(|_| {
                MessageFromStrError::InvalidAircraftId {
                    value: aircraft_id.to_owned(),
                }
            })
        };
        let hex_ident = || {
            hex_ident
                .parse::<IcaoAddress>()
                .map_err(|error| MessageFromStrError::InvalidHexIdent { value: error.input })
        };
        let flight_id = || {
            flight_id.parse::<u32>().map_err(|_| {
                MessageFromStrError::InvalidFlightId {
                    value: flight_id.to_owned(),
                }
            })
        };

        let message = match message_type {
            "SEL" => {
                let callsign =
                    parse_callsign(fields.next().ok_or(MessageFromStrError::Truncated)?)?;
                Self::SelectionChange {
                    session_id,
                    aircraft_id: aircraft_id()?,
                    hex_ident: hex_ident()?,
                    flight_id: flight_id()?,
                    time_generated,
                    time_logged,
                    callsign,
                }
            }
            "ID" => {
                let callsign =
                    parse_callsign(fields.next().ok_or(MessageFromStrError::Truncated)?)?;
                Self::NewId {
                    session_id,
                    aircraft_id: aircraft_id()?,
                    hex_ident: hex_ident()?,
                    flight_id: flight_id()?,
                    time_generated,
                    time_logged,
                    callsign,
                }
            }
            "AIR" => {
                Self::NewAircraft {
                    session_id,
                    aircraft_id: aircraft_id()?,
                    hex_ident: hex_ident()?,
                    flight_id: flight_id()?,
                    time_generated,
                    time_logged,
                }
            }
            "STA" => {
                let status_change = fields
                    .next()
                    .ok_or(MessageFromStrError::Truncated)?
                    .parse()?;
                Self::StatusChange {
                    session_id,
                    aircraft_id: aircraft_id()?,
                    hex_ident: hex_ident()?,
                    flight_id: flight_id()?,
                    time_generated,
                    time_logged,
                    status_change,
                }
            }
            "CLK" => {
                Self::Click {
                    session_id,
                    time_generated,
                    time_logged,
                }
            }
            "MSG" => {
                let callsign =
                    parse_callsign(fields.next().ok_or(MessageFromStrError::Truncated)?)?;
                let altitude = fields.next().ok_or(MessageFromStrError::Truncated)?;
                let ground_speed = fields.next().ok_or(MessageFromStrError::Truncated)?;
                let track = fields.next().ok_or(MessageFromStrError::Truncated)?;
                let latitude = fields.next().ok_or(MessageFromStrError::Truncated)?;
                let longitude = fields.next().ok_or(MessageFromStrError::Truncated)?;
                let vertical_rate = fields.next().ok_or(MessageFromStrError::Truncated)?;
                let squawk = fields.next().ok_or(MessageFromStrError::Truncated)?;
                let alert = fields.next().ok_or(MessageFromStrError::Truncated)?;
                let emergency = fields.next().ok_or(MessageFromStrError::Truncated)?;
                let spi = fields.next().ok_or(MessageFromStrError::Truncated)?;
                let is_on_ground = fields.next().ok_or(MessageFromStrError::Truncated)?;

                fn parse_bool(s: &str) -> Option<bool> {
                    match s {
                        "0" => Some(false),
                        "-1" => Some(true),
                        _ => None,
                    }
                }

                let altitude = || {
                    if altitude.is_empty() {
                        Ok(None)
                    }
                    else {
                        altitude
                            .parse()
                            .map_err(|_| {
                                MessageFromStrError::InvalidAltitude {
                                    value: altitude.to_owned(),
                                }
                            })
                            .map(Some)
                    }
                };
                let ground_speed = || {
                    ground_speed.parse().map_err(|_| {
                        MessageFromStrError::InvalidGroundSpeed {
                            value: ground_speed.to_owned(),
                        }
                    })
                };
                let track = || {
                    track.parse().map_err(|_| {
                        MessageFromStrError::InvalidTrack {
                            value: track.to_owned(),
                        }
                    })
                };
                let latitude = || {
                    latitude.parse().map_err(|_| {
                        MessageFromStrError::InvalidLatitude {
                            value: latitude.to_owned(),
                        }
                    })
                };
                let longitude = || {
                    longitude.parse().map_err(|_| {
                        MessageFromStrError::InvalidLongitude {
                            value: longitude.to_owned(),
                        }
                    })
                };
                let vertical_rate = || {
                    vertical_rate.parse().map_err(|_| {
                        MessageFromStrError::InvalidVerticalRate {
                            value: vertical_rate.to_owned(),
                        }
                    })
                };
                let squawk = || squawk.parse();
                let alert = || {
                    parse_bool(alert).ok_or_else(|| {
                        MessageFromStrError::InvalidAlert {
                            value: alert.to_owned(),
                        }
                    })
                };
                let emergency = || {
                    parse_bool(emergency).ok_or_else(|| {
                        MessageFromStrError::InvalidEmergency {
                            value: emergency.to_owned(),
                        }
                    })
                };
                let spi = || {
                    parse_bool(spi).ok_or_else(|| {
                        MessageFromStrError::InvalidSpi {
                            value: spi.to_owned(),
                        }
                    })
                };
                let is_on_ground = || {
                    if is_on_ground.is_empty() {
                        Ok(None)
                    }
                    else {
                        parse_bool(is_on_ground)
                            .ok_or_else(|| {
                                MessageFromStrError::InvalidIsOnGround {
                                    value: is_on_ground.to_owned(),
                                }
                            })
                            .map(Some)
                    }
                };

                let transmission = match transmission_type {
                    "1" => {
                        Transmission::EsIdentificationAndCategory {
                            callsign: callsign.to_owned(),
                        }
                    }
                    "2" => {
                        Transmission::EsSurfacePosition {
                            altitude: altitude()?,
                            ground_speed: ground_speed()?,
                            track: track()?,
                            latitude: latitude()?,
                            longitude: longitude()?,
                            is_on_ground: is_on_ground()?,
                        }
                    }
                    "3" => {
                        Transmission::EsAirbornePosition {
                            altitude: altitude()?,
                            latitude: latitude()?,
                            longitude: longitude()?,
                            alert: alert()?,
                            emergency: emergency()?,
                            spi: spi()?,
                            is_on_ground: is_on_ground()?,
                        }
                    }
                    "4" => {
                        Transmission::EsAirborneVelocity {
                            ground_speed: ground_speed()?,
                            track: track()?,
                            vertical_rate: vertical_rate()?,
                        }
                    }
                    "5" => {
                        Transmission::SurveillanceAltMessage {
                            altitude: altitude()?,
                            alert: alert()?,
                            spi: spi()?,
                            is_on_ground: is_on_ground()?,
                        }
                    }
                    "6" => {
                        Transmission::SurveillanceIdMessage {
                            altitude: altitude()?,
                            squawk: squawk()?,
                            alert: alert()?,
                            emergency: emergency()?,
                            spi: spi()?,
                            is_on_ground: is_on_ground()?,
                        }
                    }
                    "7" => {
                        Transmission::AirToAirMessage {
                            altitude: altitude()?,
                            is_on_ground: is_on_ground()?,
                        }
                    }
                    "8" => {
                        Transmission::AllCallReply {
                            is_on_ground: is_on_ground()?,
                        }
                    }
                    _ => {
                        return Err(MessageFromStrError::InvalidTransmissionType {
                            value: message_type.to_owned(),
                        });
                    }
                };

                Self::Transmission {
                    session_id,
                    aircraft_id: aircraft_id()?,
                    hex_ident: hex_ident()?,
                    flight_id: flight_id()?,
                    time_generated,
                    time_logged,
                    transmission,
                }
            }
            _ => {
                return Err(MessageFromStrError::InvalidMessageType {
                    value: message_type.to_owned(),
                });
            }
        };

        Ok(message)
    }
}

#[derive(Debug, thiserror::Error)]
#[error("sbs message parse error")]
pub enum MessageFromStrError {
    #[error("truncated message")]
    Truncated,
    #[error("invalid session id: {value}")]
    InvalidSessionId {
        value: String,
    },
    #[error("invalid aircraft id: {value}")]
    InvalidAircraftId {
        value: String,
    },
    #[error("invalid hex ident: {value}")]
    InvalidHexIdent {
        value: String,
    },
    #[error("invalid flight id: {value}")]
    InvalidFlightId {
        value: String,
    },
    #[error("invalid date message generated: {value}")]
    InvalidDateMessageGenerated {
        value: String,
    },
    #[error("invalid time message generated: {value}")]
    InvalidTimeMessageGenerated {
        value: String,
    },
    #[error("invalid date message logged: {value}")]
    InvalidDateMessageLogged {
        value: String,
    },
    #[error("invalid time message logged: {value}")]
    InvalidTimeMessageLogged {
        value: String,
    },
    #[error("invalid message type: {value}")]
    InvalidMessageType {
        value: String,
    },
    #[error("invalid message type: {value}")]
    InvalidTransmissionType {
        value: String,
    },
    InvalidStatusChange(#[from] StatusChangeFromStrError),
    #[error("invalid altitude: {value}")]
    InvalidAltitude {
        value: String,
    },

    #[error("invalid ground speed: {value}")]
    InvalidGroundSpeed {
        value: String,
    },
    #[error("invalid track: {value}")]
    InvalidTrack {
        value: String,
    },
    #[error("invalid latitude: {value}")]
    InvalidLatitude {
        value: String,
    },
    #[error("invalid longitude: {value}")]
    InvalidLongitude {
        value: String,
    },
    #[error("invalid vertical rate: {value}")]
    InvalidVerticalRate {
        value: String,
    },
    InvalidSquawk(#[from] SquawkFromStrError),
    #[error("invalid alert: {value}")]
    InvalidAlert {
        value: String,
    },
    #[error("invalid emergency: {value}")]
    InvalidEmergency {
        value: String,
    },
    #[error("invalid spi: {value}")]
    InvalidSpi {
        value: String,
    },
    #[error("invalid is_on_ground: {value}")]
    InvalidIsOnGround {
        value: String,
    },
}

#[derive(Clone, Debug)]
pub enum MessageType {}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StatusChange {
    PositionLost,
    SignalLost,
    Remove,
    Delete,
    Ok,
}

impl FromStr for StatusChange {
    type Err = StatusChangeFromStrError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "PL" => Ok(Self::PositionLost),
            "SL" => Ok(Self::SignalLost),
            "RM" => Ok(Self::Remove),
            "AD" => Ok(Self::Delete),
            "OK" => Ok(Self::Ok),
            _ => {
                Err(StatusChangeFromStrError {
                    input: s.to_owned(),
                })
            }
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("invalid status change")]
pub struct StatusChangeFromStrError {
    pub input: String,
}

fn parse_callsign(s: &str) -> Result<String, MessageFromStrError> {
    // todo: check the callsign
    Ok(s.replace('@', " "))
}

#[derive(Clone, Debug)]
pub enum Transmission {
    EsIdentificationAndCategory {
        callsign: String,
    },
    EsSurfacePosition {
        altitude: Option<u32>,
        ground_speed: f32,
        track: f32,
        latitude: f32,
        longitude: f32,
        is_on_ground: Option<bool>,
    },
    EsAirbornePosition {
        altitude: Option<u32>,
        latitude: f32,
        longitude: f32,
        alert: bool,
        emergency: bool,
        spi: bool,
        is_on_ground: Option<bool>,
    },
    EsAirborneVelocity {
        ground_speed: f32,
        track: f32,
        vertical_rate: i32,
    },
    SurveillanceAltMessage {
        altitude: Option<u32>,
        alert: bool,
        spi: bool,
        is_on_ground: Option<bool>,
    },
    SurveillanceIdMessage {
        altitude: Option<u32>,
        squawk: Squawk,
        alert: bool,
        emergency: bool,
        spi: bool,
        is_on_ground: Option<bool>,
    },
    AirToAirMessage {
        altitude: Option<u32>,
        is_on_ground: Option<bool>,
    },
    AllCallReply {
        is_on_ground: Option<bool>,
    },
}

#[cfg(test)]
mod tests {
    use futures_util::TryStreamExt;

    use crate::source::sbs::{
        Message,
        Reader,
    };

    const EXAMPLE: &'static str = r#"SEL,,496,2286,4CA4E5,27215,2010/02/19,18:06:07.710,2010/02/19,18:06:07.710,RYR1427
ID,,496,7162,405637,27928,2010/02/19,18:06:07.115,2010/02/19,18:06:07.115,EZY691A
AIR,,496,5906,400F01,27931,2010/02/19,18:06:07.128,2010/02/19,18:06:07.128
STA,,5,179,400AE7,10103,2008/11/28,14:58:51.153,2008/11/28,14:58:51.153,RM
CLK,,496,-1,,-1,2010/02/19,18:18:19.036,2010/02/19,18:18:19.036
MSG,1,145,256,7404F2,11267,2008/11/28,23:48:18.611,2008/11/28,23:53:19.161,RJA1118,,,,,,,,,,,
MSG,2,496,603,400CB6,13168,2008/10/13,12:24:32.414,2008/10/13,12:28:52.074,,,0,76.4,258.3,54.05735,-4.38826,,,,,,0
MSG,3,496,211,4CA2D6,10057,2008/11/28,14:53:50.594,2008/11/28,14:58:51.153,,37000,,,51.45735,-1.02826,,,0,0,0,0
MSG,4,496,469,4CA767,27854,2010/02/19,17:58:13.039,2010/02/19,17:58:13.368,,,288.6,103.2,,,-832,,,,,
MSG,5,496,329,394A65,27868,2010/02/19,17:58:12.644,2010/02/19,17:58:13.368,,10000,,,,,,,0,,0,0
MSG,6,496,237,4CA215,27864,2010/02/19,17:58:12.846,2010/02/19,17:58:13.368,,33325,,,,,,0271,0,0,0,0
MSG,7,496,742,51106E,27929,2011/03/06,07:57:36.523,2011/03/06,07:57:37.054,,3775,,,,,,,,,,0
MSG,8,496,194,405F4E,27884,2010/02/19,17:58:13.244,2010/02/19,17:58:13.368,,,,,,,,,,,,0
"#;

    #[test]
    fn it_parses_the_example() {
        for line in EXAMPLE.lines() {
            if !line.is_empty() {
                line.parse::<Message>().unwrap();
            }
        }
    }

    #[tokio::test]
    async fn it_decodes_a_stream() {
        let mut reader = Reader::new(EXAMPLE.as_bytes());

        while let Some(message) = reader.try_next().await.unwrap() {
            println!("{message:?}");
        }
    }
}
