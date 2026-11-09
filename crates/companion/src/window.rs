use crate::native::Waker;
use nolae_core::{
    companion::AgentSnapshot,
    ipc::{self, ClientMessage, ServerMessage},
};
use std::{
    io::BufReader,
    process::{Child, Command, Stdio},
    sync::mpsc::SyncSender,
    time::{Duration, Instant},
};

pub enum WindowEvent {
    Command(u32, ClientMessage),
    Closed(u32),
    Menu(String),
    Show,
}

pub fn clean_old_versions() {
    let Ok(executable) = std::env::current_exe() else {
        return;
    };
    let Some(directory) = executable.parent() else {
        return;
    };
    let _ = std::fs::remove_file(directory.join("nolae-companion.exe.new"));
    let root = directory.join("ui");
    let Ok(entries) = std::fs::read_dir(&root) else {
        return;
    };
    let Ok(current) = env!("CARGO_PKG_VERSION").parse::<semver::Version>() else {
        return;
    };
    for entry in entries.flatten().take(1_000) {
        if !entry
            .file_type()
            .is_ok_and(|kind| kind.is_dir() && !kind.is_symlink())
        {
            continue;
        }
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let Ok(version) = name.parse::<semver::Version>() else {
            continue;
        };
        if version <= current {
            let _ = std::fs::remove_file(entry.path().join("nolae-companion.previous.exe"));
        }
        if version < current {
            let _ = std::fs::remove_file(entry.path().join("nolae-companion-ui.exe"));
            let _ = std::fs::remove_dir(entry.path());
        }
    }
}

pub struct WindowProcess {
    child: Child,
    outgoing: SyncSender<ServerMessage>,
}

impl WindowProcess {
    pub fn start(
        snapshot: AgentSnapshot,
        events: SyncSender<WindowEvent>,
        waker: Waker,
    ) -> Result<Self, String> {
        let core = std::env::current_exe().map_err(|_| "Could not locate Companion")?;
        let directory = core.parent().ok_or("Could not locate Companion")?;
        let path = directory
            .join("ui")
            .join(env!("CARGO_PKG_VERSION"))
            .join("nolae-companion-ui.exe");
        #[cfg(debug_assertions)]
        let path = if path.exists() {
            path
        } else {
            directory.join("nolae-companion-ui.exe")
        };
        let mut command = Command::new(path);
        command
            .arg("--core-ui")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        #[cfg(debug_assertions)]
        command.args(
            std::env::args()
                .skip(1)
                .filter(|argument| argument.starts_with("--preview")),
        );
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(windows::Win32::System::Threading::CREATE_NO_WINDOW.0);
        }
        let mut child = command
            .spawn()
            .map_err(|_| "Could not open Companion's window. Reinstall the latest version.")?;
        let id = child.id();
        let mut input = child.stdin.take().ok_or("Could not connect the window")?;
        let output = child.stdout.take().ok_or("Could not connect the window")?;
        let (outgoing, receiver) = std::sync::mpsc::sync_channel(8);
        outgoing
            .send(ServerMessage::Hello {
                protocol: ipc::VERSION,
                version: env!("CARGO_PKG_VERSION").into(),
                snapshot,
            })
            .map_err(|_| "Could not connect the window")?;
        let writer_events = events.clone();
        std::thread::spawn(move || {
            while let Ok(message) = receiver.recv() {
                if ipc::write_message(&mut input, &message).is_err() {
                    break;
                }
            }
            let _ = writer_events.try_send(WindowEvent::Closed(id));
            waker.wake();
        });
        std::thread::spawn(move || {
            let mut reader = BufReader::new(output);
            while let Ok(Some(message)) = ipc::read_message::<ClientMessage>(&mut reader) {
                if events.send(WindowEvent::Command(id, message)).is_err() {
                    break;
                }
                waker.wake();
            }
            let _ = events.try_send(WindowEvent::Closed(id));
            waker.wake();
        });
        Ok(Self { child, outgoing })
    }

    pub fn id(&self) -> u32 {
        self.child.id()
    }
    pub fn send(&self, message: ServerMessage) -> bool {
        self.outgoing.try_send(message).is_ok()
    }
    pub fn running(&mut self) -> bool {
        self.child.try_wait().is_ok_and(|status| status.is_none())
    }
}

impl Drop for WindowProcess {
    fn drop(&mut self) {
        let _ = self.outgoing.try_send(ServerMessage::Shutdown);
        let deadline = Instant::now() + Duration::from_secs(2);
        while self.running() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        if self.running() {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
    }
}
