use semver::Version;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Connection {
    Unpaired,
    Connecting,
    Connected,
    Offline,
    ReconnectRequired,
    UpdateRequired,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentSnapshot {
    pub connection: Connection,
    pub detail: String,
    pub track_title: Option<String>,
    pub artist: Option<String>,
    pub autostart: bool,
    pub notify_optional_updates: bool,
    pub update: UpdateState,
    pub busy: bool,
}
impl Default for AgentSnapshot {
    fn default() -> Self {
        Self {
            connection: Connection::Unpaired,
            detail: "Pair from your Nolae queue to get started.".into(),
            track_title: None,
            artist: None,
            autostart: false,
            notify_optional_updates: true,
            update: UpdateState::default(),
            busy: false,
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(
    tag = "type",
    content = "value",
    rename_all = "camelCase",
    deny_unknown_fields
)]
pub enum Command {
    Pair(String),
    Autostart(bool),
    NotifyOptionalUpdates(bool),
    Disconnect,
    CheckUpdates,
    InstallUpdate,
    Quit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum UpdateRequirement {
    Critical,
    Outdated,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateNotice {
    pub version: String,
    pub requirement: Option<UpdateRequirement>,
}

impl UpdateNotice {
    pub fn changelog_url(&self) -> String {
        format!(
            "https://github.com/nolaedotapp/companion/releases/tag/v{}",
            self.version
        )
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum UpdateActivity {
    #[default]
    Idle,
    Checking,
    Installing,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateState {
    pub available: Option<UpdateNotice>,
    pub installed_version: Option<String>,
    pub activity: UpdateActivity,
    pub error: Option<String>,
}

impl UpdateState {
    pub fn required(&self) -> bool {
        self.available
            .as_ref()
            .is_some_and(|update| update.requirement.is_some())
    }

    pub fn can_download(&self) -> bool {
        let installed = self
            .installed_version
            .as_deref()
            .unwrap_or(env!("CARGO_PKG_VERSION"));
        self.available.as_ref().is_some_and(|update| {
            Version::parse(&update.version)
                .ok()
                .zip(Version::parse(installed).ok())
                .is_some_and(|(available, installed)| available > installed)
        })
    }

    pub fn status(&self) -> String {
        match self.activity {
            UpdateActivity::Checking => "Checking for updates…".into(),
            UpdateActivity::Installing => "Downloading and verifying the update…".into(),
            UpdateActivity::Idle => {
                if let Some(error) = &self.error {
                    return error.clone();
                }
                if let Some(version) = &self.installed_version {
                    return format!("v{version} installed. Restart Companion to finish.");
                }
                if let Some(update) = &self.available {
                    return format!(
                        "v{} is available. Choose Update to download it.",
                        update.version
                    );
                }
                "Checks notify you. Downloads start only when you choose Update.".into()
            }
        }
    }
}

#[cfg(debug_assertions)]
impl AgentSnapshot {
    pub fn preview(mode: Option<&str>, connection: Option<&str>) -> Self {
        let mut snapshot = AgentSnapshot {
            connection: Connection::Connected,
            detail: "Preview · no account, credentials, or network connection.".into(),
            track_title: Some("A quiet moment".into()),
            artist: Some("Nolae preview".into()),
            ..AgentSnapshot::default()
        };
        if let Some(mode) = mode {
            let requirement = match mode {
                "critical" => Some(UpdateRequirement::Critical),
                "outdated" => Some(UpdateRequirement::Outdated),
                _ => None,
            };
            snapshot.update.available = Some(UpdateNotice {
                version: if mode == "newer" { "0.3.0" } else { "0.2.0" }.into(),
                requirement,
            });
            if matches!(mode, "installed" | "newer") {
                snapshot.update.installed_version = Some("0.2.0".into());
            }
            if mode == "error" {
                snapshot.update.error = Some("Update failed verification. Try again.".into());
            }
            if mode == "downloading" {
                snapshot.update.activity = UpdateActivity::Installing;
            }
            if snapshot.update.required() {
                snapshot.connection = Connection::UpdateRequired;
                snapshot.track_title = None;
                snapshot.artist = None;
                snapshot.detail = "Local detection is paused until you update and restart.".into();
            }
        }
        let detail = match connection {
            Some("unpaired") => Some((
                Connection::Unpaired,
                "Pair from your Nolae queue to get started.",
            )),
            Some("connecting") => Some((Connection::Connecting, "Connecting this PC…")),
            Some("waiting") => Some((
                Connection::Connected,
                "Paired. Waiting for Spotify desktop.",
            )),
            Some("offline") => Some((
                Connection::Offline,
                "Nolae is unreachable. Retrying automatically; hosted polling remains available.",
            )),
            Some("reconnect") => Some((
                Connection::ReconnectRequired,
                "This device was disconnected or replaced. Pair again from your Nolae queue.",
            )),
            _ => None,
        };
        if let Some((connection, detail)) = detail {
            snapshot.busy = connection == Connection::Connecting;
            snapshot.connection = connection;
            snapshot.detail = detail.into();
            snapshot.track_title = None;
            snapshot.artist = None;
        }
        snapshot
    }
}
