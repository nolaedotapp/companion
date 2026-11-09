use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub device_id: Option<Uuid>,
    #[serde(alias = "checkUpdates")]
    pub notify_optional_updates: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            device_id: None,
            notify_optional_updates: true,
        }
    }
}
pub(crate) fn directory() -> Result<PathBuf, String> {
    directories::ProjectDirs::from("app", "Nolae", "Nolae Companion")
        .map(|dirs| dirs.config_dir().to_path_buf())
        .ok_or("User settings directory is unavailable".into())
}
impl Settings {
    pub fn load() -> Result<Self, String> {
        let path = directory()?.join("settings.json");
        match std::fs::read(path) {
            Ok(bytes) => {
                serde_json::from_slice(&bytes).map_err(|_| "Settings could not be read".into())
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(_) => Err("Settings could not be read".into()),
        }
    }
    pub fn save(&self) -> Result<(), String> {
        let dir = directory()?;
        std::fs::create_dir_all(&dir).map_err(|_| "Could not create settings directory")?;
        let temporary = dir.join("settings.json.tmp");
        let bytes = serde_json::to_vec_pretty(self).map_err(|_| "Could not encode settings")?;
        std::fs::write(&temporary, bytes).map_err(|_| "Could not save settings")?;
        std::fs::rename(temporary, dir.join("settings.json"))
            .map_err(|_| "Could not save settings".into())
    }
}

#[cfg(windows)]
fn credential() -> Result<keyring::Entry, String> {
    keyring::Entry::new("app.nolae.companion", "local-playback-device")
        .map_err(|_| "Windows Credential Manager is unavailable".into())
}
#[cfg(windows)]
pub fn save_token(token: &str) -> Result<(), String> {
    credential()?
        .set_password(token)
        .map_err(|_| "Could not save the device credential".into())
}
#[cfg(windows)]
pub fn load_token() -> Result<Option<String>, String> {
    match credential()?.get_password() {
        Ok(value) => Ok(Some(value)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(_) => Err("Could not read the device credential".into()),
    }
}
#[cfg(windows)]
pub fn delete_token() -> Result<(), String> {
    match credential()?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(_) => Err("Could not remove the device credential".into()),
    }
}
#[cfg(not(windows))]
pub fn save_token(_: &str) -> Result<(), String> {
    Err("This release supports Windows only".into())
}
#[cfg(not(windows))]
pub fn load_token() -> Result<Option<String>, String> {
    Ok(None)
}
#[cfg(not(windows))]
pub fn delete_token() -> Result<(), String> {
    Ok(())
}

#[cfg(windows)]
pub fn autostart_enabled() -> bool {
    use winreg::{RegKey, enums::HKEY_CURRENT_USER};
    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Run")
        .and_then(|key| key.get_value::<String, _>("NolaeCompanion"))
        .is_ok()
}
#[cfg(windows)]
pub fn set_autostart(enabled: bool) -> Result<(), String> {
    use winreg::{RegKey, enums::HKEY_CURRENT_USER};
    let (key, _) = RegKey::predef(HKEY_CURRENT_USER)
        .create_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Run")
        .map_err(|_| "Could not change startup preference")?;
    if enabled {
        let exe = std::env::current_exe().map_err(|_| "Could not locate this app")?;
        key.set_value(
            "NolaeCompanion",
            &format!("\"{}\" --background", exe.display()),
        )
        .map_err(|_| "Could not enable startup".into())
    } else {
        match key.delete_value("NolaeCompanion") {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err("Could not disable startup".into()),
        }
    }
}
#[cfg(not(windows))]
pub fn autostart_enabled() -> bool {
    false
}
#[cfg(not(windows))]
pub fn set_autostart(_: bool) -> Result<(), String> {
    Err("Unsupported platform".into())
}
