use super::*;
use sha2::{Digest, Sha256};
use std::net::TcpListener;
use std::os::windows::{fs::OpenOptionsExt, process::CommandExt};
use std::process::Command;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

const TEST: &str = "updates::tests::install_recovery::interrupted_and_failed_updates_preserve_installation_and_allow_retry";
const UPDATED: &[u8] = b"NOLAE SIGNED UPDATE FIXTURE";

struct Server {
    url: String,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Server {
    fn new(version: &str) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let name = archive_name(version);
        let mut core = std::fs::read(std::env::current_exe().unwrap()).unwrap();
        core.extend_from_slice(UPDATED);
        let mut entries: Vec<(&str, &[u8])> = complete();
        entries[0] = (CORE, &core);
        let archive = sign(package(&entries), FIXTURE_KEY, &name);
        let hash: String = Sha256::digest(&archive)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        let sums = format!("{hash}  {name}\n").into_bytes();
        let release = serde_json::to_vec(&serde_json::json!({
            "tag_name": format!("v{version}"),
            "created_at": "2026-01-01T00:00:00Z",
            "assets": [
                {"name": name, "url": format!("{url}/archive"), "digest": format!("sha256:{hash}")},
                {"name": "SHA256SUMS", "url": format!("{url}/sums")}
            ]
        }))
        .unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let thread = std::thread::spawn(move || {
            while !stopped.load(Ordering::Relaxed) {
                let (mut stream, _) = match listener.accept() {
                    Ok(stream) => stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(10));
                        continue;
                    }
                    Err(error) => panic!("{error}"),
                };
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                stream
                    .set_write_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") {
                    let mut byte = [0];
                    stream.read_exact(&mut byte).unwrap();
                    request.push(byte[0]);
                    assert!(request.len() < 16_384);
                }
                let request = String::from_utf8(request).unwrap();
                let path = request.split_whitespace().nth(1).unwrap();
                let body = match path {
                    "/archive" => &archive,
                    "/sums" => &sums,
                    _ if path.starts_with("/repos/fixture/companion/releases/tags/") => &release,
                    _ => panic!("Unexpected fixture request: {path}"),
                };
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .unwrap();
                stream.write_all(body).unwrap();
            }
        });
        Self {
            url,
            stop,
            thread: Some(thread),
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.thread.take().unwrap().join().unwrap();
    }
}

fn update_child(root: &Path, mode: &str) {
    assert_eq!(
        std::fs::read(root.join("recovery.fixture")).unwrap(),
        TEST.as_bytes()
    );
    assert_eq!(std::env::current_exe().unwrap(), root.join(CORE));
    if mode == "relaunch" {
        assert!(std::fs::read(root.join(CORE)).unwrap().ends_with(UPDATED));
        return;
    }
    let version = next_version();
    let server = Server::new(&version);
    let destination = root.join(STAGED_CORE);
    let staged = root.to_owned();
    let target_version = version.clone();
    let interrupt = mode == "interrupt";
    let fail_download = mode == "fail-download";
    let locked = Arc::new(Mutex::new(None));
    let lock = locked.clone();
    let replacing = root.to_owned();
    let result = self_update::backends::github::Update::configure()
        .api_base_url(&server.url)
        .repo_owner("fixture")
        .repo_name("companion")
        .auth_token("fixture")
        .release_tag(format!("v{version}"))
        .bin_name(CORE)
        .bin_install_path(destination)
        .current_version(env!("CARGO_PKG_VERSION"))
        .target(TARGET)
        .checksum_from_asset("SHA256SUMS")
        .timeout(Duration::from_secs(5))
        .show_output(false)
        .show_download_progress(false)
        .no_confirm(true)
        .verifying_keys(vec![fixture_public()])
        .verify_archive(move |archive| {
            stage_update(
                archive,
                &staged,
                &target_version,
                env!("CARGO_PKG_VERSION"),
                &fixture_public(),
            )
            .map_err(self_update::Error::archive_verification_rejected)?;
            std::fs::write(staged.join("reached"), b"staged")?;
            if interrupt {
                std::process::exit(91);
            }
            Ok(())
        })
        .verify_binary(move |binary| {
            std::fs::write(replacing.join("reached"), b"replacement")?;
            if fail_download {
                *lock.lock().unwrap() = Some(
                    std::fs::OpenOptions::new()
                        .read(true)
                        .share_mode(0)
                        .open(binary)?,
                );
            }
            Ok(())
        })
        .build()
        .unwrap()
        .update();
    if fail_download {
        assert!(result.is_err(), "A locked replacement must fail");
    } else {
        assert!(result.unwrap().is_updated());
        if mode == "fail-swap" {
            let _locked = std::fs::OpenOptions::new()
                .read(true)
                .share_mode(3)
                .open(root.join(STAGED_CORE))
                .unwrap();
            assert!(
                replace_core(root, &version)
                    .unwrap_err()
                    .contains("replace")
            );
        } else {
            replace_core(root, &version).unwrap();
            assert_eq!(std::env::current_exe().unwrap(), root.join(CORE));
            assert!(std::fs::read(root.join("ui").join(&version).join(PREVIOUS_CORE)).is_ok());
        }
    }
    drop(locked);
}

#[test]
fn interrupted_and_failed_updates_preserve_installation_and_allow_retry() {
    if let Ok(mode) = std::env::var("NOLAE_UPDATE_TEST_MODE") {
        update_child(
            &std::path::PathBuf::from(std::env::var_os("NOLAE_UPDATE_TEST_ROOT").unwrap()),
            &mode,
        );
        return;
    }
    let install = Install::new();
    let root = install.directory.path();
    let running = std::fs::read(std::env::current_exe().unwrap()).unwrap();
    std::fs::write(root.join(CORE), &running).unwrap();
    std::fs::write(root.join("recovery.fixture"), TEST).unwrap();
    let scratch = root.join("scratch");
    std::fs::create_dir(&scratch).unwrap();
    for mode in [
        "interrupt",
        "fail-download",
        "fail-swap",
        "retry",
        "relaunch",
    ] {
        let output = Command::new(root.join(CORE))
            .args(["--exact", TEST, "--nocapture"])
            .env("NOLAE_UPDATE_TEST_MODE", mode)
            .env("NOLAE_UPDATE_TEST_ROOT", root)
            .env("TEMP", &scratch)
            .env("TMP", &scratch)
            .creation_flags(0x08000000)
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(if mode == "interrupt" { 91 } else { 0 }),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        assert_eq!(
            std::fs::read(root.join("reached")).unwrap(),
            if mode == "interrupt" {
                b"staged".as_slice()
            } else {
                b"replacement".as_slice()
            }
        );
        let current = std::fs::read(root.join(CORE)).unwrap();
        if matches!(mode, "retry" | "relaunch") {
            assert!(current.starts_with(&running));
            assert!(current.ends_with(UPDATED));
        } else {
            assert_eq!(Sha256::digest(&current), Sha256::digest(&running));
        }
        assert_eq!(
            std::fs::read(root.join("ui").join(next_version()).join(WINDOW)).unwrap(),
            b"new window"
        );
        assert_eq!(
            std::fs::read(root.join("ui").join(env!("CARGO_PKG_VERSION")).join(WINDOW)).unwrap(),
            b"running window"
        );
        assert_eq!(std::fs::read(root.join("notes.txt")).unwrap(), b"user file");
    }
}
