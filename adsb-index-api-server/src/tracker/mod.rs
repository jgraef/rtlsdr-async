pub mod state;
pub mod subscriptions;

use adsb_index_api_types::live::{
    ServerToClientMessage,
    SubscriptionFilter,
};
use chrono::{
    DateTime,
    Utc,
};
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::{
    api::live::ClientId,
    source::{
        SourceId,
        adsb_deku as adsb,
        beast::{
            self,
            MlatTimestamp,
        },
        sbs,
    },
    tracker::{
        state::{
            Position,
            PositionSource,
            State,
        },
        subscriptions::Subscriptions,
    },
};

const COMMAND_QUEUE_SIZE: usize = 32;

#[derive(Debug, thiserror::Error)]
#[error("broker error")]
pub enum Error {
    #[error("reactor dead")]
    ReactorDead,
    #[error("invalid subscription: {id}")]
    InvalidSubscriptionId { client_id: ClientId, id: Uuid },
}

/// Tracks aircrafts' state and notifies subscribers.
///
/// This struct is actually just a sender to a command channel, so it's cheap to
/// clone. When creating a [`Tracker`], a task is spawned that performs all the
/// work. This task can be controlled by sending commands, which is done by the
/// methods on this struct.
///
/// When the last [`Tracker`] is dropped the command channel is closed, which
/// signals the spawned task to terminate.
#[derive(Clone, Debug)]
pub struct Tracker {
    command_sender: mpsc::Sender<Command>,
}

impl Tracker {
    pub fn new() -> Self {
        let (command_sender, command_receiver) = mpsc::channel(COMMAND_QUEUE_SIZE);

        tokio::spawn({
            async move {
                let reactor = Reactor {
                    command_receiver,
                    subscriptions: Default::default(),
                    state: Default::default(),
                };
                reactor.run().await.expect("broker reactor error");
            }
        });

        Self { command_sender }
    }

    async fn send_command(&self, command: Command) {
        self.command_sender
            .send(command)
            .await
            .expect("broker command channel closed");
    }

    pub async fn subscribe(
        &self,
        client_id: ClientId,
        id: Uuid,
        filter: SubscriptionFilter,
        start_keyframe: bool,
        message_sender: mpsc::Sender<ServerToClientMessage>,
    ) {
        self.send_command(Command::Subscribe {
            client_id,
            id,
            filter,
            start_keyframe,
            message_sender,
        })
        .await;
    }

    pub async fn unsubscribe(
        &self,
        client_id: ClientId,
        id: Uuid,
        message_sender: mpsc::Sender<ServerToClientMessage>,
    ) {
        self.send_command(Command::Unsubscribe {
            client_id,
            id,
            message_sender,
        })
        .await;
    }

    pub async fn push_beast(
        &self,
        source_id: SourceId,
        receiver_id: Option<Uuid>,
        time_received: DateTime<Utc>,
        packet: beast::output::OutputPacket,
    ) {
        self.send_command(Command::PushBeast {
            source_id,
            receiver_id,
            time_received,
            packet,
        })
        .await;
    }

    pub async fn push_sbs(&self, source_id: SourceId, mlat: bool, message: sbs::Message) {
        self.send_command(Command::PushSbs {
            source_id,
            mlat,
            message,
        })
        .await;
    }
}

#[derive(Debug)]
struct Reactor {
    subscriptions: Subscriptions,
    state: State,
    command_receiver: mpsc::Receiver<Command>,
}

impl Reactor {
    async fn run(mut self) -> Result<(), Error> {
        while let Some(command) = self.command_receiver.recv().await {
            self.handle_command(command).await?;
        }

        Ok(())
    }

    async fn handle_command(&mut self, command: Command) -> Result<(), Error> {
        match command {
            Command::Subscribe {
                client_id,
                id,
                filter,
                start_keyframe,
                message_sender,
            } => {
                if start_keyframe {
                    todo!("starting keyframe");
                }

                if let Err(error) =
                    self.subscriptions
                        .subscribe(client_id, id, filter, message_sender.clone())
                {
                    let _ = message_sender.send(ServerToClientMessage::Error {
                        id: Some(id),
                        message: Some(error.to_string()),
                    });
                }
            }
            Command::Unsubscribe {
                client_id,
                id,
                message_sender,
            } => {
                if let Err(error) = self.subscriptions.unsubscribe(client_id, id) {
                    let _ = message_sender.send(ServerToClientMessage::Error {
                        id: Some(id),
                        message: Some(error.to_string()),
                    });
                }
            }
            Command::PushBeast {
                source_id,
                receiver_id,
                time_received,
                packet,
            } => {
                self.handle_beast_packet(source_id, receiver_id, time_received, packet)
                    .await?
            }
            Command::PushSbs {
                source_id,
                mlat,
                message,
            } => {
                if mlat {
                    self.handle_sbs_mlat_message(source_id, message).await?;
                }
                else {
                    //self.handle_sbs_message(source_id, message).await?;
                    todo!("handle non-mlat sbs message");
                }
            }
        }

        Ok(())
    }

    async fn handle_beast_packet(
        &mut self,
        source_id: SourceId,
        receiver_id: Option<Uuid>,
        time_received: DateTime<Utc>,
        packet: beast::output::OutputPacket,
    ) -> Result<(), Error> {
        match packet {
            beast::output::OutputPacket::ModeSShort {
                timestamp,
                signal_level,
                data,
            } => {
                self.handle_modes_packet(
                    source_id,
                    receiver_id,
                    timestamp,
                    time_received,
                    signal_level,
                    &data,
                )
                .await?;
            }
            beast::output::OutputPacket::ModeSLong {
                timestamp,
                signal_level,
                data,
            } => {
                self.handle_modes_packet(
                    source_id,
                    receiver_id,
                    timestamp,
                    time_received,
                    signal_level,
                    &data,
                )
                .await?;
            }
            _ => {}
        }

        Ok(())
    }

    async fn handle_modes_packet(
        &mut self,
        source_id: SourceId,
        receiver_id: Option<Uuid>,
        mlat_timestamp: MlatTimestamp,
        time_received: DateTime<Utc>,
        signal_level: beast::SignalLevel,
        data: &[u8],
    ) -> Result<(), Error> {
        let time = if mlat_timestamp.is_synthetic() {
            time_received
        }
        else {
            todo!("parse beast::MlatTimestamp");
        };

        match adsb::Frame::from_bytes(data) {
            Ok(frame) => {
                self.state.update_with_modes_frame(time, &frame);
            }
            Err(error) => {
                tracing::error!(?error);
            }
        }

        Ok(())
    }

    async fn handle_sbs_mlat_message(
        &mut self,
        _source_id: SourceId,
        message: sbs::Message,
    ) -> Result<(), Error> {
        match message {
            sbs::Message::Transmission {
                hex_ident,
                time_generated,
                transmission:
                    sbs::Transmission::EsAirbornePosition {
                        latitude,
                        longitude,
                        ..
                    },
                ..
            } => {
                self.state.update_mlat_position(
                    hex_ident,
                    time_generated,
                    Position {
                        latitude,
                        longitude,
                        source: PositionSource::Mlat,
                    },
                );
            }
            _ => {}
        }

        Ok(())
    }
}

#[derive(Debug)]
enum Command {
    Subscribe {
        client_id: ClientId,
        id: Uuid,
        filter: SubscriptionFilter,
        start_keyframe: bool,
        message_sender: mpsc::Sender<ServerToClientMessage>,
    },
    Unsubscribe {
        client_id: ClientId,
        id: Uuid,
        message_sender: mpsc::Sender<ServerToClientMessage>,
    },
    PushBeast {
        source_id: SourceId,
        receiver_id: Option<Uuid>,
        time_received: DateTime<Utc>,
        packet: beast::output::OutputPacket,
    },
    PushSbs {
        source_id: SourceId,
        mlat: bool,
        message: sbs::Message,
    },
}
