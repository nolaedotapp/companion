use crate::native;
use nolae_core::companion::{AgentSnapshot, UpdateActivity, UpdateNotice, UpdateState};
use tray_icon::{TrayIcon, menu::MenuItem};

pub(crate) struct UpdateMenu {
    pub summary: MenuItem,
    pub check: MenuItem,
    pub changelog: MenuItem,
    pub restart: MenuItem,
    last_state: UpdateState,
    notified: Option<UpdateNotice>,
}

impl UpdateMenu {
    pub fn new() -> Self {
        Self {
            summary: MenuItem::with_id(
                "update",
                format!("Version {}", env!("CARGO_PKG_VERSION")),
                false,
                None,
            ),
            check: MenuItem::with_id("check", "Check for updates", true, None),
            changelog: MenuItem::with_id("changelog", "Changelog", true, None),
            restart: MenuItem::with_id("restart", "Restart Companion", false, None),
            last_state: UpdateState::default(),
            notified: None,
        }
    }

    pub fn refresh(&mut self, snapshot: &AgentSnapshot, tray: &TrayIcon, notifications: bool) {
        let state = &snapshot.update;
        if *state != self.last_state {
            self.check
                .set_enabled(state.activity == UpdateActivity::Idle);
            self.summary.set_enabled(state.available.is_some());
            self.restart.set_enabled(
                state.installed_version.is_some() && state.activity == UpdateActivity::Idle,
            );
            let label = if state.activity == UpdateActivity::Installing {
                "Downloading update...".into()
            } else if let Some(notice) = &state.available {
                if !state.can_download() && state.installed_version.is_some() {
                    format!("v{} installed - restart Companion", notice.version)
                } else if state.required() {
                    format!("v{} required - open Companion", notice.version)
                } else {
                    format!("v{} available - open Companion", notice.version)
                }
            } else {
                format!("Version {}", env!("CARGO_PKG_VERSION"))
            };
            self.summary.set_text(&label);
            let _ = tray.set_tooltip(Some(format!("Nolae Companion - {label}")));
            self.last_state = state.clone();
        }
        if notifications
            && let Some(notice) = &state.available
            && state.can_download()
            && (state.required() || snapshot.notify_optional_updates)
            && self.notified.as_ref() != Some(notice)
        {
            let title = if cfg!(debug_assertions)
                && std::env::args().any(|arg| arg == "--preview-notification")
            {
                "Preview: Companion update"
            } else if state.required() {
                "Companion update required"
            } else {
                "Companion update available"
            };
            let message = format!(
                "v{} is available. Open Companion from the tray for its changelog. Updates download only after you choose Update.",
                notice.version
            );
            let delivered = native::notify_update(tray, title, &message);
            #[cfg(debug_assertions)]
            if std::env::args().any(|arg| arg == "--preview-notification") {
                eprintln!("Preview notification delivery: {delivered}");
            }
            let _ = delivered;
            self.notified = Some(notice.clone());
        }
    }
}
