use crate::protocol::{HEARTBEAT_MS, Observation, PlaybackState};

pub trait PlaybackProvider {
    fn snapshot(&mut self) -> Result<Observation, String>;
}

#[derive(Default)]
pub struct PublishGate {
    last: Option<(Observation, u64)>,
}

impl PublishGate {
    pub fn for_upload(&self, current: &Observation) -> Observation {
        let mut upload = current.clone();
        if let (Some(track), Some((previous, _))) = (&mut upload.track, &self.last)
            && let Some(before) = &previous.track
            && track.title == before.title
            && track.artist == before.artist
            && track.album == before.album
            && track.artwork_data_url == before.artwork_data_url
        {
            track.artwork_data_url = None;
        }
        upload
    }
    pub fn should_publish(&self, current: &Observation, now_ms: u64) -> bool {
        let Some((previous, sent_at)) = &self.last else {
            return true;
        };
        let elapsed = now_ms.saturating_sub(*sent_at);
        if elapsed >= HEARTBEAT_MS || current.state != previous.state {
            return true;
        }
        match (&current.track, &previous.track) {
            (Some(a), Some(b)) => {
                let expected = if previous.state == PlaybackState::Playing {
                    let projected = b.position_ms.saturating_add(elapsed);
                    if b.duration_ms > 0 {
                        projected.min(b.duration_ms)
                    } else {
                        projected
                    }
                } else {
                    b.position_ms
                };
                a.title != b.title
                    || a.artist != b.artist
                    || a.album != b.album
                    || a.duration_ms != b.duration_ms
                    || a.artwork_data_url != b.artwork_data_url
                    || a.position_ms.abs_diff(expected) > 1_000
            }
            (None, None) => false,
            _ => true,
        }
    }

    pub fn published(&mut self, observation: Observation, now_ms: u64) {
        self.last = Some((observation, now_ms));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::Track;

    fn song(position_ms: u64) -> Observation {
        Observation {
            state: PlaybackState::Playing,
            track: Some(Track {
                title: "Track".into(),
                artist: "Artist".into(),
                album: None,
                duration_ms: 180_000,
                position_ms,
                artwork_data_url: None,
            }),
        }
    }

    #[test]
    fn interpolated_progress_does_not_send_every_local_read() {
        let mut gate = PublishGate::default();
        assert!(gate.should_publish(&song(0), 0));
        gate.published(song(0), 0);
        for elapsed in (500..5_000).step_by(500) {
            assert!(!gate.should_publish(&song(elapsed), elapsed));
        }
        assert!(gate.should_publish(&song(5_000), 5_000));
    }

    #[test]
    fn seek_pause_and_missing_session_are_immediate() {
        let mut gate = PublishGate::default();
        gate.published(song(0), 0);
        assert!(gate.should_publish(&song(60_000), 500));
        let mut paused = song(500);
        paused.state = PlaybackState::Paused;
        assert!(gate.should_publish(&paused, 500));
        assert!(gate.should_publish(&Observation::unavailable(), 500));
    }

    #[test]
    fn heartbeats_omit_unchanged_cover_bytes_but_new_tracks_send_them() {
        let mut observation = song(0);
        observation.track.as_mut().unwrap().artwork_data_url =
            Some("data:image/png;base64,aGVsbG8=".into());
        let mut gate = PublishGate::default();
        assert!(
            gate.for_upload(&observation)
                .track
                .unwrap()
                .artwork_data_url
                .is_some()
        );
        gate.published(observation.clone(), 0);
        assert!(
            gate.for_upload(&observation)
                .track
                .unwrap()
                .artwork_data_url
                .is_none()
        );
        observation.track.as_mut().unwrap().title = "Another song".into();
        assert!(
            gate.for_upload(&observation)
                .track
                .unwrap()
                .artwork_data_url
                .is_some()
        );
    }
}
