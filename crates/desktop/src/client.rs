use nolae_core::{
    companion::{AgentSnapshot, Command},
    ipc::{self, ClientMessage, ServerMessage},
};
use std::{
    io::{self, BufReader, Stdout},
    sync::{Arc, Mutex},
};

#[derive(Clone)]
pub struct Client {
    snapshot: Arc<Mutex<AgentSnapshot>>,
    output: Option<Arc<Mutex<Stdout>>>,
}

impl Client {
    pub fn connect() -> Result<(Self, async_channel::Receiver<ServerMessage>), String> {
        let mut input = BufReader::new(io::stdin());
        let hello = ipc::read_message(&mut input).map_err(|_| "Could not connect to Companion")?;
        let Some(ServerMessage::Hello {
            protocol,
            version,
            snapshot,
        }) = hello
        else {
            return Err("Start Nolae Companion from its shortcut.".into());
        };
        if protocol != ipc::VERSION || version != env!("CARGO_PKG_VERSION") {
            return Err("Restart Companion to finish updating its window.".into());
        }
        let (sender, receiver) = async_channel::bounded(8);
        std::thread::spawn(move || {
            while let Ok(Some(message)) = ipc::read_message(&mut input) {
                if sender.send_blocking(message).is_err() {
                    break;
                }
            }
        });
        Ok((
            Self {
                snapshot: Arc::new(Mutex::new(snapshot)),
                output: Some(Arc::new(Mutex::new(io::stdout()))),
            },
            receiver,
        ))
    }

    #[cfg(debug_assertions)]
    pub fn preview(update: Option<&str>, connection: Option<&str>) -> Self {
        Self {
            snapshot: Arc::new(Mutex::new(AgentSnapshot::preview(update, connection))),
            output: None,
        }
    }

    pub fn snapshot(&self) -> AgentSnapshot {
        self.snapshot
            .lock()
            .map(|snapshot| snapshot.clone())
            .unwrap_or_default()
    }
    pub fn set_snapshot(&self, snapshot: AgentSnapshot) {
        if let Ok(mut current) = self.snapshot.lock() {
            *current = snapshot;
        }
    }
    pub fn send(&self, command: Command) {
        let _ = self.write(ClientMessage::Command(command));
    }
    pub fn restart(&self) -> Result<(), String> {
        self.write(ClientMessage::Restart)
    }

    fn write(&self, message: ClientMessage) -> Result<(), String> {
        if let Some(output) = &self.output {
            let mut output = output.lock().map_err(|_| "Companion is unavailable")?;
            ipc::write_message(&mut *output, &message)
                .map_err(|_| "Companion is unavailable. Reopen it from the tray.")?;
        }
        Ok(())
    }
}
