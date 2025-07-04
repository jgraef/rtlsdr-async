pub mod history;
pub mod rtlsdr;
pub mod tar1090_db;

use chrono::Utc;
use futures_util::{
    Stream,
    StreamExt,
    pin_mut,
};
use serde::Deserialize;
use tokio::{
    io::BufReader,
    net::TcpStream,
};
use tokio_util::sync::CancellationToken;

use crate::{
    Error,
    tracker::Tracker,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SourceId(usize);

#[derive(Clone, Debug, Deserialize)]
pub enum SourceConfig {
    Beast { address: String },
    SbsMlat { address: String },
}

impl SourceConfig {
    pub async fn run(
        &self,
        source_id: SourceId,
        shutdown: CancellationToken,
        tracker: Tracker,
    ) -> Result<(), Error> {
        match self {
            SourceConfig::Beast { address } => {
                connect_client(
                    address,
                    shutdown,
                    adsbee_beast::output::ReaderWithReceiverId::new,
                    async |(receiver_id, packet)| {
                        // note: adsb.lol beast out sends synthetic no-forward timestamps, so the
                        // best we can do is to take the time when we receive the packet.
                        let time_received = Utc::now();

                        tracker
                            .push_beast(source_id, receiver_id, time_received, packet)
                            .await
                    },
                )
                .await?;
            }
            SourceConfig::SbsMlat { address } => {
                connect_client(
                    address,
                    shutdown,
                    adsbee_sbs::Reader::new,
                    async |message| tracker.push_sbs(source_id, true, message).await,
                )
                .await?;
            }
        }

        Ok(())
    }
}

async fn connect_client<T, F, R, E, P>(
    address: &str,
    shutdown: CancellationToken,
    create_reader: F,
    mut handle_message: P,
) -> Result<(), Error>
where
    F: FnOnce(BufReader<TcpStream>) -> R,
    R: Stream<Item = Result<T, E>>,
    Error: From<E>,
    P: AsyncFnMut(T),
{
    let stream = BufReader::new(TcpStream::connect(address).await?);
    let reader = create_reader(stream);
    pin_mut!(reader);

    loop {
        tokio::select! {
            _ = shutdown.cancelled() => {
                break;
            }
            message = reader.next() => {
                match message {
                    None => break,
                    Some(Err(error)) => return Err(error.into()),
                    Some(Ok(message)) => {
                        handle_message(message).await;
                    }
                }
            }
        }
    }

    Ok(())
}
