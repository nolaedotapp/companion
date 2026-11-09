use nolae_core::playback::PlaybackProvider;
use nolae_core::protocol::{Observation, PlaybackState, Track};

#[cfg(any(windows, test))]
fn artwork_data_url(bytes: &[u8], mime: &str) -> Option<String> {
    use base64::{Engine, engine::general_purpose::STANDARD};
    use std::io::Cursor;

    if bytes.is_empty() || bytes.len() > 4_194_304 {
        return None;
    }
    let format = match mime {
        "image/png" => image::ImageFormat::Png,
        "image/jpeg" => image::ImageFormat::Jpeg,
        _ => return None,
    };
    if bytes.len() <= 130_000 {
        return Some(format!("data:{mime};base64,{}", STANDARD.encode(bytes)));
    }
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4_096);
    limits.max_image_height = Some(4_096);
    limits.max_alloc = Some(67_108_864);
    let mut reader = image::ImageReader::with_format(Cursor::new(bytes), format);
    reader.limits(limits);
    let decoded = reader.decode().ok()?;
    for edge in [384, 256, 192, 128] {
        let mut output = Cursor::new(Vec::new());
        decoded
            .thumbnail(edge, edge)
            .write_to(&mut output, image::ImageFormat::Png)
            .ok()?;
        let resized = output.into_inner();
        if resized.len() <= 130_000 {
            return Some(format!(
                "data:image/png;base64,{}",
                STANDARD.encode(resized)
            ));
        }
    }
    None
}

fn is_spotify_session(id: &str) -> bool {
    let id = id.to_ascii_lowercase();
    matches!(id.as_str(), "spotify" | "spotify.exe")
        || (id.starts_with("spotifyab.spotifymusic_") && id.ends_with("!spotify"))
}

#[cfg(windows)]
mod native {
    use super::*;
    fn wait<T: windows::core::RuntimeType>(
        operation: windows_future::IAsyncOperation<T>,
    ) -> windows::core::Result<T> {
        let started = std::time::Instant::now();
        while operation.Status()? == windows_future::AsyncStatus::Started {
            if started.elapsed() >= std::time::Duration::from_secs(2) {
                let _ = operation.Cancel();
                return Err(windows::core::Error::from_hresult(
                    windows::core::HRESULT::from_win32(1460),
                ));
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        operation.GetResults()
    }
    use windows::{
        Media::Control::{
            GlobalSystemMediaTransportControlsSession as Session,
            GlobalSystemMediaTransportControlsSessionManager as Manager,
            GlobalSystemMediaTransportControlsSessionPlaybackStatus as Status,
        },
        Storage::Streams::DataReader,
        Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize, RoUninitialize},
    };

    pub struct WindowsPlayback {
        manager: Manager,
        artwork_key: String,
        artwork: Option<String>,
        previous_artwork: Option<String>,
        artwork_attempts_left: u8,
        thread_bound: std::marker::PhantomData<std::rc::Rc<()>>,
    }

    impl WindowsPlayback {
        pub fn new() -> Result<Self, String> {
            unsafe { RoInitialize(RO_INIT_MULTITHREADED) }
                .map_err(|_| "Windows media access is unavailable")?;
            match Manager::RequestAsync().and_then(wait) {
                Ok(manager) => Ok(Self {
                    manager,
                    artwork_key: String::new(),
                    artwork: None,
                    previous_artwork: None,
                    artwork_attempts_left: 0,
                    thread_bound: std::marker::PhantomData,
                }),
                Err(_) => {
                    unsafe { RoUninitialize() };
                    Err("Could not open Windows media sessions".into())
                }
            }
        }

        fn selected(&self) -> Option<Session> {
            let sessions = self.manager.GetSessions().ok()?;
            let mut paused = None;
            let mut playing = None;
            for session in sessions {
                let Ok(id) = session.SourceAppUserModelId() else {
                    continue;
                };
                if !is_spotify_session(&id.to_string()) {
                    continue;
                }
                if session
                    .GetPlaybackInfo()
                    .is_ok_and(|p| p.PlaybackStatus().ok() == Some(Status::Playing))
                {
                    if playing.is_some() {
                        return None;
                    }
                    playing = Some(session);
                } else if paused.is_none() {
                    paused = Some(session);
                }
            }
            playing.or(paused)
        }
    }

    impl Drop for WindowsPlayback {
        fn drop(&mut self) {
            unsafe { RoUninitialize() };
        }
    }

    impl PlaybackProvider for WindowsPlayback {
        fn snapshot(&mut self) -> Result<Observation, String> {
            let Some(session) = self.selected() else {
                return Ok(Observation::unavailable());
            };
            let result = (|| -> windows::core::Result<Observation> {
                let properties = wait(session.TryGetMediaPropertiesAsync()?)?;
                let title = properties.Title()?.to_string();
                if title.trim().is_empty() {
                    return Ok(Observation::unavailable());
                }
                let artist = properties.Artist()?.to_string();
                let album = properties.AlbumTitle()?.to_string();
                let playing = session.GetPlaybackInfo()?.PlaybackStatus()? == Status::Playing;
                let timeline = session.GetTimelineProperties()?;
                let start = timeline.StartTime()?.Duration;
                let duration =
                    timeline.EndTime()?.Duration.saturating_sub(start).max(0) as u64 / 10_000;
                let mut position =
                    timeline.Position()?.Duration.saturating_sub(start).max(0) as u64 / 10_000;
                if playing {
                    let unix_ms = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis() as i64;
                    let observed_ms =
                        timeline.LastUpdatedTime()?.UniversalTime / 10_000 - 11_644_473_600_000;
                    let elapsed = if observed_ms > 0 {
                        unix_ms.saturating_sub(observed_ms).clamp(0, 86_400_000) as u64
                    } else {
                        0
                    };
                    position = position.saturating_add(elapsed);
                }
                if duration > 0 {
                    position = position.min(duration);
                }
                let artwork_key = format!("{title}\0{artist}\0{album}");
                if self.artwork_key != artwork_key {
                    self.artwork_key = artwork_key;
                    self.previous_artwork = self.artwork.take();
                    self.artwork_attempts_left = 4;
                }
                if self.artwork_attempts_left > 0 {
                    self.artwork_attempts_left -= 1;
                    let candidate = (|| -> windows::core::Result<Option<String>> {
                        let stream = wait(properties.Thumbnail()?.OpenReadAsync()?)?;
                        let size = stream.Size()?;
                        let mime = stream.ContentType()?.to_string();
                        if size == 0
                            || size > 4_194_304
                            || !matches!(mime.as_str(), "image/png" | "image/jpeg")
                        {
                            return Ok(None);
                        }
                        let reader = DataReader::CreateDataReader(&stream)?;
                        let loaded = wait(reader.LoadAsync(size as u32)?)?;
                        if loaded != size as u32 {
                            return Ok(None);
                        }
                        let mut bytes = vec![0u8; size as usize];
                        reader.ReadBytes(&mut bytes)?;
                        Ok(artwork_data_url(&bytes, &mime))
                    })()
                    .unwrap_or(None);
                    if candidate != self.previous_artwork || self.artwork_attempts_left == 0 {
                        self.artwork = candidate;
                        if self.artwork.is_some() {
                            self.artwork_attempts_left = 0;
                        }
                    }
                }
                let observation = Observation {
                    state: if playing {
                        PlaybackState::Playing
                    } else {
                        PlaybackState::Paused
                    },
                    track: Some(Track {
                        title,
                        artist,
                        album: (!album.is_empty()).then_some(album),
                        duration_ms: duration,
                        position_ms: position,
                        artwork_data_url: self.artwork.clone(),
                    }),
                };
                Ok(observation)
            })();
            let observation =
                result.map_err(|_| "Could not read Spotify's Windows media session".to_string())?;
            observation.validate().map_err(str::to_string)?;
            Ok(observation)
        }
    }
}

#[cfg(windows)]
pub use native::WindowsPlayback;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oversized_spotify_artwork_is_resized_within_the_existing_upload_limit() {
        use base64::{Engine, engine::general_purpose::STANDARD};
        use std::io::Cursor;

        let mut seed = 17u32;
        let pixels: Vec<u8> = (0..512 * 512 * 3)
            .map(|_| {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                seed as u8
            })
            .collect();
        let image = image::RgbImage::from_raw(512, 512, pixels).unwrap();
        let mut original = Cursor::new(Vec::new());
        image
            .write_to(&mut original, image::ImageFormat::Png)
            .unwrap();
        assert!(original.get_ref().len() > 130_000);
        let data = artwork_data_url(original.get_ref(), "image/png").unwrap();
        assert!(data.len() < 180_000);
        let bytes = STANDARD
            .decode(data.strip_prefix("data:image/png;base64,").unwrap())
            .unwrap();
        assert!(bytes.len() <= 130_000);
        let resized = image::load_from_memory(&bytes).unwrap();
        assert!(resized.width() <= 384 && resized.height() <= 384);
        assert_eq!(artwork_data_url(&bytes, "image/png").unwrap(), data);
        assert!(artwork_data_url(b"invalid", "image/svg+xml").is_none());
    }
    #[test]
    fn limits_collection_to_spotify_desktop_sessions() {
        assert!(is_spotify_session("Spotify.exe"));
        assert!(is_spotify_session(
            "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify"
        ));
        for id in [
            "VLC",
            "MySpotifyRecorder.exe",
            "SpotifyAB.SpotifyMusic_fake!OtherApp",
        ] {
            assert!(!is_spotify_session(id));
        }
    }
}
