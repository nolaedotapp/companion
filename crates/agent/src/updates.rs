use semver::Version;
use serde::Deserialize;
use std::io::{Cursor, Read};
use std::path::Path;

const PUBLISHER_KEY: [u8; 32] = *include_bytes!("../../../release/publisher.pub");
const FEED_NAME: &str = "companion-updates.zip";
const FEED_URL: &str =
    "https://github.com/nolaedotapp/companion/releases/latest/download/companion-updates.zip";
const MAX_ARCHIVE: u64 = 131_072;
const MAX_POLICY: u64 = 65_536;

pub use nolae_core::companion::{UpdateActivity, UpdateNotice, UpdateRequirement, UpdateState};

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Policy {
    schema_version: u32,
    releases: Vec<PolicyRelease>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct PolicyRelease {
    version: Version,
    priority: Priority,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum Priority {
    Normal,
    Critical,
}

impl Policy {
    fn validate(&self) -> Result<(), String> {
        if self.schema_version != 1 || self.releases.is_empty() || self.releases.len() > 1_000 {
            return Err("Invalid update history".into());
        }
        let mut previous = None;
        for release in &self.releases {
            let version = &release.version;
            if !version.pre.is_empty()
                || !version.build.is_empty()
                || previous.is_some_and(|previous| version <= previous)
            {
                return Err(
                    "Update history must contain unique stable versions in ascending order".into(),
                );
            }
            previous = Some(version);
        }
        Ok(())
    }

    fn notice(&self, current: &Version) -> Option<UpdateNotice> {
        let newer: Vec<_> = self
            .releases
            .iter()
            .filter(|release| &release.version > current)
            .collect();
        let latest = newer.last()?;
        let requirement = if newer
            .iter()
            .any(|release| release.priority == Priority::Critical)
        {
            Some(UpdateRequirement::Critical)
        } else if newer.len() >= 3 {
            Some(UpdateRequirement::Outdated)
        } else {
            None
        };
        Some(UpdateNotice {
            version: latest.version.to_string(),
            requirement,
        })
    }
}

pub(crate) struct CheckResult {
    pub available: Option<UpdateNotice>,
    pub archive: Option<Vec<u8>>,
}

fn decode(bytes: &[u8], key: &[u8; 32]) -> Result<Policy, String> {
    if bytes.len() as u64 > MAX_ARCHIVE {
        return Err("Update history is too large".into());
    }
    let keys =
        zipsign_api::verify::collect_keys([Ok(*key)]).map_err(|_| "Invalid publisher key")?;
    zipsign_api::verify::verify_zip(&mut Cursor::new(bytes), &keys, Some(FEED_NAME.as_bytes()))
        .map_err(|_| "Update history failed signature verification")?;
    let mut archive =
        zip::ZipArchive::new(Cursor::new(bytes)).map_err(|_| "Invalid update history archive")?;
    if archive.len() != 1 {
        return Err("Invalid update history archive".into());
    }
    let file = archive
        .by_name("updates.json")
        .map_err(|_| "Update history is missing")?;
    let mut json = Vec::new();
    file.take(MAX_POLICY + 1)
        .read_to_end(&mut json)
        .map_err(|_| "Could not read update history")?;
    if json.len() as u64 > MAX_POLICY {
        return Err("Update history is too large".into());
    }
    let policy: Policy = serde_json::from_slice(&json).map_err(|_| "Invalid update history")?;
    policy.validate()?;
    Ok(policy)
}

fn cached_policy() -> Result<Option<Policy>, String> {
    let path = crate::settings::directory()?.join(FEED_NAME);
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("Could not read saved update history".into()),
    };
    let mut bytes = Vec::new();
    file.take(MAX_ARCHIVE + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Could not read saved update history")?;
    decode(&bytes, &PUBLISHER_KEY).map(Some)
}

pub(crate) fn cached() -> Result<Option<UpdateNotice>, String> {
    Ok(cached_policy()?
        .and_then(|policy| policy.notice(&Version::parse(env!("CARGO_PKG_VERSION")).unwrap())))
}

pub(crate) fn check() -> Result<CheckResult, String> {
    let client = reqwest::blocking::Client::builder()
        .https_only(true)
        .timeout(std::time::Duration::from_secs(20))
        .connect_timeout(std::time::Duration::from_secs(5))
        .user_agent(concat!("nolae-companion/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|_| "Could not initialize update checks")?;
    let response = client
        .get(FEED_URL)
        .send()
        .map_err(|_| "Could not check for updates. Try again later.")?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Err("No public update has been published yet.".into());
    }
    let response = response
        .error_for_status()
        .map_err(|_| "Could not check for updates. Try again later.")?;
    let mut bytes = Vec::new();
    response
        .take(MAX_ARCHIVE + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Could not read update history")?;
    let policy = decode(&bytes, &PUBLISHER_KEY)?;
    if let Some(previous) = cached_policy().unwrap_or(None)
        && !policy.releases.starts_with(&previous.releases)
    {
        return Err("Update history moved backwards. Keeping the previous update notice.".into());
    }
    Ok(CheckResult {
        available: policy.notice(&Version::parse(env!("CARGO_PKG_VERSION")).unwrap()),
        archive: Some(bytes),
    })
}

impl CheckResult {
    pub fn cache(&self) -> Result<(), String> {
        let Some(bytes) = &self.archive else {
            return Ok(());
        };
        let directory = crate::settings::directory()?;
        std::fs::create_dir_all(&directory).map_err(|_| "Could not save update history")?;
        let temporary = directory.join("updates.tmp");
        std::fs::write(&temporary, bytes).map_err(|_| "Could not save update history")?;
        std::fs::rename(temporary, directory.join(FEED_NAME))
            .map_err(|_| "Could not save update history".into())
    }
}

const TARGET: &str = "x86_64-pc-windows-msvc";
const CORE: &str = "nolae-companion.exe";
const STAGED_CORE: &str = "nolae-companion.exe.new";
const PREVIOUS_CORE: &str = "nolae-companion.previous.exe";
const WINDOW: &str = "nolae-companion-ui.exe";
const LICENSES: [&str; 3] = ["LICENSE", "OFL-Manrope.txt", "THIRD_PARTY_NOTICES.html"];
const MAX_ENTRIES: usize = 32;
const MAX_COMPONENT: u64 = 134_217_728;
const MAX_LICENSE: u64 = 4_194_304;

pub(crate) fn install(version: &str, installed: &str) -> Result<bool, String> {
    let directory = std::env::current_exe()
        .map_err(|_| "Could not locate Companion")?
        .parent()
        .ok_or("Could not locate Companion")?
        .to_path_buf();
    let target_version = version.to_owned();
    let installed_version = installed.to_owned();
    let destination = directory.join(STAGED_CORE);
    let staging_directory = directory.clone();
    let status = self_update::backends::github::Update::configure()
        .repo_owner("nolaedotapp")
        .repo_name("companion")
        .release_tag(format!("v{version}"))
        .bin_name(CORE)
        .bin_install_path(destination)
        .current_version(installed)
        .target(TARGET)
        .checksum_from_asset("SHA256SUMS")
        .timeout(std::time::Duration::from_secs(90))
        .show_output(false)
        .show_download_progress(false)
        .no_confirm(true)
        .verifying_keys(vec![PUBLISHER_KEY])
        .verify_archive(move |archive: &Path| {
            stage_update(
                archive,
                &staging_directory,
                &target_version,
                &installed_version,
                &PUBLISHER_KEY,
            )
            .map_err(self_update::Error::archive_verification_rejected)
        })
        .build()
        .map_err(|_| "Could not initialize signed updates")?
        .update()
        .map_err(|_| "Update failed verification or could not be installed. Try again.")?;
    if status.is_updated() {
        replace_core(&directory, version)?;
    }
    Ok(status.is_updated())
}

#[cfg(windows)]
fn replace_core(directory: &Path, version: &str) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;

    let current = directory.join(CORE);
    let staged = directory.join(STAGED_CORE);
    let backup = directory.join("ui").join(version).join(PREVIOUS_CORE);
    let wide = |path: &Path| {
        path.as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>()
    };
    let (current_wide, staged_wide, backup_wide) = (wide(&current), wide(&staged), wide(&backup));
    std::fs::OpenOptions::new()
        .write(true)
        .open(&staged)
        .and_then(|file| file.sync_all())
        .map_err(|_| "Could not save the update. Try again.")?;
    let replaced = unsafe {
        windows_sys::Win32::Storage::FileSystem::ReplaceFileW(
            current_wide.as_ptr(),
            staged_wide.as_ptr(),
            backup_wide.as_ptr(),
            0,
            std::ptr::null(),
            std::ptr::null(),
        )
    };
    if replaced == 0 {
        if !current.exists() && backup.exists() {
            std::fs::rename(&backup, &current)
                .map_err(|_| "Could not restore Companion. Reinstall the latest version.")?;
        }
        return Err("Could not replace Companion. Close other copies and try again.".into());
    }
    Ok(())
}

#[cfg(not(windows))]
fn replace_core(_: &Path, _: &str) -> Result<(), String> {
    Err("Updates are available on Windows only.".into())
}

fn archive_name(version: &str) -> String {
    format!("nolae-companion-v{version}-{TARGET}.zip")
}

fn stable_version(version: &str) -> Result<Version, String> {
    let parsed = Version::parse(version).map_err(|_| "Invalid update version")?;
    if !parsed.pre.is_empty() || !parsed.build.is_empty() || parsed.to_string() != version {
        return Err("Invalid update version".into());
    }
    Ok(parsed)
}

fn stage_update(
    archive: &Path,
    directory: &Path,
    version: &str,
    installed: &str,
    key: &[u8; 32],
) -> Result<(), String> {
    let target = stable_version(version)?;
    let current =
        Version::parse(env!("CARGO_PKG_VERSION")).map_err(|_| "Invalid update version")?;
    let installed = Version::parse(installed).unwrap_or_else(|_| current.clone());
    if target <= current || target <= installed {
        return Err("The update is not newer than this version".into());
    }
    if archive.file_name().and_then(|name| name.to_str()) != Some(archive_name(version).as_str()) {
        return Err("The update archive does not match its release".into());
    }
    self_update::verify_signature(archive, &[*key])
        .map_err(|_| "The update failed signature verification")?;
    let file = std::fs::File::open(archive).map_err(|_| "Could not read the update")?;
    let mut archive = zip::ZipArchive::new(file).map_err(|_| "Invalid update archive")?;
    validate_entries(&mut archive)?;
    extract(
        &mut archive,
        WINDOW,
        &directory.join("ui").join(version),
        MAX_COMPONENT,
    )?;
    let licenses = directory.join("licenses");
    for name in LICENSES {
        extract(
            &mut archive,
            &format!("licenses/{name}"),
            &licenses,
            MAX_LICENSE,
        )?;
    }
    for name in LICENSES {
        let legacy = directory.join(name);
        if std::fs::symlink_metadata(&legacy).is_ok_and(|metadata| metadata.is_file()) {
            let _ = std::fs::remove_file(legacy);
        }
    }
    Ok(())
}

fn validate_entries<R: Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
) -> Result<(), String> {
    if archive.len() > MAX_ENTRIES || archive.has_duplicate_names() {
        return Err("Invalid update archive".into());
    }
    let mut names = std::collections::HashSet::new();
    let mut total = 0u64;
    for index in 0..archive.len() {
        let entry = archive
            .by_index_raw(index)
            .map_err(|_| "Invalid update archive")?;
        let name = entry
            .name()
            .map_err(|_| "Invalid update archive")?
            .into_owned();
        let safe = !name.is_empty()
            && !name.starts_with('/')
            && !name.contains(['\\', ':', '\0'])
            && name
                .trim_end_matches('/')
                .split('/')
                .all(|part| !part.is_empty() && part != "." && part != "..")
            && entry.enclosed_name().is_some();
        if !safe || entry.is_symlink() || !names.insert(name.to_ascii_lowercase()) {
            return Err("Invalid update archive".into());
        }
        let limit = if name.starts_with("licenses/") {
            MAX_LICENSE
        } else {
            MAX_COMPONENT
        };
        if entry.size() > limit {
            return Err("The update is too large".into());
        }
        total = total.saturating_add(entry.size());
    }
    if total > 2 * MAX_COMPONENT + 3 * MAX_LICENSE {
        return Err("The update is too large".into());
    }
    let required = [CORE.to_owned(), WINDOW.to_owned()]
        .into_iter()
        .chain(LICENSES.map(|name| format!("licenses/{name}")));
    for name in required {
        let entry = archive
            .by_name(&name)
            .map_err(|_| "The update is missing a required file")?;
        if !entry.is_file() || entry.size() == 0 {
            return Err("The update is missing a required file".into());
        }
    }
    Ok(())
}

fn extract<R: Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    name: &str,
    destination: &Path,
    limit: u64,
) -> Result<(), String> {
    let file_name = name.rsplit('/').next().ok_or("Invalid update archive")?;
    let component = archive
        .by_name(name)
        .map_err(|_| "The update is missing a required file")?;
    if component.size() > limit {
        return Err("The update is too large".into());
    }
    std::fs::create_dir_all(destination).map_err(|_| "Could not stage the update")?;
    let mut temporary =
        tempfile::NamedTempFile::new_in(destination).map_err(|_| "Could not stage the update")?;
    let size = std::io::copy(&mut component.take(limit + 1), &mut temporary)
        .map_err(|_| "Could not extract the update")?;
    if size > limit || size == 0 {
        return Err("The update is too large".into());
    }
    temporary
        .as_file()
        .sync_all()
        .map_err(|_| "Could not save the update")?;
    temporary
        .persist(destination.join(file_name))
        .map_err(|_| "Could not stage the update")?;
    Ok(())
}

#[cfg(test)]
#[path = "update_tests.rs"]
mod tests;
