use std::process::Stdio;

use tokio::{
    io::{
        AsyncBufReadExt,
        BufReader,
    },
    process::{
        ChildStdout,
        Command,
    },
};

use crate::RawFrame;

#[derive(Debug, thiserror::Error)]
#[error("rtl_adsb error")]
pub enum Error {
    Io(#[from] std::io::Error),
    InvalidLine(String),
}

/// Quick and dirty demodulator.
///
/// Spawns `rtl_adsb` and reads its output.
#[derive(Debug)]
pub struct RtlAdsbCommand {
    stdout: BufReader<ChildStdout>,
    buffer: String,
}

impl RtlAdsbCommand {
    pub async fn new() -> Result<Self, Error> {
        let mut process = Command::new("rtl_adsb")
            .arg("-S")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()?;
        let stdout = BufReader::new(process.stdout.take().expect("missing stdout"));
        Ok(Self {
            stdout,
            buffer: String::with_capacity(128),
        })
    }

    pub async fn next(&mut self) -> Result<Option<RawFrame>, Error> {
        loop {
            self.buffer.clear();
            if self.stdout.read_line(&mut self.buffer).await? == 0 {
                return Ok(None);
            }

            let line = self.buffer.trim();
            match line.len() {
                16 => {
                    let mut data = [0; 7];
                    hex::decode_to_slice(&self.buffer[1..15], &mut data)
                        .map_err(|_| Error::InvalidLine(line.to_owned()))?;
                    return Ok(Some(RawFrame::ModeSShort { data }));
                }
                30 => {
                    let mut data = [0; 14];
                    hex::decode_to_slice(&self.buffer[1..29], &mut data)
                        .map_err(|_| Error::InvalidLine(line.to_owned()))?;
                    return Ok(Some(RawFrame::ModeSLong { data }));
                }
                _ => {}
            }
        }
    }
}
