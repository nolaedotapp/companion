use super::*;
use std::io::Write;

const FIXTURE_KEY: [u8; 32] = [41; 32];

fn next_version() -> String {
    let mut version = Version::parse(env!("CARGO_PKG_VERSION")).unwrap();
    version.patch += 1;
    version.to_string()
}

fn fixture_public() -> [u8; 32] {
    zipsign_api::SigningKey::from_bytes(&FIXTURE_KEY)
        .verifying_key()
        .to_bytes()
}

fn package(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, bytes) in entries {
        zip.start_file(*name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(bytes).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

fn with_duplicate(name: &str, placeholder: &str) -> Vec<u8> {
    let mut entries = complete();
    entries.push((placeholder, b"second copy"));
    let mut bytes = package(&entries);
    let (from, to) = (placeholder.as_bytes(), name.as_bytes());
    let mut index = 0;
    while let Some(found) = bytes[index..].windows(from.len()).position(|w| w == from) {
        bytes[index + found..index + found + to.len()].copy_from_slice(to);
        index += found + to.len();
    }
    bytes
}

fn complete() -> Vec<(&'static str, &'static [u8])> {
    vec![
        ("nolae-companion.exe", b"new core"),
        ("nolae-companion-ui.exe", b"new window"),
        ("licenses/LICENSE", b"license"),
        ("licenses/OFL-Manrope.txt", b"font license"),
        ("licenses/THIRD_PARTY_NOTICES.html", b"notices"),
    ]
}

fn sign(bytes: Vec<u8>, key: [u8; 32], context: &str) -> Vec<u8> {
    let mut signed = Cursor::new(Vec::new());
    zipsign_api::sign::copy_and_sign_zip(
        &mut Cursor::new(bytes),
        &mut signed,
        &[zipsign_api::SigningKey::from_bytes(&key)],
        Some(context.as_bytes()),
    )
    .unwrap();
    signed.into_inner()
}

struct Install {
    directory: tempfile::TempDir,
}

impl Install {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let running = directory.path().join("ui").join(env!("CARGO_PKG_VERSION"));
        std::fs::create_dir_all(&running).unwrap();
        std::fs::write(running.join(WINDOW), b"running window").unwrap();
        std::fs::write(directory.path().join("LICENSE"), b"old license").unwrap();
        std::fs::write(directory.path().join("nolae.ico"), b"icon").unwrap();
        std::fs::write(directory.path().join("notes.txt"), b"user file").unwrap();
        Self { directory }
    }

    fn stage(&self, file_name: &str, bytes: &[u8], version: &str) -> Result<(), String> {
        let downloads = tempfile::tempdir().unwrap();
        let archive = downloads.path().join(file_name.replace('/', "_"));
        std::fs::write(&archive, bytes).unwrap();
        stage_update(
            &archive,
            self.directory.path(),
            version,
            env!("CARGO_PKG_VERSION"),
            &fixture_public(),
        )
    }

    fn untouched(&self) {
        let root = self.directory.path();
        assert_eq!(
            std::fs::read(root.join("ui").join(env!("CARGO_PKG_VERSION")).join(WINDOW)).unwrap(),
            b"running window"
        );
        assert!(!root.join("ui").join(next_version()).join(WINDOW).exists());
        assert!(!root.join("licenses").exists());
        assert_eq!(std::fs::read(root.join("LICENSE")).unwrap(), b"old license");
    }
}

#[test]
fn signed_update_stages_window_and_licenses_without_touching_user_files() {
    let install = Install::new();
    let version = next_version();
    let name = archive_name(&version);
    install
        .stage(
            &name,
            &sign(package(&complete()), FIXTURE_KEY, &name),
            &version,
        )
        .unwrap();
    let root = install.directory.path();
    assert_eq!(
        std::fs::read(root.join("ui").join(&version).join(WINDOW)).unwrap(),
        b"new window"
    );
    assert_eq!(
        std::fs::read(root.join("ui").join(env!("CARGO_PKG_VERSION")).join(WINDOW)).unwrap(),
        b"running window"
    );
    assert_eq!(
        std::fs::read(root.join("licenses/LICENSE")).unwrap(),
        b"license"
    );
    assert!(root.join("licenses/THIRD_PARTY_NOTICES.html").exists());
    assert!(!root.join("LICENSE").exists());
    assert!(!root.join("nolae-companion.exe").exists());
    assert_eq!(std::fs::read(root.join("notes.txt")).unwrap(), b"user file");
    assert_eq!(std::fs::read(root.join("nolae.ico")).unwrap(), b"icon");
}

#[test]
fn unsigned_tampered_or_foreign_updates_are_rejected_before_staging() {
    let install = Install::new();
    let version = next_version();
    let name = archive_name(&version);
    let unsigned = package(&complete());
    assert!(install.stage(&name, &unsigned, &version).is_err());
    let foreign = sign(unsigned.clone(), [7; 32], &name);
    assert!(install.stage(&name, &foreign, &version).is_err());
    let mut tampered = sign(unsigned.clone(), FIXTURE_KEY, &name);
    let last = tampered.len() - 30;
    tampered[last] ^= 1;
    assert!(install.stage(&name, &tampered, &version).is_err());
    let renamed = sign(unsigned, FIXTURE_KEY, &archive_name("999.0.0"));
    assert!(install.stage(&name, &renamed, &version).is_err());
    install.untouched();
}

#[test]
fn signed_archives_cannot_be_replayed_as_another_or_older_version() {
    let install = Install::new();
    let old = archive_name(env!("CARGO_PKG_VERSION"));
    let old_package = sign(package(&complete()), FIXTURE_KEY, &old);
    assert!(
        install
            .stage(&old, &old_package, env!("CARGO_PKG_VERSION"))
            .is_err()
    );
    assert!(install.stage(&old, &old_package, &next_version()).is_err());
    for version in ["0.2", "0.2.0-beta.1", "0.2.0+build", "../0.2.0", "00.2.0"] {
        let name = archive_name(version);
        let bytes = sign(package(&complete()), FIXTURE_KEY, &name);
        assert!(install.stage(&name, &bytes, version).is_err(), "{version}");
    }
    install.untouched();
}

#[test]
fn malformed_or_unsafe_archive_components_are_rejected() {
    let version = next_version();
    let name = archive_name(&version);
    let mut cases: Vec<Vec<(&str, &[u8])>> = vec![
        complete()[1..].to_vec(),
        complete()[..4].to_vec(),
        [complete(), vec![("../escape.exe", b"x")]].concat(),
        [complete(), vec![("/absolute.exe", b"x")]].concat(),
        [complete(), vec![("licenses\\LICENSE.exe", b"x")]].concat(),
        [complete(), vec![("C:/escape.exe", b"x")]].concat(),
        [complete(), vec![("Nolae-Companion-UI.exe", b"x")]].concat(),
    ];
    let mut empty = complete();
    empty[1].1 = b"";
    cases.push(empty);
    let mut crowded = complete();
    let extra: Vec<String> = (0..MAX_ENTRIES)
        .map(|index| format!("extra/{index}"))
        .collect();
    crowded.extend(extra.iter().map(|name| (name.as_str(), b"x" as &[u8])));
    cases.push(crowded);
    for (placeholder, real) in [
        ("nolae-companion-ui.exX", WINDOW),
        ("nolae-companion.exX", CORE),
    ] {
        let mut archive =
            zip::ZipArchive::new(Cursor::new(with_duplicate(real, placeholder))).unwrap();
        assert!(validate_entries(&mut archive).is_err());
    }
    for (case, entries) in cases.iter().enumerate() {
        let install = Install::new();
        let bytes = sign(package(entries), FIXTURE_KEY, &name);
        assert!(
            install.stage(&name, &bytes, &version).is_err(),
            "case {case}"
        );
        install.untouched();
    }
}

#[test]
fn interrupted_staging_keeps_the_running_files_and_a_retry_completes() {
    let install = Install::new();
    let root = install.directory.path();
    let version = next_version();
    let staged = root.join("ui").join(&version);
    std::fs::create_dir_all(&staged).unwrap();
    std::fs::write(staged.join(WINDOW), b"partial").unwrap();
    std::fs::write(staged.join(".tmpLeftover"), b"partial").unwrap();
    std::fs::write(root.join("licenses"), b"blocks the directory").unwrap();
    let name = archive_name(&version);
    let bytes = sign(package(&complete()), FIXTURE_KEY, &name);
    assert!(install.stage(&name, &bytes, &version).is_err());
    assert_eq!(
        std::fs::read(root.join("ui").join(env!("CARGO_PKG_VERSION")).join(WINDOW)).unwrap(),
        b"running window"
    );
    assert_eq!(std::fs::read(root.join("LICENSE")).unwrap(), b"old license");
    std::fs::remove_file(root.join("licenses")).unwrap();
    install.stage(&name, &bytes, &version).unwrap();
    install.stage(&name, &bytes, &version).unwrap();
    assert_eq!(std::fs::read(staged.join(WINDOW)).unwrap(), b"new window");
    assert_eq!(
        std::fs::read(root.join("ui").join(env!("CARGO_PKG_VERSION")).join(WINDOW)).unwrap(),
        b"running window"
    );
    assert!(root.join("licenses/OFL-Manrope.txt").exists());
}

#[test]
fn published_history_matches_this_build() {
    let policy: Policy =
        serde_json::from_str(include_str!("../../../release/updates.json")).unwrap();
    policy.validate().unwrap();
    assert_eq!(
        policy.releases.last().unwrap().version.to_string(),
        env!("CARGO_PKG_VERSION")
    );
}

fn policy(entries: &[(&str, Priority)]) -> Policy {
    Policy {
        schema_version: 1,
        releases: entries
            .iter()
            .map(|(version, priority)| PolicyRelease {
                version: Version::parse(version).unwrap(),
                priority: *priority,
            })
            .collect(),
    }
}

#[test]
fn requirements_count_actual_releases_and_retain_critical_fixes() {
    let current = Version::parse("0.1.0").unwrap();
    let mut history = policy(&[
        ("0.1.0", Priority::Normal),
        ("0.5.0", Priority::Normal),
        ("2.0.0", Priority::Normal),
    ]);
    assert_eq!(history.notice(&current).unwrap().requirement, None);
    history.releases.push(PolicyRelease {
        version: Version::parse("2.0.1").unwrap(),
        priority: Priority::Normal,
    });
    assert_eq!(
        history.notice(&current).unwrap().requirement,
        Some(UpdateRequirement::Outdated)
    );
    history.releases[1].priority = Priority::Critical;
    assert_eq!(
        history.notice(&current).unwrap().requirement,
        Some(UpdateRequirement::Critical)
    );
    assert_eq!(
        history
            .notice(&Version::parse("0.5.0").unwrap())
            .unwrap()
            .requirement,
        None
    );
    assert!(history.notice(&Version::parse("2.0.1").unwrap()).is_none());
}

#[test]
fn downloaded_update_does_not_hide_newer_releases() {
    let mut state = UpdateState {
        installed_version: Some("0.2.0".into()),
        ..UpdateState::default()
    };
    state.available = Some(UpdateNotice {
        version: "0.2.0".into(),
        requirement: None,
    });
    assert!(!state.can_download());
    state.available.as_mut().unwrap().version = "0.3.0".into();
    assert!(state.can_download());
    assert_eq!(
        state.available.unwrap().changelog_url(),
        "https://github.com/nolaedotapp/companion/releases/tag/v0.3.0"
    );
}

#[test]
fn unsigned_or_modified_policy_cannot_require_an_update() {
    let key = zipsign_api::SigningKey::from_bytes(&[23; 32]);
    let mut unsigned = zip::ZipWriter::new(Cursor::new(Vec::new()));
    unsigned
        .start_file("updates.json", zip::write::SimpleFileOptions::default())
        .unwrap();
    unsigned.write_all(br#"{"schemaVersion":1,"releases":[{"version":"0.1.0","priority":"normal"},{"version":"0.2.0","priority":"critical"}]}"#).unwrap();
    let mut unsigned = unsigned.finish().unwrap();
    let public = key.verifying_key().to_bytes();
    assert!(decode(unsigned.get_ref(), &public).is_err());
    let mut signed = Cursor::new(Vec::new());
    zipsign_api::sign::copy_and_sign_zip(
        &mut unsigned,
        &mut signed,
        &[key],
        Some(FEED_NAME.as_bytes()),
    )
    .unwrap();
    let mut bytes = signed.into_inner();
    assert!(decode(&bytes, &public).is_ok());
    bytes[40] ^= 1;
    assert!(decode(&bytes, &public).is_err());
}

#[test]
fn ambiguous_or_unstable_release_history_is_rejected() {
    for versions in [
        ["0.2.0", "0.1.0"],
        ["0.1.0", "0.1.0"],
        ["0.1.0", "0.2.0-beta.1"],
        ["0.1.0", "0.2.0+build"],
    ] {
        assert!(
            policy(&versions.map(|version| (version, Priority::Normal)))
                .validate()
                .is_err()
        );
    }
}

#[cfg(windows)]
#[path = "update_install_tests.rs"]
mod install_recovery;
