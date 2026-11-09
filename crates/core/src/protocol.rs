use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const PROTOCOL_VERSION: u8 = 1;
pub const HEARTBEAT_MS: u64 = 5_000;
pub const MAX_TRACK_DURATION_MS: u64 = 24 * 60 * 60 * 1_000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Track {
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    pub duration_ms: u64,
    pub position_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artwork_data_url: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlaybackState {
    Playing,
    Paused,
    Unavailable,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Observation {
    pub state: PlaybackState,
    pub track: Option<Track>,
}

impl Observation {
    pub fn unavailable() -> Self {
        Self {
            state: PlaybackState::Unavailable,
            track: None,
        }
    }

    pub fn validate(&self) -> Result<(), &'static str> {
        match (&self.state, &self.track) {
            (PlaybackState::Unavailable, None) => Ok(()),
            (PlaybackState::Unavailable, Some(_)) | (_, None) => Err("inconsistent playback state"),
            (_, Some(track)) => {
                if track.title.trim().is_empty()
                    || track.title.encode_utf16().count() > 512
                    || track.artist.encode_utf16().count() > 512
                    || track
                        .album
                        .as_ref()
                        .is_some_and(|s| s.encode_utf16().count() > 512)
                    || track.duration_ms > MAX_TRACK_DURATION_MS
                    || track.position_ms > MAX_TRACK_DURATION_MS
                    || (track.duration_ms > 0 && track.position_ms > track.duration_ms)
                {
                    return Err("invalid track metadata");
                }
                if let Some(art) = &track.artwork_data_url
                    && (art.len() > 180_000
                        || !(art.starts_with("data:image/png;base64,")
                            || art.starts_with("data:image/jpeg;base64,")))
                {
                    return Err("invalid artwork");
                }
                Ok(())
            }
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlaybackFrame {
    pub version: u8,
    pub device_id: Uuid,
    pub session_id: Uuid,
    pub sequence: u64,
    pub observation: Observation,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairRequest<'a> {
    pub version: u8,
    pub code: &'a str,
    pub device_name: &'a str,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PairResponse {
    pub version: u8,
    pub device_id: Uuid,
    pub device_token: String,
    pub display_name: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SessionResponse {
    pub session_id: Uuid,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn playback_state_requires_matching_track_data() {
        assert!(Observation::unavailable().validate().is_ok());
        assert!(
            Observation {
                state: PlaybackState::Playing,
                track: None
            }
            .validate()
            .is_err()
        );
        let track = Track {
            title: "Track".into(),
            artist: "Artist".into(),
            album: None,
            duration_ms: 180_000,
            position_ms: 0,
            artwork_data_url: None,
        };
        assert!(
            Observation {
                state: PlaybackState::Unavailable,
                track: Some(track)
            }
            .validate()
            .is_err()
        );
    }

    #[test]
    fn unexpected_observation_fields_are_rejected() {
        for field in ["spotifyToken", "spotifyUri", "command"] {
            let value = format!(r#"{{"state":"unavailable","track":null,"{field}":"unexpected"}}"#);
            assert!(serde_json::from_str::<Observation>(&value).is_err());
        }
    }

    #[test]
    fn unicode_limits_match_the_servers_utf16_contract() {
        let mut observation = Observation {
            state: PlaybackState::Playing,
            track: Some(Track {
                title: "🎵".repeat(256),
                artist: "Artist".into(),
                album: None,
                duration_ms: 180_000,
                position_ms: 1_000,
                artwork_data_url: None,
            }),
        };
        assert!(observation.validate().is_ok());
        observation.track.as_mut().unwrap().title.push('🎵');
        assert!(observation.validate().is_err());
    }
}
