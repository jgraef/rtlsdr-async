use tokio::sync::mpsc;

use crate::{
    Error,
    source::{
        beast,
        sbs,
    },
};

const COMMAND_QUEUE_SIZE: usize = 1024;

#[derive(Clone, Debug)]
pub struct Processor {
    command_sender: mpsc::Sender<Command>,
}

impl Processor {
    pub fn new() -> Self {
        let (command_sender, command_receiver) = mpsc::channel(COMMAND_QUEUE_SIZE);

        tokio::spawn(async move {
            let reactor = Reactor { command_receiver };
            reactor.run().await.expect("processor reactor died");
        });

        Self { command_sender }
    }

    async fn send_command(&self, command: Command) {
        self.command_sender
            .send(command)
            .await
            .expect("processor command channel closed");
    }
}

#[derive(Debug)]
struct Reactor {
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
            Command::PushBeast { source_id, packet } => todo!(),
            Command::PushSbs { source_id, message } => todo!(),
        }

        Ok(())
    }
}

#[derive(Debug)]
enum Command {
    PushBeast {
        source_id: usize,
        packet: beast::output::OutputPacket,
    },
    PushSbs {
        source_id: usize,
        message: sbs::Message,
    },
}
