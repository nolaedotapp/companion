mod api;
mod settings;
mod updates;
mod worker;

pub use nolae_core::companion::{AgentSnapshot, Command, Connection};
pub use updates::{UpdateActivity, UpdateNotice, UpdateRequirement, UpdateState};
pub use worker::Agent;

pub fn probe() -> Result<bool, String> {
    use nolae_core::playback::PlaybackProvider;
    let mut provider = nolae_windows_playback::WindowsPlayback::new()?;
    Ok(provider.snapshot()?.track.is_some())
}

pub fn uninstall() -> Result<(), String> {
    settings::set_autostart(false)?;
    if let Ok(Some(token)) = settings::load_token()
        && let Ok(api) = api::Api::new(api::PRODUCTION_SERVER)
    {
        let _ = api.disconnect(&token);
    }
    settings::delete_token()?;
    settings::Settings::default().save()
}
