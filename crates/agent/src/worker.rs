use crate::{
    api::{Api, ApiError, PRODUCTION_SERVER},
    settings::{self, Settings},
    updates,
};
use nolae_core::{
    playback::{PlaybackProvider, PublishGate},
    protocol::{Observation, PROTOCOL_VERSION, PlaybackFrame},
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Sender},
};
use std::time::{Duration, Instant};
use uuid::Uuid;

use nolae_core::companion::{AgentSnapshot, Command, Connection};

#[derive(Clone)]
pub struct Agent {
    commands: Sender<Command>,
    state: Arc<SharedState>,
    running: Arc<AtomicBool>,
    capture_disabled: Arc<AtomicBool>,
}
struct Sample {
    observation: Observation,
    captured_at: Instant,
}

type WakeHandler = std::sync::Arc<dyn Fn() + Send + Sync>;

struct SharedState {
    value: Mutex<AgentSnapshot>,
    wake: Mutex<Option<WakeHandler>>,
}

impl SharedState {
    fn new(snapshot: AgentSnapshot) -> Self {
        Self {
            value: Mutex::new(snapshot),
            wake: Mutex::new(None),
        }
    }

    fn lock(&self) -> std::sync::LockResult<std::sync::MutexGuard<'_, AgentSnapshot>> {
        self.value.lock()
    }
}

fn change(state: &SharedState, action: impl FnOnce(&mut AgentSnapshot)) {
    let changed = if let Ok(mut current) = state.lock() {
        let previous = current.clone();
        action(&mut current);
        *current != previous
    } else {
        false
    };
    if changed {
        let wake = state.wake.lock().ok().and_then(|handler| handler.clone());
        if let Some(wake) = wake {
            wake();
        }
    }
}

impl Agent {
    #[cfg(debug_assertions)]
    pub fn preview(update: Option<&str>, connection: Option<&str>) -> Self {
        let (commands, _receiver) = mpsc::channel();
        let snapshot = AgentSnapshot::preview(update, connection);
        Self {
            commands,
            state: Arc::new(SharedState::new(snapshot)),
            running: Arc::new(AtomicBool::new(false)),
            capture_disabled: Arc::new(AtomicBool::new(true)),
        }
    }
    pub fn start() -> Self {
        let state = Arc::new(SharedState::new(AgentSnapshot {
            autostart: settings::autostart_enabled(),
            ..AgentSnapshot::default()
        }));
        let running = Arc::new(AtomicBool::new(true));
        let capture_disabled = Arc::new(AtomicBool::new(true));
        let (commands, receiver) = mpsc::channel();
        let sample = Arc::new(Mutex::new(Sample {
            observation: Observation::unavailable(),
            captured_at: Instant::now(),
        }));
        let capture_state = state.clone();
        let source_state = sample.clone();
        let source_running = running.clone();
        let source_capture_disabled = capture_disabled.clone();
        std::thread::spawn(move || {
            sample_loop(
                source_state,
                source_running,
                source_capture_disabled,
                capture_state,
            )
        });
        let worker_state = state.clone();
        let worker_running = running.clone();
        let worker_capture_disabled = capture_disabled.clone();
        std::thread::spawn(move || {
            network_loop(
                receiver,
                sample,
                worker_state,
                worker_running,
                worker_capture_disabled,
            )
        });
        Self {
            commands,
            state,
            running,
            capture_disabled,
        }
    }
    pub fn send(&self, command: Command) {
        if matches!(command, Command::Disconnect | Command::Quit) {
            self.capture_disabled.store(true, Ordering::Relaxed);
        }
        if matches!(command, Command::Quit) {
            self.running.store(false, Ordering::Relaxed);
        }
        let _ = self.commands.send(command);
    }
    pub fn set_wake_handler(&self, handler: impl Fn() + Send + Sync + 'static) {
        if let Ok(mut wake) = self.state.wake.lock() {
            *wake = Some(Arc::new(handler));
        }
    }

    pub fn snapshot(&self) -> AgentSnapshot {
        self.state.lock().map(|s| s.clone()).unwrap_or_default()
    }
}

fn sample_loop(
    sample: Arc<Mutex<Sample>>,
    running: Arc<AtomicBool>,
    capture_disabled: Arc<AtomicBool>,
    state: Arc<SharedState>,
) {
    #[cfg(windows)]
    {
        let mut provider = None;
        while running.load(Ordering::Relaxed) {
            let observation = if capture_disabled.load(Ordering::Relaxed)
                || state.lock().map(|s| s.update.required()).unwrap_or(true)
            {
                Observation::unavailable()
            } else {
                if provider.is_none() {
                    provider = nolae_windows_playback::WindowsPlayback::new().ok();
                }
                match provider.as_mut().map(PlaybackProvider::snapshot) {
                    Some(Ok(observation)) => observation,
                    _ => {
                        provider = None;
                        Observation::unavailable()
                    }
                }
            };
            if let Ok(mut current) = sample.lock() {
                *current = Sample {
                    observation,
                    captured_at: Instant::now(),
                };
            }
            std::thread::sleep(Duration::from_millis(500));
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (sample, running, capture_disabled, state);
    }
}

fn perform_update(
    state: &SharedState,
    install_requested: bool,
    check: impl FnOnce() -> Result<updates::CheckResult, String>,
    install: impl FnOnce(&str, &str) -> Result<bool, String>,
) {
    let result = check().and_then(|checked| {
        let previous = state
            .lock()
            .map_err(|_| "Update state is unavailable")?
            .update
            .available
            .clone();
        if let Some(previous) = previous.as_ref() {
            let next = checked
                .available
                .as_ref()
                .ok_or("Update history moved backwards. Try again later.")?;
            if semver::Version::parse(&next.version).ok()
                < semver::Version::parse(&previous.version).ok()
                || (previous.requirement.is_some() && next.requirement.is_none())
                || (previous.requirement == Some(updates::UpdateRequirement::Critical)
                    && next.requirement != previous.requirement)
            {
                return Err("Update history moved backwards. Try again later.".into());
            }
        }
        let cache_error = checked.cache().err();
        change(state, |s| {
            s.update.available = checked.available;
            s.update.error = cache_error;
        });
        let target = state.lock().ok().and_then(|s| {
            (install_requested && s.update.can_download()).then(|| {
                (
                    s.update.available.as_ref().unwrap().version.clone(),
                    s.update
                        .installed_version
                        .clone()
                        .unwrap_or(env!("CARGO_PKG_VERSION").into()),
                )
            })
        });
        if let Some((version, installed)) = target {
            change(state, |s| {
                s.update.activity = updates::UpdateActivity::Installing
            });
            if install(&version, &installed)? {
                change(state, |s| s.update.installed_version = Some(version));
            }
        }
        Ok(())
    });
    change(state, |s| {
        s.update.activity = updates::UpdateActivity::Idle;
        if let Err(error) = result {
            s.update.error = Some(error);
        }
    });
}

fn launch_update(state: Arc<SharedState>, updating: Arc<AtomicBool>, install_requested: bool) {
    if updating.swap(true, Ordering::AcqRel) {
        return;
    }
    change(&state, |s| {
        s.update.activity = updates::UpdateActivity::Checking;
        s.update.error = None;
    });
    std::thread::spawn(move || {
        perform_update(&state, install_requested, updates::check, updates::install);
        updating.store(false, Ordering::Release);
    });
}

fn network_loop(
    receiver: mpsc::Receiver<Command>,
    sample: Arc<Mutex<Sample>>,
    state: Arc<SharedState>,
    running: Arc<AtomicBool>,
    capture_disabled: Arc<AtomicBool>,
) {
    let mut preferences = match Settings::load() {
        Ok(settings) => settings,
        Err(message) => {
            change(&state, |s| s.detail = message);
            Settings::default()
        }
    };
    let updating = Arc::new(AtomicBool::new(false));
    change(&state, |s| {
        s.notify_optional_updates = preferences.notify_optional_updates;
    });
    match updates::cached() {
        Ok(available) => change(&state, |s| s.update.available = available),
        Err(error) => change(&state, |s| s.update.error = Some(error)),
    }
    let server = if cfg!(debug_assertions) {
        std::env::var("NOLAE_DEV_SERVER").unwrap_or(PRODUCTION_SERVER.into())
    } else {
        PRODUCTION_SERVER.into()
    };
    let Ok(api) = Api::new(&server) else {
        change(&state, |s| {
            s.detail = "Invalid server configuration.".into()
        });
        return;
    };
    let mut token = match settings::load_token() {
        Ok(token) => token,
        Err(message) => {
            change(&state, |s| s.detail = message);
            None
        }
    };
    capture_disabled.store(
        token.is_none() || preferences.device_id.is_none(),
        Ordering::Relaxed,
    );
    let mut session: Option<Uuid> = None;
    let mut sequence = 0u64;
    let started = Instant::now();
    let mut gate = PublishGate::default();
    let mut force_artwork = false;
    let mut next_attempt = Instant::now();
    let mut failures = 0u32;
    let mut next_update = Instant::now() + Duration::from_secs(60);
    while running.load(Ordering::Relaxed) {
        if let Ok(command) = receiver.recv_timeout(Duration::from_millis(250)) {
            match command {
                Command::Quit => break,
                Command::Pair(code) => {
                    if state.lock().map(|s| s.update.required()).unwrap_or(true) {
                        continue;
                    }
                    change(&state, |s| {
                        s.busy = true;
                        s.connection = Connection::Connecting;
                        s.detail = "Connecting this PC…".into();
                    });
                    match api.pair(code.trim()).and_then(|pair| {
                        settings::save_token(&pair.device_token)
                            .map_err(|_| ApiError::Unavailable)?;
                        preferences.device_id = Some(pair.device_id);
                        preferences.save().map_err(|_| ApiError::Unavailable)?;
                        Ok(pair.device_token)
                    }) {
                        Ok(value) => {
                            token = Some(value);
                            session = None;
                            sequence = 0;
                            gate = PublishGate::default();
                            next_attempt = Instant::now();
                            capture_disabled.store(false, Ordering::Relaxed);
                            change(&state, |s| {
                                s.detail = "Paired. Waiting for Spotify desktop.".into();
                            });
                        }
                        Err(_) => change(&state, |s| {
                            s.connection = Connection::Unpaired;
                            s.detail = "Could not pair. Create a fresh code in your Nolae queue and try again.".into();
                        }),
                    }
                    change(&state, |s| s.busy = false);
                }
                Command::Autostart(enabled) => match settings::set_autostart(enabled) {
                    Ok(()) => change(&state, |s| s.autostart = enabled),
                    Err(message) => change(&state, |s| s.detail = message),
                },
                Command::NotifyOptionalUpdates(enabled) => {
                    let previous = preferences.notify_optional_updates;
                    preferences.notify_optional_updates = enabled;
                    match preferences.save() {
                        Ok(()) => change(&state, |s| s.notify_optional_updates = enabled),
                        Err(error) => {
                            preferences.notify_optional_updates = previous;
                            change(&state, |s| s.update.error = Some(error));
                        }
                    }
                }
                Command::Disconnect => {
                    if let Some(value) = token.as_ref() {
                        let _ = api.disconnect(value);
                    }
                    token = None;
                    session = None;
                    let _ = settings::delete_token();
                    capture_disabled.store(true, Ordering::Relaxed);
                    preferences.device_id = None;
                    let _ = preferences.save();
                    change(&state, |s| {
                        s.connection = Connection::Unpaired;
                        s.track_title = None;
                        s.artist = None;
                        s.detail = "Disconnected. You can also revoke this PC from Nolae.".into();
                    });
                }
                Command::CheckUpdates => {
                    launch_update(state.clone(), updating.clone(), false);
                    next_update = Instant::now() + Duration::from_secs(3600);
                }
                Command::InstallUpdate => launch_update(state.clone(), updating.clone(), true),
            }
        }
        if Instant::now() >= next_update {
            launch_update(state.clone(), updating.clone(), false);
            next_update = Instant::now() + Duration::from_secs(3600);
        }
        if state.lock().map(|s| s.update.required()).unwrap_or(true) {
            change(&state, |s| {
                s.connection = Connection::UpdateRequired;
                s.track_title = None;
                s.artist = None;
                s.detail =
                    "Local detection is paused. Update and restart Companion to continue.".into();
            });
            continue;
        }
        let Some(credential) = token.as_ref() else {
            continue;
        };
        let Some(device_id) = preferences.device_id else {
            continue;
        };
        if Instant::now() < next_attempt {
            continue;
        }
        let observation = sample
            .lock()
            .ok()
            .filter(|s| s.captured_at.elapsed() < Duration::from_secs(3))
            .map(|s| s.observation.clone())
            .unwrap_or_else(Observation::unavailable);
        let elapsed = started.elapsed().as_millis() as u64;
        if !force_artwork && !gate.should_publish(&observation, elapsed) {
            continue;
        }
        let result = (|| {
            if session.is_none() {
                session = Some(api.session(credential)?.session_id);
                sequence = 0;
            }
            sequence = sequence.saturating_add(1);
            let frame = PlaybackFrame {
                version: PROTOCOL_VERSION,
                device_id,
                session_id: session.unwrap(),
                sequence,
                observation: if force_artwork {
                    observation.clone()
                } else {
                    gate.for_upload(&observation)
                },
            };
            api.publish(credential, &frame)
        })();
        match result {
            Ok(needs_artwork) => {
                force_artwork = needs_artwork
                    && observation
                        .track
                        .as_ref()
                        .is_some_and(|track| track.artwork_data_url.is_some());
                failures = 0;
                gate.published(observation.clone(), elapsed);
                change(&state, |s| {
                    s.connection = Connection::Connected;
                    s.track_title = observation.track.as_ref().map(|t| t.title.clone());
                    s.artist = observation.track.as_ref().map(|t| t.artist.clone());
                    s.detail = if observation.track.is_some() {
                        "Local playback is connected to Nolae.".into()
                    } else {
                        "Connected. Open Spotify desktop on this PC.".into()
                    };
                });
            }
            Err(ApiError::Rejected | ApiError::Superseded) => {
                token = None;
                session = None;
                let _ = settings::delete_token();
                capture_disabled.store(true, Ordering::Relaxed);
                change(&state, |s| {
                    s.connection = Connection::ReconnectRequired;
                    s.track_title = None;
                    s.artist = None;
                    s.detail = "This device was disconnected or replaced. Pair again from your Nolae queue.".into();
                });
            }
            Err(error) => {
                failures = failures.saturating_add(1);
                let wait = match error {
                    ApiError::Backoff(duration) => duration,
                    _ => Duration::from_secs((1u64 << failures.min(6)).min(60)),
                };
                next_attempt = Instant::now() + wait;
                change(&state, |s| {
                    s.connection = Connection::Offline;
                    s.detail = "Nolae is unreachable. Retrying automatically; hosted polling remains available.".into();
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_notifications_only_wake_for_changes_and_release_the_lock() {
        let state = Arc::new(SharedState::new(AgentSnapshot::default()));
        let wakes = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let observed = wakes.clone();
        let readable = Arc::downgrade(&state);
        *state.wake.lock().unwrap() = Some(Arc::new(move || {
            assert!(readable.upgrade().unwrap().value.try_lock().is_ok());
            observed.fetch_add(1, Ordering::Relaxed);
        }));
        change(&state, |_| {});
        assert_eq!(wakes.load(Ordering::Relaxed), 0);
        change(&state, |snapshot| snapshot.busy = true);
        change(&state, |snapshot| snapshot.busy = true);
        assert_eq!(wakes.load(Ordering::Relaxed), 1);
    }
    use updates::{CheckResult, UpdateNotice, UpdateRequirement, UpdateState};

    fn checked(version: &str) -> CheckResult {
        CheckResult {
            available: Some(UpdateNotice {
                version: version.into(),
                requirement: Some(UpdateRequirement::Critical),
            }),
            archive: None,
        }
    }

    #[test]
    fn automatic_checks_never_download_even_required_updates() {
        let state = SharedState::new(AgentSnapshot::default());
        perform_update(
            &state,
            false,
            || Ok(checked("0.2.0")),
            |_, _| panic!("A check must never download"),
        );
        let snapshot = state.lock().unwrap();
        assert!(snapshot.update.required());
        assert!(snapshot.update.can_download());
        assert!(snapshot.update.installed_version.is_none());
    }

    #[test]
    fn checks_continue_after_install_and_explicit_update_jumps_to_latest() {
        let state = SharedState::new(AgentSnapshot {
            update: UpdateState {
                installed_version: Some("0.2.0".into()),
                ..UpdateState::default()
            },
            ..AgentSnapshot::default()
        });
        perform_update(
            &state,
            false,
            || Ok(checked("0.3.0")),
            |_, _| panic!("A check must never download"),
        );
        assert_eq!(
            state
                .lock()
                .unwrap()
                .update
                .available
                .as_ref()
                .unwrap()
                .version,
            "0.3.0"
        );
        perform_update(
            &state,
            true,
            || Ok(checked("1.0.0")),
            |version, installed| {
                assert_eq!((version, installed), ("1.0.0", "0.2.0"));
                Ok(true)
            },
        );
        assert_eq!(
            state.lock().unwrap().update.installed_version.as_deref(),
            Some("1.0.0")
        );
        perform_update(
            &state,
            true,
            || Ok(checked("1.0.0")),
            |_, _| panic!("Already installed"),
        );
    }

    #[test]
    fn failed_installs_keep_the_running_version_and_can_be_retried() {
        let state = SharedState::new(AgentSnapshot::default());
        perform_update(
            &state,
            true,
            || Ok(checked("0.2.0")),
            |_, _| Err("Update failed verification or could not be installed. Try again.".into()),
        );
        {
            let snapshot = state.lock().unwrap();
            assert!(snapshot.update.installed_version.is_none());
            assert!(snapshot.update.can_download());
            assert_eq!(snapshot.update.activity, updates::UpdateActivity::Idle);
            assert!(snapshot.update.error.is_some());
        }
        perform_update(&state, true, || Ok(checked("0.2.0")), |_, _| Ok(true));
        let snapshot = state.lock().unwrap();
        assert_eq!(snapshot.update.installed_version.as_deref(), Some("0.2.0"));
        assert!(snapshot.update.error.is_none());
    }

    #[test]
    fn failed_checks_preserve_required_update_and_restart_state() {
        let state = SharedState::new(AgentSnapshot {
            update: UpdateState {
                available: checked("0.2.0").available,
                installed_version: Some("0.2.0".into()),
                ..UpdateState::default()
            },
            ..AgentSnapshot::default()
        });
        perform_update(
            &state,
            false,
            || Err("Offline".into()),
            |_, _| panic!("Offline"),
        );
        let snapshot = state.lock().unwrap();
        assert!(snapshot.update.required());
        assert_eq!(snapshot.update.installed_version.as_deref(), Some("0.2.0"));
        assert_eq!(snapshot.update.error.as_deref(), Some("Offline"));
    }
    #[test]
    fn stale_responses_cannot_clear_a_required_update_before_it_is_saved() {
        let state = SharedState::new(AgentSnapshot {
            update: UpdateState {
                available: checked("0.3.0").available,
                ..UpdateState::default()
            },
            ..AgentSnapshot::default()
        });
        for available in [
            None,
            checked("0.2.0").available,
            Some(UpdateNotice {
                version: "0.4.0".into(),
                requirement: None,
            }),
        ] {
            perform_update(
                &state,
                false,
                || {
                    Ok(CheckResult {
                        available,
                        archive: None,
                    })
                },
                |_, _| panic!("A stale check must never install"),
            );
            let snapshot = state.lock().unwrap();
            assert_eq!(snapshot.update.available.as_ref().unwrap().version, "0.3.0");
            assert!(snapshot.update.required());
            assert!(
                snapshot
                    .update
                    .error
                    .as_deref()
                    .unwrap()
                    .contains("backwards")
            );
        }
    }
}
