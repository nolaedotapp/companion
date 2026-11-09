use nolae_core::protocol::{
    PROTOCOL_VERSION, PairRequest, PairResponse, PlaybackFrame, SessionResponse,
};
use reqwest::{
    blocking::{Client, Response},
    redirect::Policy,
};
use std::time::Duration;

pub const PRODUCTION_SERVER: &str = "https://nolae.app";

#[derive(Debug)]
pub enum ApiError {
    Unavailable,
    Rejected,
    Superseded,
    Backoff(Duration),
    InvalidResponse,
}

pub struct Api {
    client: Client,
    base: String,
}
impl Api {
    pub fn new(base: &str) -> Result<Self, ApiError> {
        if !allowed_server(base, cfg!(debug_assertions)) {
            return Err(ApiError::Rejected);
        }
        let client = Client::builder()
            .https_only(!base.starts_with("http://"))
            .redirect(Policy::none())
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(10))
            .user_agent(concat!("nolae-companion/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|_| ApiError::Unavailable)?;
        Ok(Self {
            client,
            base: base.trim_end_matches('/').into(),
        })
    }

    fn post<T: serde::Serialize>(
        &self,
        action: &str,
        token: Option<&str>,
        body: &T,
    ) -> Result<Response, ApiError> {
        let mut request = self
            .client
            .post(format!("{}/api/local-playback/{action}", self.base))
            .header("X-Nolae-Local-Client", "1")
            .json(body);
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        let response = request.send().map_err(|_| ApiError::Unavailable)?;
        match response.status().as_u16() {
            200..=299 => Ok(response),
            401 | 403 => Err(ApiError::Rejected),
            409 => Err(ApiError::Superseded),
            429 | 503 => {
                let wait = response
                    .headers()
                    .get("Retry-After")
                    .and_then(|h| h.to_str().ok())
                    .and_then(|s| s.parse::<u64>().ok())
                    .unwrap_or(15)
                    .clamp(1, 86_400);
                Err(ApiError::Backoff(Duration::from_secs(wait)))
            }
            _ => Err(ApiError::Unavailable),
        }
    }

    pub fn pair(&self, code: &str) -> Result<PairResponse, ApiError> {
        if code.len() != 24
            || !code
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
        {
            return Err(ApiError::Rejected);
        }
        let request = PairRequest {
            version: PROTOCOL_VERSION,
            code,
            device_name: "Windows Spotify",
        };
        let response = self.post("pair", None, &request)?;
        let body = response.bytes().map_err(|_| ApiError::InvalidResponse)?;
        if body.len() > 4_096 {
            return Err(ApiError::InvalidResponse);
        }
        let paired: PairResponse =
            serde_json::from_slice(&body).map_err(|_| ApiError::InvalidResponse)?;
        let parts: Vec<_> = paired.device_token.split('.').collect();
        if paired.version != PROTOCOL_VERSION
            || paired.device_token.len() > 512
            || parts.len() != 3
            || parts[0].is_empty()
            || parts[1] != paired.device_id.to_string()
            || parts[2].len() != 43
            || !parts[2]
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
        {
            return Err(ApiError::InvalidResponse);
        }
        Ok(paired)
    }

    pub fn session(&self, token: &str) -> Result<SessionResponse, ApiError> {
        self.post("session", Some(token), &serde_json::json!({}))?
            .json()
            .map_err(|_| ApiError::InvalidResponse)
    }
    pub fn publish(&self, token: &str, frame: &PlaybackFrame) -> Result<bool, ApiError> {
        self.post("events", Some(token), frame).map(|response| {
            response
                .headers()
                .get("X-Nolae-Needs-Artwork")
                .is_some_and(|value| value == "1")
        })
    }
    pub fn disconnect(&self, token: &str) -> Result<(), ApiError> {
        self.post("disconnect", Some(token), &serde_json::json!({}))
            .map(|_| ())
    }
}

fn allowed_server(base: &str, development: bool) -> bool {
    if base == PRODUCTION_SERVER {
        return true;
    }
    if !development {
        return false;
    }
    let Ok(url) = reqwest::Url::parse(base) else {
        return false;
    };
    url.scheme() == "http"
        && matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"))
        && url.username().is_empty()
        && url.password().is_none()
        && url.path() == "/"
        && url.query().is_none()
        && url.fragment().is_none()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::mpsc,
    };

    fn endpoint(status: &str, headers: &str, body: &str) -> (Api, mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let api = Api::new(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
        let response = format!(
            "HTTP/1.1 {status}\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut bytes = Vec::new();
            let mut buffer = [0; 4096];
            loop {
                let count = stream.read(&mut buffer).unwrap();
                if count == 0 {
                    break;
                }
                bytes.extend_from_slice(&buffer[..count]);
                if let Some(end) = bytes.windows(4).position(|s| s == b"\r\n\r\n") {
                    let header = String::from_utf8_lossy(&bytes[..end]).to_ascii_lowercase();
                    let length: usize = header
                        .lines()
                        .find_map(|l| l.strip_prefix("content-length: "))
                        .unwrap_or("0")
                        .trim()
                        .parse()
                        .unwrap();
                    if bytes.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            let _ = tx.send(String::from_utf8(bytes).unwrap());
            stream.write_all(response.as_bytes()).unwrap();
        });
        (api, rx)
    }

    #[test]
    fn device_calls_are_scoped_and_honor_retry_after_without_browser_authority() {
        let (api, request) = endpoint("429 Too Many Requests", "Retry-After: 1234\r\n", "{}");
        assert!(
            matches!(api.session("fixture-device-credential"), Err(ApiError::Backoff(wait)) if wait == Duration::from_secs(1234))
        );
        let request = request
            .recv_timeout(Duration::from_secs(3))
            .unwrap()
            .to_ascii_lowercase();
        assert!(request.starts_with("post /api/local-playback/session "));
        assert!(request.contains("authorization: bearer fixture-device-credential\r\n"));
        assert!(request.contains("x-nolae-local-client: 1\r\n"));
        assert!(!request.contains("cookie:"));
        assert!(!request.contains("origin:"));
    }

    #[test]
    fn redirect_cannot_forward_a_device_credential() {
        let target = TcpListener::bind("127.0.0.1:0").unwrap();
        target.set_nonblocking(true).unwrap();
        let (api, _) = endpoint(
            "307 Temporary Redirect",
            &format!(
                "Location: http://{}/collect\r\n",
                target.local_addr().unwrap()
            ),
            "",
        );
        assert!(matches!(
            api.disconnect("fixture-device-credential"),
            Err(ApiError::Unavailable)
        ));
        assert_eq!(
            target.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
    }

    #[test]
    fn pairing_rejects_invalid_credentials() {
        let body = r#"{"version":1,"deviceId":"11111111-1111-4111-8111-111111111111","deviceToken":"","displayName":"PC"}"#;
        let (api, _) = endpoint("200 OK", "Content-Type: application/json\r\n", body);
        assert!(matches!(
            api.pair("abcdefghijklmnopqrstuvwx"),
            Err(ApiError::InvalidResponse)
        ));
    }
    #[test]
    fn credentials_cannot_be_sent_to_arbitrary_or_lookalike_origins() {
        assert!(allowed_server(PRODUCTION_SERVER, false));
        assert!(allowed_server("http://127.0.0.1:3000", true));
        for origin in [
            "http://nolae.app",
            "https://nolae.app.attacker.invalid",
            "https://nolae.app@attacker.invalid",
            "https://nolae.app/path",
            "http://127.0.0.1:3000",
        ] {
            assert!(!allowed_server(origin, false));
        }
    }
}
