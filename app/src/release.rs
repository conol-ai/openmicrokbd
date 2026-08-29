//! GitHub Release discovery and integrity-checked artifact downloads.
//!
//! Release CI publishes `release-manifest.json` beside the desktop packages and firmware
//! image. The app checks GitHub's stable `releases/latest/download` URL on a
//! worker thread, selects the package for the running OS/architecture, and verifies
//! every downloaded artifact against the SHA-256 recorded in that manifest.

use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::events;

pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const MANIFEST_URL: &str = match option_env!("OPENMICRO_UPDATE_MANIFEST_URL") {
    Some(url) => url,
    None => {
        "https://github.com/conol-ai/openmicrokbd/releases/latest/download/release-manifest.json"
    }
};

const MANIFEST_LIMIT: u64 = 128 * 1024;
const APP_DOWNLOAD_LIMIT: u64 = 512 * 1024 * 1024;
const FIRMWARE_DOWNLOAD_MIN: u64 = 192;
const FIRMWARE_DOWNLOAD_LIMIT: u64 = 126 * 1024;

#[derive(Deserialize)]
struct BundledFirmwareManifest {
    version: String,
    sha256: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ReleaseCatalog {
    pub schema: u32,
    pub product: String,
    pub release_url: String,
    pub app: AppRelease,
    pub firmware: FirmwareRelease,
}

#[derive(Clone, Debug, Deserialize)]
pub struct AppRelease {
    pub version: String,
    pub macos: MacOsRelease,
    #[serde(default)]
    pub windows: Option<WindowsRelease>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct MacOsRelease {
    pub aarch64: ReleaseAsset,
    pub x86_64: ReleaseAsset,
}

#[derive(Clone, Debug, Deserialize)]
pub struct WindowsRelease {
    pub aarch64: ReleaseAsset,
    pub x86_64: ReleaseAsset,
}

#[derive(Clone, Debug, Deserialize)]
pub struct FirmwareRelease {
    pub version: String,
    pub board: String,
    pub protocol: u32,
    #[serde(flatten)]
    pub asset: ReleaseAsset,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ReleaseAsset {
    pub name: String,
    pub url: String,
    pub sha256: String,
    pub size: u64,
}

impl ReleaseCatalog {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema != 1 {
            return Err(format!(
                "unsupported release manifest schema {}",
                self.schema
            ));
        }
        if self.product != "openmicrokbd" {
            return Err(format!(
                "release manifest is for product {:?}",
                self.product
            ));
        }
        if self.firmware.board != "openmicro-stm32f072cb" {
            return Err(format!(
                "release firmware is for board {:?}",
                self.firmware.board
            ));
        }
        if self.firmware.protocol != 2 {
            return Err(format!(
                "unsupported firmware protocol {}",
                self.firmware.protocol
            ));
        }
        validate_version(&self.app.version)?;
        validate_version(&self.firmware.version)?;
        validate_https_url(&self.release_url)?;
        let mut assets = vec![
            &self.app.macos.aarch64,
            &self.app.macos.x86_64,
            &self.firmware.asset,
        ];
        if let Some(windows) = &self.app.windows {
            assets.extend([&windows.aarch64, &windows.x86_64]);
        }
        for asset in assets {
            asset.validate()?;
        }
        if !(FIRMWARE_DOWNLOAD_MIN..=FIRMWARE_DOWNLOAD_LIMIT).contains(&self.firmware.asset.size) {
            return Err(format!(
                "firmware asset is {} bytes; expected {}..={} bytes",
                self.firmware.asset.size, FIRMWARE_DOWNLOAD_MIN, FIRMWARE_DOWNLOAD_LIMIT
            ));
        }
        Ok(())
    }

    pub fn app_asset(&self) -> Option<&ReleaseAsset> {
        match (std::env::consts::OS, std::env::consts::ARCH) {
            ("macos", "aarch64") => Some(&self.app.macos.aarch64),
            ("macos", "x86_64") => Some(&self.app.macos.x86_64),
            ("windows", "aarch64") => self.app.windows.as_ref().map(|set| &set.aarch64),
            ("windows", "x86_64") => self.app.windows.as_ref().map(|set| &set.x86_64),
            _ => None,
        }
    }
}

impl ReleaseAsset {
    fn validate(&self) -> Result<(), String> {
        validate_https_url(&self.url)?;
        if self.name.is_empty()
            || self.name.len() > 128
            || self.name == "."
            || self.name == ".."
            || !self
                .name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        {
            return Err(format!("unsafe release asset name {:?}", self.name));
        }
        if self.size == 0 {
            return Err(format!("release asset {} has no size", self.name));
        }
        if self.sha256.len() != 64
            || !self
                .sha256
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        {
            return Err(format!(
                "release asset {} has an invalid SHA-256",
                self.name
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DownloadKind {
    App,
    Firmware,
}

#[derive(Clone, Debug)]
pub enum ReleaseMsg {
    Catalog(ReleaseCatalog),
    CatalogUnavailable(String),
    DownloadProgress {
        kind: DownloadKind,
        version: String,
        fraction: f64,
    },
    DownloadReady {
        kind: DownloadKind,
        version: String,
        path: PathBuf,
    },
    DownloadFailed {
        kind: DownloadKind,
        version: String,
        error: String,
    },
}

/// Check for the newest stable release without delaying UI startup.
pub fn spawn_catalog_check() {
    std::thread::spawn(|| match fetch_catalog(MANIFEST_URL) {
        Ok(catalog) => events::post(ReleaseMsg::Catalog(catalog)),
        Err(error) => events::post(ReleaseMsg::CatalogUnavailable(error)),
    });
}

/// Download one manifest asset into the app cache and verify its exact size
/// and SHA-256 before making it available to the UI.
pub fn spawn_download(kind: DownloadKind, version: String, asset: ReleaseAsset) {
    std::thread::spawn(move || {
        let progress_version = version.clone();
        let result = download_asset(kind, &asset, |fraction| {
            events::post(ReleaseMsg::DownloadProgress {
                kind,
                version: progress_version.clone(),
                fraction,
            });
        });
        match result {
            Ok(path) => events::post(ReleaseMsg::DownloadReady {
                kind,
                version,
                path,
            }),
            Err(error) => events::post(ReleaseMsg::DownloadFailed {
                kind,
                version,
                error,
            }),
        }
    });
}

/// Return the firmware packaged inside a release app after verifying the
/// code-signature-covered file against its adjacent manifest. Development
/// builds simply return `Ok(None)`.
pub fn bundled_firmware() -> Result<Option<(String, PathBuf)>, String> {
    let executable =
        std::env::current_exe().map_err(|e| format!("cannot locate app executable: {e}"))?;
    #[cfg(target_os = "macos")]
    let Some(firmware_dir) = executable
        .parent()
        .and_then(Path::parent)
        .filter(|path| path.file_name().and_then(|name| name.to_str()) == Some("Contents"))
        .map(|contents| contents.join("Resources").join("firmware"))
    else {
        return Ok(None);
    };
    #[cfg(not(target_os = "macos"))]
    let Some(firmware_dir) = executable.parent().map(|directory| directory.join("firmware")) else {
        return Ok(None);
    };
    let image = firmware_dir.join("openmicro-fw.bin");
    let manifest_path = firmware_dir.join("manifest.json");
    if !image.is_file() || !manifest_path.is_file() {
        return Ok(None);
    }

    let manifest_bytes = fs::read(&manifest_path)
        .map_err(|e| format!("cannot read bundled firmware manifest: {e}"))?;
    if manifest_bytes.len() as u64 > MANIFEST_LIMIT {
        return Err("bundled firmware manifest is unexpectedly large".into());
    }
    let manifest: BundledFirmwareManifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|e| format!("invalid bundled firmware manifest: {e}"))?;
    validate_version(&manifest.version)?;
    if manifest.sha256.len() != 64
        || !manifest
            .sha256
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return Err("bundled firmware manifest has an invalid SHA-256".into());
    }
    let size = fs::metadata(&image)
        .map_err(|e| format!("cannot inspect bundled firmware: {e}"))?
        .len();
    if !(FIRMWARE_DOWNLOAD_MIN..=FIRMWARE_DOWNLOAD_LIMIT).contains(&size) {
        return Err(format!(
            "bundled firmware is {size} bytes; expected {FIRMWARE_DOWNLOAD_MIN}..={FIRMWARE_DOWNLOAD_LIMIT}"
        ));
    }
    verify_sha256(&image, &manifest.sha256)
        .map_err(|e| format!("bundled firmware integrity check failed: {e}"))?;
    Ok(Some((manifest.version, image)))
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout_read(Duration::from_secs(30))
        .redirects(8)
        .user_agent(&format!("OpenMicro/{APP_VERSION}"))
        .build()
}

fn fetch_catalog(url: &str) -> Result<ReleaseCatalog, String> {
    validate_https_url(url)?;
    let response = agent()
        .get(url)
        .call()
        .map_err(|e| format!("release check failed: {e}"))?;
    validate_https_url(response.get_url())?;
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(MANIFEST_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("cannot read release manifest: {e}"))?;
    if bytes.len() as u64 > MANIFEST_LIMIT {
        return Err("release manifest is unexpectedly large".into());
    }
    let catalog: ReleaseCatalog =
        serde_json::from_slice(&bytes).map_err(|e| format!("invalid release manifest: {e}"))?;
    catalog.validate()?;
    Ok(catalog)
}

fn download_asset(
    kind: DownloadKind,
    asset: &ReleaseAsset,
    mut progress: impl FnMut(f64),
) -> Result<PathBuf, String> {
    asset.validate()?;
    let hard_limit = match kind {
        DownloadKind::App => APP_DOWNLOAD_LIMIT,
        DownloadKind::Firmware => FIRMWARE_DOWNLOAD_LIMIT,
    };
    if asset.size > hard_limit {
        return Err(format!(
            "{} is {} bytes; download limit is {}",
            asset.name, asset.size, hard_limit
        ));
    }

    let cache = dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("openmicro")
        .join("updates");
    fs::create_dir_all(&cache).map_err(|e| format!("cannot create update cache: {e}"))?;
    let destination = cache.join(&asset.name);
    if destination.is_file() && verify_file(&destination, asset).is_ok() {
        progress(1.0);
        return Ok(destination);
    }

    let partial = cache.join(format!("{}.part", asset.name));
    let _ = fs::remove_file(&partial);
    let response = agent()
        .get(&asset.url)
        .call()
        .map_err(|e| format!("download {} failed: {e}", asset.name))?;
    validate_https_url(response.get_url())?;
    let mut reader = response.into_reader();
    let mut output = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&partial)
        .map_err(|e| format!("cannot create {}: {e}", partial.display()))?;
    let mut hasher = Sha256::new();
    let mut total = 0u64;
    let mut buffer = [0u8; 64 * 1024];

    let result = (|| -> Result<(), String> {
        loop {
            let read = reader
                .read(&mut buffer)
                .map_err(|e| format!("download {} interrupted: {e}", asset.name))?;
            if read == 0 {
                break;
            }
            total += read as u64;
            if total > asset.size || total > hard_limit {
                return Err(format!(
                    "download {} exceeded its declared size",
                    asset.name
                ));
            }
            output
                .write_all(&buffer[..read])
                .map_err(|e| format!("cannot write {}: {e}", partial.display()))?;
            hasher.update(&buffer[..read]);
            progress(total as f64 / asset.size as f64);
        }
        output
            .sync_all()
            .map_err(|e| format!("cannot finish {}: {e}", partial.display()))?;
        if total != asset.size {
            return Err(format!(
                "download {} is {} bytes; expected {}",
                asset.name, total, asset.size
            ));
        }
        let actual = format!("{:x}", hasher.finalize());
        if actual != asset.sha256 {
            return Err(format!(
                "download {} failed integrity verification",
                asset.name
            ));
        }
        Ok(())
    })();

    if let Err(error) = result {
        drop(output);
        let _ = fs::remove_file(&partial);
        return Err(error);
    }
    drop(output);
    let _ = fs::remove_file(&destination);
    fs::rename(&partial, &destination)
        .map_err(|e| format!("cannot store downloaded update: {e}"))?;
    progress(1.0);
    Ok(destination)
}

fn verify_file(path: &Path, asset: &ReleaseAsset) -> Result<(), String> {
    let mut file = File::open(path).map_err(|e| e.to_string())?;
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    if metadata.len() != asset.size {
        return Err("cached file size does not match".into());
    }
    verify_sha256_reader(&mut file, &asset.sha256)
}

fn verify_sha256(path: &Path, expected: &str) -> Result<(), String> {
    let mut file = File::open(path).map_err(|e| e.to_string())?;
    verify_sha256_reader(&mut file, expected)
}

fn verify_sha256_reader(reader: &mut impl Read, expected: &str) -> Result<(), String> {
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = reader.read(&mut buffer).map_err(|e| e.to_string())?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let actual = format!("{:x}", hasher.finalize());
    if actual != expected {
        return Err("file hash does not match".into());
    }
    Ok(())
}

fn validate_https_url(url: &str) -> Result<(), String> {
    if !url.starts_with("https://") || url.bytes().any(|b| b.is_ascii_whitespace()) {
        return Err(format!("release URL must use HTTPS: {url:?}"));
    }
    Ok(())
}

fn validate_version(version: &str) -> Result<(), String> {
    if version.is_empty() || version.len() > 64 {
        return Err(format!("invalid release version {version:?}"));
    }
    let version = version.strip_prefix('v').unwrap_or(version);
    let (core, prerelease) = match version.split_once('-') {
        Some((core, suffix)) => (core, Some(suffix)),
        None => (version, None),
    };
    let parts: Vec<_> = core.split('.').collect();
    if parts.len() != 3
        || parts
            .iter()
            .any(|part| part.is_empty() || part.parse::<u64>().is_err())
        || prerelease.is_some_and(|suffix| {
            suffix.is_empty()
                || !suffix
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
        })
    {
        return Err(format!("invalid release version {version:?}"));
    }
    Ok(())
}

/// True when `candidate` is a newer semver-like version than `current`.
/// Release versions are intentionally limited to numeric major.minor.patch;
/// a prerelease suffix sorts below the corresponding stable release.
pub fn is_newer(candidate: &str, current: &str) -> bool {
    fn parse(version: &str) -> ([u64; 3], bool) {
        let version = version.strip_prefix('v').unwrap_or(version);
        let (core, prerelease) = match version.split_once('-') {
            Some((core, _)) => (core, true),
            None => (version, false),
        };
        let mut out = [0u64; 3];
        for (index, part) in core.split('.').take(3).enumerate() {
            out[index] = part.parse().unwrap_or(0);
        }
        (out, prerelease)
    }

    let (candidate_core, candidate_pre) = parse(candidate);
    let (current_core, current_pre) = parse(current);
    candidate_core > current_core
        || (candidate_core == current_core && current_pre && !candidate_pre)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_comparison_handles_stable_and_prerelease() {
        assert!(is_newer("0.3.0", "0.2.9"));
        assert!(is_newer("v1.0.0", "1.0.0-beta.1"));
        assert!(!is_newer("1.0.0-beta.1", "1.0.0"));
        assert!(!is_newer("0.2.0", "0.2.0"));
    }

    #[test]
    fn manifest_rejects_path_traversal_asset_names() {
        let asset = ReleaseAsset {
            name: "../OpenMicro.dmg".into(),
            url: "https://example.com/OpenMicro.dmg".into(),
            sha256: "0".repeat(64),
            size: 1,
        };
        assert!(asset.validate().is_err());
    }

    #[test]
    fn release_versions_require_three_numeric_components() {
        assert!(validate_version("0.2.1").is_ok());
        assert!(validate_version("v12.3.4-beta.1").is_ok());
        for invalid in [
            "0.2",
            "0.2.x",
            "1.2.3.4",
            "1.2.3-",
            "18446744073709551616.0.0",
            "",
            "latest",
        ] {
            assert!(
                validate_version(invalid).is_err(),
                "{invalid:?} should be rejected"
            );
        }
    }

    #[test]
    fn published_manifest_shape_deserializes_and_validates() {
        let manifest = serde_json::json!({
            "schema": 1,
            "product": "openmicrokbd",
            "release_url": "https://github.com/conol-ai/openmicrokbd/releases/tag/v0.3.0",
            "app": {
                "version": "0.3.0",
                "macos": {
                    "aarch64": {
                        "name": "OpenMicro-0.3.0-macos-aarch64.dmg",
                        "url": "https://github.com/conol-ai/openmicrokbd/releases/download/v0.3.0/OpenMicro-0.3.0-macos-aarch64.dmg",
                        "sha256": "a".repeat(64),
                        "size": 40_000_000
                    },
                    "x86_64": {
                        "name": "OpenMicro-0.3.0-macos-x86_64.dmg",
                        "url": "https://github.com/conol-ai/openmicrokbd/releases/download/v0.3.0/OpenMicro-0.3.0-macos-x86_64.dmg",
                        "sha256": "b".repeat(64),
                        "size": 40_000_000
                    }
                },
                "windows": {
                    "aarch64": {
                        "name": "OpenMicro-0.3.0-windows-aarch64.zip",
                        "url": "https://github.com/conol-ai/openmicrokbd/releases/download/v0.3.0/OpenMicro-0.3.0-windows-aarch64.zip",
                        "sha256": "d".repeat(64),
                        "size": 30_000_000
                    },
                    "x86_64": {
                        "name": "OpenMicro-0.3.0-windows-x86_64.zip",
                        "url": "https://github.com/conol-ai/openmicrokbd/releases/download/v0.3.0/OpenMicro-0.3.0-windows-x86_64.zip",
                        "sha256": "e".repeat(64),
                        "size": 30_000_000
                    }
                }
            },
            "firmware": {
                "version": "0.3.1",
                "board": "openmicro-stm32f072cb",
                "protocol": 2,
                "name": "openmicro-fw-0.3.1.bin",
                "url": "https://github.com/conol-ai/openmicrokbd/releases/download/v0.3.0/openmicro-fw-0.3.1.bin",
                "sha256": "c".repeat(64),
                "size": 28_000
            }
        });
        let catalog: ReleaseCatalog = serde_json::from_value(manifest).unwrap();
        catalog.validate().unwrap();
        assert!(catalog.app.windows.is_some());
        if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
            assert_eq!(
                catalog.app_asset().expect("Windows asset").name,
                "OpenMicro-0.3.0-windows-x86_64.zip"
            );
        }
    }
}
