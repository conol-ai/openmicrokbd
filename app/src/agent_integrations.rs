//! Safe, user-triggered installation of coding-agent activity integrations.
//!
//! Agent configuration is external state, not part of [`crate::config::AppConfig`].
//! The Settings UI derives status from the target files each time it opens and
//! installs only OpenMicro-owned hook entries or plugin scripts. Existing
//! configuration is parsed structurally, backed up, and replaced atomically.

use std::collections::HashSet;
use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

const DEFAULT_APP_BINARY: &str = "/Applications/OpenMicro.app/Contents/MacOS/OpenMicro";
const MANAGED_MARKER: &str = "// Managed by OpenMicro Settings. Remove this line before editing; marked files may be replaced on update.";
const SHELL_MANAGED_MARKER: &str = "# Managed by OpenMicro Settings. Remove this line before editing; marked files may be replaced on update.";
const LEGACY_MANAGED_MARKER: &str =
    "// Managed by OpenMicro Settings. Local changes may be replaced on update.";
const LEGACY_SHELL_MANAGED_MARKER: &str =
    "# Managed by OpenMicro Settings. Local changes may be replaced on update.";
const MAX_AGENT_CONFIG_BYTES: u64 = 4 * 1024 * 1024;
const CODEX_TEMPLATE: &str = include_str!("../codex-hooks.example.json");
const CLAUDE_TEMPLATE: &str = include_str!("../claude-code-hooks.example.json");
const OPENCODE_TEMPLATE: &str = include_str!("../opencode-openmicro.example.ts");
const DEEP_CODE_TEMPLATE: &str = include_str!("../deep-code-notify.example.sh");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntegrationKind {
    Codex,
    ClaudeCode,
    OpenCode,
    DeepCode,
}

impl IntegrationKind {
    pub const ALL: [Self; 4] = [
        Self::Codex,
        Self::ClaudeCode,
        Self::OpenCode,
        Self::DeepCode,
    ];

    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Codex => "Codex",
            Self::ClaudeCode => "Claude Code",
            Self::OpenCode => "OpenCode",
            Self::DeepCode => "Deep Code",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstallState {
    NotInstalled,
    Installed,
    NeedsUpdate,
    Conflict,
    Unavailable,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IntegrationReport {
    pub kind: IntegrationKind,
    pub state: InstallState,
    pub target: PathBuf,
    pub detail: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstallDisposition {
    AlreadyInstalled,
    Installed,
    Updated,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstallReceipt {
    pub disposition: InstallDisposition,
    pub changed_files: Vec<PathBuf>,
    pub backups: Vec<PathBuf>,
}

#[derive(Clone, Debug)]
struct InstallLayout {
    executable: PathBuf,
    codex_hooks: PathBuf,
    claude_settings: PathBuf,
    opencode_plugin: PathBuf,
    deep_code_settings: PathBuf,
    deep_code_script: PathBuf,
}

impl InstallLayout {
    fn discover() -> Result<Self, String> {
        if !cfg!(unix) {
            return Err(
                "coding-agent activity installation is currently available on macOS and Linux only"
                    .into(),
            );
        }

        let home =
            dirs::home_dir().ok_or_else(|| "cannot locate the user home directory".to_string())?;
        let executable = env::current_exe()
            .map_err(|error| format!("cannot locate the OpenMicro executable: {error}"))?;
        validate_executable(&executable)?;

        let executable_text = path_text(&executable)?;
        if cfg!(target_os = "macos") && executable_text.contains("/AppTranslocation/") {
            return Err(
                "OpenMicro is running from macOS App Translocation; move it to Applications and reopen it before installing hooks"
                    .into(),
            );
        }

        let codex_home = nonempty_env_path("CODEX_HOME")?.unwrap_or_else(|| home.join(".codex"));
        let claude_home =
            nonempty_env_path("CLAUDE_CONFIG_DIR")?.unwrap_or_else(|| home.join(".claude"));
        let xdg_config =
            nonempty_env_path("XDG_CONFIG_HOME")?.unwrap_or_else(|| home.join(".config"));

        Ok(Self {
            executable,
            codex_hooks: codex_home.join("hooks.json"),
            claude_settings: claude_home.join("settings.json"),
            opencode_plugin: xdg_config.join("opencode/plugins/openmicro.ts"),
            deep_code_settings: home.join(".deepcode/settings.json"),
            deep_code_script: home.join(".deepcode/openmicro-notify.sh"),
        })
    }

    fn target(&self, kind: IntegrationKind) -> &Path {
        match kind {
            IntegrationKind::Codex => &self.codex_hooks,
            IntegrationKind::ClaudeCode => &self.claude_settings,
            IntegrationKind::OpenCode => &self.opencode_plugin,
            IntegrationKind::DeepCode => &self.deep_code_settings,
        }
    }

    #[cfg(test)]
    fn test(home: &Path, executable: &Path) -> Self {
        Self {
            executable: executable.to_path_buf(),
            codex_hooks: home.join(".codex/hooks.json"),
            claude_settings: home.join(".claude/settings.json"),
            opencode_plugin: home.join(".config/opencode/plugins/openmicro.ts"),
            deep_code_settings: home.join(".deepcode/settings.json"),
            deep_code_script: home.join(".deepcode/openmicro-notify.sh"),
        }
    }
}

fn nonempty_env_path(name: &str) -> Result<Option<PathBuf>, String> {
    let path = env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    if path.as_ref().is_some_and(|path| !path.is_absolute()) {
        return Err(format!("{name} must be an absolute path"));
    }
    Ok(path)
}

fn validate_executable(path: &Path) -> Result<(), String> {
    let metadata = fs::metadata(path)
        .map_err(|error| format!("cannot inspect {}: {error}", path.display()))?;
    if !metadata.is_file() {
        return Err(format!(
            "{} is not a regular executable file",
            path.display()
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            return Err(format!("{} is not executable", path.display()));
        }
    }
    let _ = path_text(path)?;
    Ok(())
}

fn path_text(path: &Path) -> Result<&str, String> {
    path.to_str()
        .ok_or_else(|| format!("{} is not a UTF-8 path", path.display()))
}

pub fn scan_system_all() -> Vec<IntegrationReport> {
    match InstallLayout::discover() {
        Ok(layout) => IntegrationKind::ALL
            .into_iter()
            .map(|kind| inspect(&layout, kind))
            .collect(),
        Err(error) => IntegrationKind::ALL
            .into_iter()
            .map(|kind| IntegrationReport {
                kind,
                state: InstallState::Unavailable,
                target: PathBuf::new(),
                detail: Some(error.clone()),
            })
            .collect(),
    }
}

pub fn install_system(kind: IntegrationKind) -> Result<InstallReceipt, String> {
    let layout = InstallLayout::discover()?;
    install(&layout, kind)
}

fn inspect(layout: &InstallLayout, kind: IntegrationKind) -> IntegrationReport {
    let result = match kind {
        IntegrationKind::Codex | IntegrationKind::ClaudeCode => inspect_json_hooks(layout, kind),
        IntegrationKind::OpenCode => inspect_owned_template(
            &layout.opencode_plugin,
            &render_opencode_plugin(&layout.executable),
            "const OPENMICRO = ",
        ),
        IntegrationKind::DeepCode => inspect_deep_code(layout),
    };

    match result {
        Ok(state) => IntegrationReport {
            kind,
            state,
            target: layout.target(kind).to_path_buf(),
            detail: None,
        },
        Err(detail) => IntegrationReport {
            kind,
            state: InstallState::Conflict,
            target: layout.target(kind).to_path_buf(),
            detail: Some(detail),
        },
    }
}

fn install(layout: &InstallLayout, kind: IntegrationKind) -> Result<InstallReceipt, String> {
    let report = inspect(layout, kind);
    match report.state {
        InstallState::Installed => {
            return Ok(InstallReceipt {
                disposition: InstallDisposition::AlreadyInstalled,
                changed_files: Vec::new(),
                backups: Vec::new(),
            });
        }
        InstallState::Conflict | InstallState::Unavailable => {
            return Err(report
                .detail
                .unwrap_or_else(|| "the integration target needs manual review".into()));
        }
        InstallState::NotInstalled | InstallState::NeedsUpdate => {}
    }

    match kind {
        IntegrationKind::Codex | IntegrationKind::ClaudeCode => {
            install_json_hooks(layout, kind, report.state)
        }
        IntegrationKind::OpenCode => install_opencode(layout, report.state),
        IntegrationKind::DeepCode => install_deep_code(layout, report.state),
    }
}

#[derive(Clone)]
struct FileSnapshot {
    logical_path: PathBuf,
    resolved_path: PathBuf,
    bytes: Option<Vec<u8>>,
    digest: Option<[u8; 32]>,
    mode: Option<u32>,
}

impl FileSnapshot {
    fn read(logical_path: &Path) -> Result<Self, String> {
        let symlink_metadata = match fs::symlink_metadata(logical_path) {
            Ok(metadata) => Some(metadata),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => {
                return Err(format!(
                    "cannot inspect {}: {error}",
                    logical_path.display()
                ))
            }
        };

        let resolved_path = if symlink_metadata
            .as_ref()
            .is_some_and(|metadata| metadata.file_type().is_symlink())
        {
            fs::canonicalize(logical_path).map_err(|error| {
                format!(
                    "cannot resolve settings symlink {}: {error}",
                    logical_path.display()
                )
            })?
        } else {
            logical_path.to_path_buf()
        };

        if symlink_metadata.is_none() {
            return Ok(Self {
                logical_path: logical_path.to_path_buf(),
                resolved_path,
                bytes: None,
                digest: None,
                mode: None,
            });
        }

        let metadata = fs::metadata(&resolved_path)
            .map_err(|error| format!("cannot inspect {}: {error}", resolved_path.display()))?;
        if !metadata.is_file() {
            return Err(format!("{} is not a regular file", resolved_path.display()));
        }
        if metadata.len() > MAX_AGENT_CONFIG_BYTES {
            return Err(format!(
                "{} is larger than the {} MiB installer limit",
                logical_path.display(),
                MAX_AGENT_CONFIG_BYTES / (1024 * 1024)
            ));
        }

        let mut bytes = Vec::with_capacity(metadata.len() as usize);
        File::open(&resolved_path)
            .map_err(|error| format!("cannot open {}: {error}", resolved_path.display()))?
            .take(MAX_AGENT_CONFIG_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| format!("cannot read {}: {error}", resolved_path.display()))?;
        if bytes.len() as u64 > MAX_AGENT_CONFIG_BYTES {
            return Err(format!(
                "{} changed beyond the installer size limit",
                logical_path.display()
            ));
        }
        let digest = Some(Sha256::digest(&bytes).into());

        Ok(Self {
            logical_path: logical_path.to_path_buf(),
            resolved_path,
            bytes: Some(bytes),
            digest,
            mode: file_mode(&metadata),
        })
    }

    fn still_matches(&self) -> Result<bool, String> {
        let current = Self::read(&self.logical_path)?;
        Ok(current.resolved_path == self.resolved_path
            && current.digest == self.digest
            && current.mode == self.mode)
    }

    fn matches_file(&self, path: &Path) -> Result<bool, String> {
        let metadata = fs::metadata(path)
            .map_err(|error| format!("cannot inspect {}: {error}", path.display()))?;
        if !metadata.is_file() || metadata.len() > MAX_AGENT_CONFIG_BYTES {
            return Ok(false);
        }
        let mut bytes = Vec::with_capacity(metadata.len() as usize);
        File::open(path)
            .map_err(|error| format!("cannot open {}: {error}", path.display()))?
            .take(MAX_AGENT_CONFIG_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
        if bytes.len() as u64 > MAX_AGENT_CONFIG_BYTES {
            return Ok(false);
        }
        Ok(Some(Sha256::digest(&bytes).into()) == self.digest && file_mode(&metadata) == self.mode)
    }
}

#[cfg(unix)]
fn file_mode(metadata: &fs::Metadata) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    Some(metadata.permissions().mode() & 0o777)
}

#[cfg(not(unix))]
fn file_mode(_metadata: &fs::Metadata) -> Option<u32> {
    None
}

#[cfg(unix)]
fn set_file_mode(path: &Path, mode: u32) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
        .map_err(|error| format!("cannot set permissions on {}: {error}", path.display()))
}

#[cfg(not(unix))]
fn set_file_mode(_path: &Path, _mode: u32) -> Result<(), String> {
    Ok(())
}

struct PlannedWrite {
    snapshot: FileSnapshot,
    contents: Vec<u8>,
    default_mode: u32,
    required_mode: Option<u32>,
}

struct StagedWrite {
    planned: PlannedWrite,
    temporary_path: PathBuf,
}

fn apply_writes(writes: Vec<PlannedWrite>) -> Result<(Vec<PathBuf>, Vec<PathBuf>), String> {
    let writes: Vec<_> = writes
        .into_iter()
        .filter(|write| {
            write.snapshot.bytes.as_deref() != Some(write.contents.as_slice())
                || write
                    .required_mode
                    .is_some_and(|required| write.snapshot.mode != Some(required))
        })
        .collect();
    if writes.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }

    let mut staged = Vec::with_capacity(writes.len());
    for write in writes {
        match stage_write(write) {
            Ok(value) => staged.push(value),
            Err(error) => {
                cleanup_staged(&staged);
                return Err(error);
            }
        }
    }

    for write in &staged {
        match write.planned.snapshot.still_matches() {
            Ok(true) => {}
            Ok(false) => {
                cleanup_staged(&staged);
                return Err(format!(
                    "{} changed while OpenMicro was preparing the installation; nothing was replaced",
                    write.planned.snapshot.logical_path.display()
                ));
            }
            Err(error) => {
                cleanup_staged(&staged);
                return Err(error);
            }
        }
    }

    let mut changed = Vec::with_capacity(staged.len());
    let mut backups = Vec::new();
    for index in 0..staged.len() {
        let write = &staged[index];
        let still_matches = match write.planned.snapshot.still_matches() {
            Ok(value) => value,
            Err(error) => {
                cleanup_staged(&staged[index..]);
                return Err(partial_write_error(error, &changed, &backups));
            }
        };
        if !still_matches {
            cleanup_staged(&staged[index..]);
            return Err(partial_write_error(
                format!(
                    "{} changed before OpenMicro could replace it",
                    write.planned.snapshot.logical_path.display()
                ),
                &changed,
                &backups,
            ));
        }

        let backup = match publish_staged(write) {
            Ok(backup) => backup,
            Err(error) => {
                // Publication never overwrites a concurrently created path.
                // A displaced live file remains at the path named in the
                // error if it could not be restored without clobbering one.
                cleanup_staged(&staged[index..]);
                return Err(partial_write_error(error, &changed, &backups));
            }
        };
        if let Some(path) = backup {
            backups.push(path);
        }
        sync_parent(&write.planned.snapshot.resolved_path);
        changed.push(write.planned.snapshot.logical_path.clone());
    }

    Ok((changed, backups))
}

fn publish_staged(write: &StagedWrite) -> Result<Option<PathBuf>, String> {
    let snapshot = &write.planned.snapshot;
    let backup = if snapshot.bytes.is_some() {
        let backup = displace_for_backup(snapshot)?;
        let matches = match snapshot.matches_file(&backup) {
            Ok(matches) => matches,
            Err(error) => {
                let recovery = restore_displaced(&backup, &snapshot.resolved_path);
                return Err(format!("{error}; {recovery}"));
            }
        };
        if !matches {
            let recovery = restore_displaced(&backup, &snapshot.resolved_path);
            return Err(format!(
                "{} changed before OpenMicro could replace it; {recovery}",
                snapshot.logical_path.display()
            ));
        }
        Some(backup)
    } else {
        None
    };

    // The target is absent after displacement (or was absent in the original
    // snapshot). Hard-link publication succeeds only if another process has
    // not created a new target in the meantime.
    if let Err(error) = fs::hard_link(&write.temporary_path, &snapshot.resolved_path) {
        let recovery = backup.as_ref().map_or_else(
            || "no existing file was changed".to_string(),
            |backup| restore_displaced(backup, &snapshot.resolved_path),
        );
        return Err(format!(
            "cannot publish {} without overwriting another change: {error}; {recovery}",
            snapshot.logical_path.display()
        ));
    }
    let _ = fs::remove_file(&write.temporary_path);
    Ok(backup)
}

fn displace_for_backup(snapshot: &FileSnapshot) -> Result<PathBuf, String> {
    let parent = snapshot.resolved_path.parent().ok_or_else(|| {
        format!(
            "{} has no parent directory",
            snapshot.logical_path.display()
        )
    })?;
    let file_name = snapshot
        .resolved_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("{} has no UTF-8 file name", snapshot.logical_path.display()))?;
    let backup_dir = create_unique_backup_dir(parent, file_name)?;
    let backup_path = backup_dir.join(file_name);
    if let Err(error) = fs::rename(&snapshot.resolved_path, &backup_path) {
        let _ = fs::remove_dir(&backup_dir);
        return Err(format!(
            "cannot move {} into a recoverable backup: {error}",
            snapshot.logical_path.display()
        ));
    }
    sync_parent(&backup_path);
    sync_parent(&backup_dir);
    Ok(backup_path)
}

fn create_unique_backup_dir(parent: &Path, file_name: &str) -> Result<PathBuf, String> {
    let seed = unique_seed();
    for attempt in 0..32u32 {
        let candidate = sibling_candidate(parent, file_name, "backup", seed, attempt);
        match fs::create_dir(&candidate) {
            Ok(()) => {
                if let Err(error) = set_file_mode(&candidate, 0o700) {
                    let _ = fs::remove_dir(&candidate);
                    return Err(error);
                }
                sync_parent(&candidate);
                return Ok(candidate);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(format!(
                    "cannot create backup directory {}: {error}",
                    candidate.display()
                ))
            }
        }
    }
    Err(format!(
        "cannot reserve a backup beside {}",
        parent.join(file_name).display()
    ))
}

fn restore_displaced(backup: &Path, target: &Path) -> String {
    match fs::hard_link(backup, target) {
        Ok(()) => {
            sync_parent(target);
            format!(
                "the live file was restored and the displaced copy remains at {}",
                backup.display()
            )
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => format!(
            "a newer live file was kept and the displaced copy remains at {}",
            backup.display()
        ),
        Err(error) => format!(
            "the displaced file remains at {} but could not be restored to {}: {error}",
            backup.display(),
            target.display()
        ),
    }
}

fn partial_write_error(error: String, changed: &[PathBuf], backups: &[PathBuf]) -> String {
    if changed.is_empty() && backups.is_empty() {
        return error;
    }
    let changed = changed
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    let backup_detail = if backups.is_empty() {
        String::new()
    } else {
        format!(
            "; backups: {}",
            backups
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    format!(
        "{error}; partial installation changed: {changed}{backup_detail}; run Install again to finish"
    )
}

fn stage_write(write: PlannedWrite) -> Result<StagedWrite, String> {
    let parent = write.snapshot.resolved_path.parent().ok_or_else(|| {
        format!(
            "{} has no parent directory",
            write.snapshot.logical_path.display()
        )
    })?;
    let parent_existed = parent.exists();
    fs::create_dir_all(parent)
        .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
    if !parent_existed {
        let _ = set_file_mode(parent, 0o700);
    }

    let (temporary_path, mut file) = create_unique_sibling(&write.snapshot.resolved_path, "tmp")?;
    let mode = write
        .required_mode
        .or(write.snapshot.mode)
        .unwrap_or(write.default_mode);
    let staged_result = (|| {
        file.write_all(&write.contents)
            .map_err(|error| format!("cannot write {}: {error}", temporary_path.display()))?;
        set_file_mode(&temporary_path, mode)?;
        file.sync_all()
            .map_err(|error| format!("cannot sync {}: {error}", temporary_path.display()))
    })();
    if let Err(error) = staged_result {
        let _ = fs::remove_file(&temporary_path);
        return Err(error);
    }

    Ok(StagedWrite {
        planned: write,
        temporary_path,
    })
}

fn create_unique_sibling(target: &Path, purpose: &str) -> Result<(PathBuf, File), String> {
    let parent = target
        .parent()
        .ok_or_else(|| format!("{} has no parent directory", target.display()))?;
    let file_name = target
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("{} has no UTF-8 file name", target.display()))?;
    let seed = unique_seed();
    for attempt in 0..32u32 {
        let candidate = sibling_candidate(parent, file_name, purpose, seed, attempt);
        let mut options = OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            // Staged settings may contain private commands or tokens. Do not
            // leave a world-readable window before chmod.
            options.mode(0o600);
        }
        match options.open(&candidate) {
            Ok(file) => return Ok((candidate, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(format!("cannot create {}: {error}", candidate.display())),
        }
    }
    Err(format!(
        "cannot reserve a temporary file beside {}",
        target.display()
    ))
}

fn unique_seed() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

fn sibling_candidate(
    parent: &Path,
    file_name: &str,
    purpose: &str,
    seed: u128,
    attempt: u32,
) -> PathBuf {
    parent.join(format!(
        ".{file_name}.openmicro-{purpose}-{}-{seed}-{attempt}",
        std::process::id()
    ))
}

fn cleanup_staged(staged: &[StagedWrite]) {
    for write in staged {
        let _ = fs::remove_file(&write.temporary_path);
    }
}

fn sync_parent(path: &Path) {
    if let Some(parent) = path.parent() {
        if let Ok(directory) = File::open(parent) {
            let _ = directory.sync_all();
        }
    }
}

fn parse_json_root(snapshot: &FileSnapshot) -> Result<Value, String> {
    let Some(bytes) = snapshot.bytes.as_ref() else {
        return Ok(Value::Object(Map::new()));
    };
    let value: Value = serde_json::from_slice(bytes).map_err(|error| {
        format!(
            "{} is not valid JSON: {error}",
            snapshot.logical_path.display()
        )
    })?;
    if !value.is_object() {
        return Err(format!(
            "{} must contain a JSON object",
            snapshot.logical_path.display()
        ));
    }
    Ok(value)
}

fn desired_hooks(
    layout: &InstallLayout,
    kind: IntegrationKind,
) -> Result<Map<String, Value>, String> {
    let template = match kind {
        IntegrationKind::Codex => CODEX_TEMPLATE,
        IntegrationKind::ClaudeCode => CLAUDE_TEMPLATE,
        _ => return Err("this integration does not use JSON lifecycle hooks".into()),
    };
    let mut root: Value = serde_json::from_str(template)
        .map_err(|error| format!("embedded hook template is invalid: {error}"))?;
    let executable = path_text(&layout.executable)?;
    let hooks = root
        .get_mut("hooks")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| "embedded hook template has no hooks object".to_string())?;

    for groups in hooks.values_mut() {
        let groups = groups
            .as_array_mut()
            .ok_or_else(|| "embedded hook event is not an array".to_string())?;
        for group in groups {
            let commands = group
                .get_mut("hooks")
                .and_then(Value::as_array_mut)
                .ok_or_else(|| "embedded hook group has no hooks array".to_string())?;
            for command in commands {
                let object = command
                    .as_object_mut()
                    .ok_or_else(|| "embedded command hook is not an object".to_string())?;
                match kind {
                    IntegrationKind::Codex => {
                        object.insert(
                            "command".into(),
                            Value::String(format!("{} agent-hook codex", shell_word(executable))),
                        );
                    }
                    IntegrationKind::ClaudeCode => {
                        object.insert("command".into(), Value::String(executable.into()));
                    }
                    _ => unreachable!(),
                }
            }
        }
    }
    Ok(hooks.clone())
}

fn shell_word(value: &str) -> String {
    if !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"/_-.+:".contains(&byte))
    {
        value.into()
    } else {
        format!("'{}'", value.replace('\'', "'\"'\"'"))
    }
}

fn inspect_json_hooks(
    layout: &InstallLayout,
    kind: IntegrationKind,
) -> Result<InstallState, String> {
    let snapshot = FileSnapshot::read(layout.target(kind))?;
    if snapshot.bytes.is_none() {
        return Ok(InstallState::NotInstalled);
    }
    let root = parse_json_root(&snapshot)?;
    let had_managed = contains_managed_hook(&root, kind, &layout.executable);
    let mut normalized = root.clone();
    merge_desired_hooks(
        &mut normalized,
        kind,
        &layout.executable,
        &desired_hooks(layout, kind)?,
    )?;
    if normalized == root {
        Ok(InstallState::Installed)
    } else if had_managed {
        Ok(InstallState::NeedsUpdate)
    } else {
        Ok(InstallState::NotInstalled)
    }
}

fn install_json_hooks(
    layout: &InstallLayout,
    kind: IntegrationKind,
    prior_state: InstallState,
) -> Result<InstallReceipt, String> {
    let snapshot = FileSnapshot::read(layout.target(kind))?;
    let mut root = parse_json_root(&snapshot)?;
    merge_desired_hooks(
        &mut root,
        kind,
        &layout.executable,
        &desired_hooks(layout, kind)?,
    )?;
    let mut contents = serde_json::to_vec_pretty(&root)
        .map_err(|error| format!("cannot serialize agent settings: {error}"))?;
    contents.push(b'\n');
    let (changed_files, backups) = apply_writes(vec![PlannedWrite {
        snapshot,
        contents,
        default_mode: 0o600,
        required_mode: None,
    }])?;
    Ok(InstallReceipt {
        disposition: disposition(prior_state, changed_files.is_empty()),
        changed_files,
        backups,
    })
}

fn merge_desired_hooks(
    root: &mut Value,
    kind: IntegrationKind,
    executable: &Path,
    desired: &Map<String, Value>,
) -> Result<(), String> {
    let root = root
        .as_object_mut()
        .ok_or_else(|| "agent settings must contain a JSON object".to_string())?;
    if !root.contains_key("hooks") {
        root.insert("hooks".into(), Value::Object(Map::new()));
    }
    let hooks = root
        .get_mut("hooks")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| "the existing hooks value must be a JSON object".to_string())?;

    let mut emptied = HashSet::new();
    for (event, value) in hooks.iter_mut() {
        let Some(groups) = value.as_array_mut() else {
            continue;
        };
        let desired_groups = desired
            .get(event)
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        if prune_managed_groups(groups, event, kind, executable, desired_groups)?
            && groups.is_empty()
        {
            emptied.insert(event.clone());
        }
    }
    for event in emptied {
        hooks.shift_remove(&event);
    }

    for (event, groups) in desired {
        let desired_groups = groups
            .as_array()
            .ok_or_else(|| format!("embedded {event} hooks are not an array"))?;
        let existing = hooks
            .entry(event.clone())
            .or_insert_with(|| Value::Array(Vec::new()))
            .as_array_mut()
            .ok_or_else(|| format!("the existing {event} hooks value must be an array"))?;
        existing.extend(desired_groups.iter().cloned());
    }
    Ok(())
}

fn prune_managed_groups(
    groups: &mut Vec<Value>,
    event: &str,
    kind: IntegrationKind,
    executable: &Path,
    desired_groups: &[Value],
) -> Result<bool, String> {
    let mut changed = false;
    let mut retained = Vec::with_capacity(groups.len());
    for group in std::mem::take(groups) {
        if is_known_managed_group(&group, event, kind, executable, desired_groups) {
            changed = true;
        } else if contains_openmicro_hook_candidate(&group, kind, executable) {
            return Err(format!(
                "the existing {event} OpenMicro hook was customized; remove it manually before installing"
            ));
        } else {
            retained.push(group);
        }
    }
    *groups = retained;
    Ok(changed)
}

fn is_known_managed_group(
    group: &Value,
    event: &str,
    kind: IntegrationKind,
    executable: &Path,
    desired_groups: &[Value],
) -> bool {
    let Some(normalized) = normalized_managed_group(group, kind, executable) else {
        return false;
    };
    if desired_groups.iter().any(|desired| {
        normalized_managed_group(desired, kind, executable).as_ref() == Some(&normalized)
    }) {
        return true;
    }

    kind == IntegrationKind::Codex
        && legacy_codex_group(event).is_some_and(|legacy| {
            normalized_managed_group(&legacy, kind, executable).as_ref() == Some(&normalized)
        })
}

/// Normalize only the executable in a complete one-command hook group. All
/// other group and command fields remain part of the ownership fingerprint.
fn normalized_managed_group(
    group: &Value,
    kind: IntegrationKind,
    executable: &Path,
) -> Option<Value> {
    let mut normalized = group.clone();
    let commands = normalized.get_mut("hooks")?.as_array_mut()?;
    if commands.len() != 1 || !is_managed_hook(&commands[0], kind, executable) {
        return None;
    }
    commands[0].as_object_mut()?.insert(
        "command".into(),
        Value::String("<OPENMICRO_EXECUTABLE>".into()),
    );
    Some(normalized)
}

fn legacy_codex_group(event: &str) -> Option<Value> {
    let mut command = Map::new();
    command.insert("type".into(), Value::String("command".into()));
    command.insert(
        "command".into(),
        Value::String(format!("{DEFAULT_APP_BINARY} codex-hook")),
    );
    command.insert("timeout".into(), Value::from(2));
    match event {
        "UserPromptSubmit" | "PermissionRequest" | "PostToolUse" | "Stop" => {
            command.insert("async".into(), Value::Bool(true));
        }
        "SessionEnd" => {}
        _ => return None,
    }
    Some(serde_json::json!({ "hooks": [Value::Object(command)] }))
}

fn contains_openmicro_hook_candidate(
    group: &Value,
    kind: IntegrationKind,
    executable: &Path,
) -> bool {
    group
        .get("hooks")
        .and_then(Value::as_array)
        .is_some_and(|commands| {
            commands
                .iter()
                .any(|command| looks_like_openmicro_hook(command, kind, executable))
        })
}

fn contains_managed_hook(root: &Value, kind: IntegrationKind, executable: &Path) -> bool {
    root.get("hooks")
        .and_then(Value::as_object)
        .is_some_and(|events| {
            events.values().any(|groups| {
                groups.as_array().is_some_and(|groups| {
                    groups.iter().any(|group| {
                        group
                            .get("hooks")
                            .and_then(Value::as_array)
                            .is_some_and(|commands| {
                                commands
                                    .iter()
                                    .any(|command| is_managed_hook(command, kind, executable))
                            })
                    })
                })
            })
        })
}

fn is_managed_hook(command: &Value, kind: IntegrationKind, executable: &Path) -> bool {
    let Some(object) = command.as_object() else {
        return false;
    };
    if object.get("type").and_then(Value::as_str) != Some("command") {
        return false;
    }
    let Some(command_text) = object.get("command").and_then(Value::as_str) else {
        return false;
    };

    match kind {
        IntegrationKind::Codex => managed_shell_invocation(
            command_text,
            executable,
            &["agent-hook codex", "codex-hook"],
        ),
        IntegrationKind::ClaudeCode => {
            let args_match = object
                .get("args")
                .and_then(Value::as_array)
                .is_some_and(|args| {
                    args.len() == 2
                        && args[0].as_str() == Some("agent-hook")
                        && args[1].as_str() == Some("claude-code")
                });
            (args_match && is_managed_binary_path(command_text, executable))
                || managed_shell_invocation(
                    command_text,
                    executable,
                    &["agent-hook claude-code", "claude-hook", "claude-code-hook"],
                )
        }
        IntegrationKind::OpenCode | IntegrationKind::DeepCode => false,
    }
}

fn looks_like_openmicro_hook(command: &Value, kind: IntegrationKind, executable: &Path) -> bool {
    if is_managed_hook(command, kind, executable) {
        return true;
    }
    let Some(object) = command.as_object() else {
        return false;
    };
    let Some(command_text) = object.get("command").and_then(Value::as_str) else {
        return false;
    };

    match kind {
        IntegrationKind::Codex => {
            decoded_shell_invocation(command_text).is_some_and(|(program, arguments)| {
                is_managed_binary_path(&program, executable)
                    && (arguments.starts_with("agent-hook codex")
                        || arguments.starts_with("codex-hook"))
            })
        }
        IntegrationKind::ClaudeCode => {
            let direct = is_managed_binary_path(command_text, executable)
                && object
                    .get("args")
                    .and_then(Value::as_array)
                    .and_then(|args| args.first())
                    .and_then(Value::as_str)
                    .is_some_and(|first| {
                        matches!(first, "agent-hook" | "claude-hook" | "claude-code-hook")
                    });
            direct
                || decoded_shell_invocation(command_text).is_some_and(|(program, arguments)| {
                    is_managed_binary_path(&program, executable)
                        && (arguments.starts_with("agent-hook claude")
                            || arguments.starts_with("claude-hook")
                            || arguments.starts_with("claude-code-hook"))
                })
        }
        IntegrationKind::OpenCode | IntegrationKind::DeepCode => false,
    }
}

/// Recognize only the shell grammar emitted by [`shell_word`], followed by
/// one exact helper invocation. Substring matching here would let an unrelated
/// command such as `echo openmicro agent-hook codex` be deleted during merge.
fn managed_shell_invocation(command: &str, executable: &Path, suffixes: &[&str]) -> bool {
    suffixes.iter().any(|suffix| {
        let Some(program) = command.strip_suffix(&format!(" {suffix}")) else {
            return false;
        };
        let Some(program) = decode_shell_word(program) else {
            return false;
        };
        is_managed_binary_path(&program, executable)
    })
}

fn decoded_shell_invocation(command: &str) -> Option<(String, &str)> {
    for (index, character) in command.char_indices() {
        if character != ' ' {
            continue;
        }
        if let Some(program) = decode_shell_word(&command[..index]) {
            let arguments = command[index..].trim_start_matches(' ');
            if !arguments.is_empty() {
                return Some((program, arguments));
            }
        }
    }
    None
}

fn decode_shell_word(word: &str) -> Option<String> {
    if word.is_empty() {
        return None;
    }
    if word
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || b"/_-.+:".contains(&byte))
    {
        return Some(word.into());
    }
    let inner = word.strip_prefix('\'')?.strip_suffix('\'')?;
    let decoded = inner.replace("'\"'\"'", "'");
    (shell_word(&decoded) == word).then_some(decoded)
}

fn is_managed_binary_path(candidate: &str, executable: &Path) -> bool {
    let candidate = Path::new(candidate);
    if candidate == executable || candidate == Path::new(DEFAULT_APP_BINARY) {
        return true;
    }
    if !candidate.is_absolute() {
        return false;
    }

    // Recognize stale source-build paths and normal macOS bundle binaries,
    // while leaving arbitrary wrappers containing "openmicro" untouched.
    let file_name = candidate.file_name().and_then(|name| name.to_str());
    if file_name == Some("openmicro-app") {
        return true;
    }
    file_name == Some("OpenMicro")
        && candidate
            .parent()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            == Some("MacOS")
        && candidate
            .parent()
            .and_then(Path::parent)
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            == Some("Contents")
}

fn render_opencode_plugin(executable: &Path) -> Result<Vec<u8>, String> {
    replace_declaration(
        OPENCODE_TEMPLATE,
        &format!(
            "const OPENMICRO = {}",
            serde_json::to_string(DEFAULT_APP_BINARY).expect("static path is JSON")
        ),
        &format!(
            "const OPENMICRO = {}",
            serde_json::to_string(path_text(executable)?)
                .map_err(|error| format!("cannot quote executable path: {error}"))?
        ),
    )
}

fn render_deep_code_script(executable: &Path) -> Result<Vec<u8>, String> {
    replace_declaration(
        DEEP_CODE_TEMPLATE,
        &format!("OPENMICRO_BIN=\"{DEFAULT_APP_BINARY}\""),
        &format!("OPENMICRO_BIN={}", shell_word(path_text(executable)?)),
    )
}

fn replace_declaration(template: &str, old: &str, new: &str) -> Result<Vec<u8>, String> {
    if template.matches(old).count() != 1 {
        return Err("embedded integration template has an unexpected binary declaration".into());
    }
    let rendered = template
        .replacen(old, new, 1)
        .replacen(LEGACY_MANAGED_MARKER, MANAGED_MARKER, 1)
        .replacen(LEGACY_SHELL_MANAGED_MARKER, SHELL_MANAGED_MARKER, 1);
    Ok(rendered.into_bytes())
}

fn inspect_owned_template(
    path: &Path,
    desired: &Result<Vec<u8>, String>,
    declaration_prefix: &str,
) -> Result<InstallState, String> {
    let desired = desired.as_ref().map_err(Clone::clone)?;
    let snapshot = FileSnapshot::read(path)?;
    let Some(existing) = snapshot.bytes.as_ref() else {
        return Ok(InstallState::NotInstalled);
    };
    if existing == desired {
        return Ok(InstallState::Installed);
    }
    let existing = std::str::from_utf8(existing)
        .map_err(|_| format!("{} is not a UTF-8 text file", path.display()))?;
    let desired = std::str::from_utf8(desired).expect("embedded templates are UTF-8");
    if has_managed_marker(existing) {
        return Ok(InstallState::NeedsUpdate);
    }
    if normalized_owned_template(existing, declaration_prefix)
        == normalized_owned_template(desired, declaration_prefix)
    {
        Ok(InstallState::NeedsUpdate)
    } else {
        Err(format!(
            "{} already exists and is not an unmodified OpenMicro integration",
            path.display()
        ))
    }
}

fn has_managed_marker(text: &str) -> bool {
    text.lines().any(|line| {
        matches!(
            line,
            MANAGED_MARKER
                | SHELL_MANAGED_MARKER
                | LEGACY_MANAGED_MARKER
                | LEGACY_SHELL_MANAGED_MARKER
        )
    })
}

fn normalized_owned_template(text: &str, declaration_prefix: &str) -> Option<String> {
    let without_marker = text
        .replacen(&format!("{MANAGED_MARKER}\n"), "", 1)
        .replacen(&format!("{SHELL_MANAGED_MARKER}\n"), "", 1)
        .replacen(&format!("{LEGACY_MANAGED_MARKER}\n"), "", 1)
        .replacen(&format!("{LEGACY_SHELL_MANAGED_MARKER}\n"), "", 1);
    let mut found = 0usize;
    let mut normalized = String::with_capacity(without_marker.len());
    for line in without_marker.split_inclusive('\n') {
        let (content, newline) = line
            .strip_suffix('\n')
            .map_or((line, ""), |content| (content, "\n"));
        if content.starts_with(declaration_prefix) {
            found += 1;
            normalized.push_str(declaration_prefix);
            normalized.push_str("<OPENMICRO_EXECUTABLE>");
            normalized.push_str(newline);
        } else {
            normalized.push_str(line);
        }
    }
    (found == 1).then_some(normalized)
}

fn install_opencode(
    layout: &InstallLayout,
    prior_state: InstallState,
) -> Result<InstallReceipt, String> {
    let desired = render_opencode_plugin(&layout.executable)?;
    inspect_owned_template(
        &layout.opencode_plugin,
        &Ok(desired.clone()),
        "const OPENMICRO = ",
    )?;
    let snapshot = FileSnapshot::read(&layout.opencode_plugin)?;
    let (changed_files, backups) = apply_writes(vec![PlannedWrite {
        snapshot,
        contents: desired,
        default_mode: 0o600,
        required_mode: None,
    }])?;
    Ok(InstallReceipt {
        disposition: disposition(prior_state, changed_files.is_empty()),
        changed_files,
        backups,
    })
}

fn inspect_deep_code(layout: &InstallLayout) -> Result<InstallState, String> {
    let settings = FileSnapshot::read(&layout.deep_code_settings)?;
    let root = parse_json_root(&settings)?;
    let object = root
        .as_object()
        .ok_or_else(|| "Deep Code settings must contain a JSON object".to_string())?;
    let script_path = path_text(&layout.deep_code_script)?;
    let notify_installed = match object.get("notify") {
        None => false,
        Some(Value::String(value)) if value == script_path => true,
        Some(_) => {
            return Err(format!(
                "{} already configures a different notifier; OpenMicro will not replace it",
                layout.deep_code_settings.display()
            ))
        }
    };

    let mut script_state = inspect_owned_template(
        &layout.deep_code_script,
        &render_deep_code_script(&layout.executable),
        "OPENMICRO_BIN=",
    )?;
    if script_state == InstallState::Installed
        && FileSnapshot::read(&layout.deep_code_script)?.mode != Some(0o700)
    {
        script_state = InstallState::NeedsUpdate;
    }
    match (notify_installed, script_state) {
        (true, InstallState::Installed) => Ok(InstallState::Installed),
        (false, InstallState::NotInstalled) => Ok(InstallState::NotInstalled),
        _ => Ok(InstallState::NeedsUpdate),
    }
}

fn install_deep_code(
    layout: &InstallLayout,
    prior_state: InstallState,
) -> Result<InstallReceipt, String> {
    let settings_snapshot = FileSnapshot::read(&layout.deep_code_settings)?;
    let mut settings = parse_json_root(&settings_snapshot)?;
    let settings_object = settings
        .as_object_mut()
        .ok_or_else(|| "Deep Code settings must contain a JSON object".to_string())?;
    let script_path = path_text(&layout.deep_code_script)?;
    match settings_object.get("notify") {
        None => {}
        Some(Value::String(value)) if value == script_path => {}
        Some(_) => {
            return Err(format!(
                "{} already configures a different notifier; OpenMicro will not replace it",
                layout.deep_code_settings.display()
            ))
        }
    }

    let desired_script = render_deep_code_script(&layout.executable)?;
    inspect_owned_template(
        &layout.deep_code_script,
        &Ok(desired_script.clone()),
        "OPENMICRO_BIN=",
    )?;
    let script_snapshot = FileSnapshot::read(&layout.deep_code_script)?;

    settings_object.insert("notify".into(), Value::String(script_path.into()));
    let mut settings_contents = serde_json::to_vec_pretty(&settings)
        .map_err(|error| format!("cannot serialize Deep Code settings: {error}"))?;
    settings_contents.push(b'\n');

    let (changed_files, backups) = apply_writes(vec![
        PlannedWrite {
            snapshot: script_snapshot,
            contents: desired_script,
            default_mode: 0o700,
            required_mode: Some(0o700),
        },
        PlannedWrite {
            snapshot: settings_snapshot,
            contents: settings_contents,
            default_mode: 0o600,
            required_mode: None,
        },
    ])?;
    Ok(InstallReceipt {
        disposition: disposition(prior_state, changed_files.is_empty()),
        changed_files,
        backups,
    })
}

fn disposition(prior_state: InstallState, unchanged: bool) -> InstallDisposition {
    if unchanged {
        InstallDisposition::AlreadyInstalled
    } else if prior_state == InstallState::NotInstalled {
        InstallDisposition::Installed
    } else {
        InstallDisposition::Updated
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_DIR_COUNTER: AtomicU64 = AtomicU64::new(0);

    struct TestDir(PathBuf);

    impl TestDir {
        fn new() -> Self {
            let id = TEST_DIR_COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = env::temp_dir().join(format!(
                "openmicro-agent-integrations-{}-{id}",
                std::process::id()
            ));
            fs::create_dir_all(&path).expect("create test directory");
            Self(path)
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn layout_in(root: &TestDir) -> InstallLayout {
        let executable = root.0.join("OpenMicro build's binary");
        fs::write(&executable, b"test executable").expect("write executable");
        InstallLayout::test(&root.0, &executable)
    }

    #[test]
    fn codex_install_preserves_other_hooks_and_upgrades_legacy_entries() {
        let root = TestDir::new();
        let layout = layout_in(&root);
        fs::create_dir_all(layout.codex_hooks.parent().unwrap()).unwrap();
        let original = serde_json::json!({
            "description": "mine",
            "custom": { "token": "do not remove" },
            "hooks": {
                "Stop": [
                    { "matcher": "keep", "hooks": [
                        { "type": "command", "command": "echo keep" },
                        { "type": "command", "command": "echo OpenMicro agent-hook codex" }
                    ] },
                    { "hooks": [{
                        "type": "command",
                        "command": "/Applications/OpenMicro.app/Contents/MacOS/OpenMicro codex-hook",
                        "timeout": 2,
                        "async": true
                    }] }
                ]
            }
        });
        let original_bytes = serde_json::to_vec_pretty(&original).unwrap();
        fs::write(&layout.codex_hooks, &original_bytes).unwrap();

        let receipt = install(&layout, IntegrationKind::Codex).expect("install Codex hooks");
        assert_eq!(receipt.disposition, InstallDisposition::Updated);
        assert_eq!(
            inspect(&layout, IntegrationKind::Codex).state,
            InstallState::Installed
        );

        let installed: Value =
            serde_json::from_slice(&fs::read(&layout.codex_hooks).unwrap()).unwrap();
        assert_eq!(installed["description"], "mine");
        assert_eq!(installed["custom"]["token"], "do not remove");
        let stop = installed["hooks"]["Stop"].as_array().unwrap();
        assert!(stop.iter().any(|group| group["matcher"] == "keep"));
        assert!(serde_json::to_string(&installed)
            .unwrap()
            .contains("echo OpenMicro agent-hook codex"));
        assert!(!serde_json::to_string(&installed)
            .unwrap()
            .contains("\"async\":true"));
        assert!(serde_json::to_string(&installed)
            .unwrap()
            .contains("agent-hook codex"));

        let backup = receipt.backups.single().expect("one backup");
        assert_eq!(fs::read(backup).unwrap(), original_bytes);
        let before = fs::read(&layout.codex_hooks).unwrap();
        let second = install(&layout, IntegrationKind::Codex).expect("idempotent reinstall");
        assert_eq!(second.disposition, InstallDisposition::AlreadyInstalled);
        assert!(second.changed_files.is_empty());
        assert_eq!(fs::read(&layout.codex_hooks).unwrap(), before);
    }

    #[test]
    fn claude_install_uses_exec_form_and_quotes_nothing_in_the_path_field() {
        let root = TestDir::new();
        let layout = layout_in(&root);
        fs::create_dir_all(layout.claude_settings.parent().unwrap()).unwrap();
        fs::write(
            &layout.claude_settings,
            br#"{"permissions":{"allow":["Read"]},"hooks":{"Stop":[{"hooks":[{"type":"command","command":"echo keep"}]}]}}"#,
        )
        .unwrap();

        install(&layout, IntegrationKind::ClaudeCode).expect("install Claude hooks");
        let installed: Value =
            serde_json::from_slice(&fs::read(&layout.claude_settings).unwrap()).unwrap();
        assert_eq!(installed["permissions"]["allow"][0], "Read");
        let serialized = serde_json::to_string(&installed).unwrap();
        assert!(serialized.contains(path_text(&layout.executable).unwrap()));
        assert!(serialized.contains("claude-code"));
        assert!(installed["hooks"]["Stop"]
            .as_array()
            .unwrap()
            .iter()
            .any(|group| group["hooks"][0]["command"] == "echo keep"));
    }

    #[test]
    fn malformed_json_is_a_conflict_and_is_never_replaced() {
        let root = TestDir::new();
        let layout = layout_in(&root);
        fs::create_dir_all(layout.codex_hooks.parent().unwrap()).unwrap();
        fs::write(&layout.codex_hooks, b"{not json").unwrap();
        let before = fs::read(&layout.codex_hooks).unwrap();

        let report = inspect(&layout, IntegrationKind::Codex);
        assert_eq!(report.state, InstallState::Conflict);
        assert!(install(&layout, IntegrationKind::Codex).is_err());
        assert_eq!(fs::read(&layout.codex_hooks).unwrap(), before);
    }

    #[test]
    fn incompatible_desired_event_shape_is_a_conflict_and_is_never_replaced() {
        let root = TestDir::new();
        let layout = layout_in(&root);
        fs::create_dir_all(layout.codex_hooks.parent().unwrap()).unwrap();
        let before = br#"{"hooks":{"Stop":{"custom":"leave this alone"}}}"#;
        fs::write(&layout.codex_hooks, before).unwrap();

        let report = inspect(&layout, IntegrationKind::Codex);
        assert_eq!(report.state, InstallState::Conflict);
        assert!(report.detail.unwrap().contains("Stop"));
        assert!(install(&layout, IntegrationKind::Codex).is_err());
        assert_eq!(fs::read(&layout.codex_hooks).unwrap(), before);
    }

    #[test]
    fn managed_hook_recognition_requires_an_exact_invocation_grammar() {
        let executable = Path::new("/tmp/OpenMicro.app/Contents/MacOS/OpenMicro");
        let exact = serde_json::json!({
            "type": "command",
            "command": "/tmp/OpenMicro.app/Contents/MacOS/OpenMicro agent-hook codex"
        });
        assert!(is_managed_hook(&exact, IntegrationKind::Codex, executable));

        for unrelated in [
            serde_json::json!({
                "type": "command",
                "command": "echo OpenMicro agent-hook codex"
            }),
            serde_json::json!({
                "type": "notification",
                "command": "/Applications/OpenMicro.app/Contents/MacOS/OpenMicro agent-hook codex"
            }),
            serde_json::json!({
                "type": "command",
                "command": "/tmp/my-openmicro-wrapper",
                "args": ["agent-hook", "claude-code"]
            }),
        ] {
            assert!(!is_managed_hook(
                &unrelated,
                IntegrationKind::Codex,
                executable
            ));
            assert!(!is_managed_hook(
                &unrelated,
                IntegrationKind::ClaudeCode,
                executable
            ));
        }

        let legacy = serde_json::json!({
            "type": "command",
            "command": "/Applications/OpenMicro.app/Contents/MacOS/OpenMicro codex-hook"
        });
        assert!(is_managed_hook(&legacy, IntegrationKind::Codex, executable));
    }

    #[test]
    fn customized_openmicro_hook_group_requires_manual_review() {
        let root = TestDir::new();
        let layout = layout_in(&root);
        fs::create_dir_all(layout.codex_hooks.parent().unwrap()).unwrap();
        let command = format!(
            "{} agent-hook codex",
            shell_word(path_text(&layout.executable).unwrap())
        );
        let existing = serde_json::json!({
            "hooks": {
                "Stop": [{
                    "matcher": "user customization",
                    "hooks": [{
                        "type": "command",
                        "command": command,
                        "timeout": 9
                    }]
                }]
            }
        });
        let before = serde_json::to_vec_pretty(&existing).unwrap();
        fs::write(&layout.codex_hooks, &before).unwrap();

        let report = inspect(&layout, IntegrationKind::Codex);
        assert_eq!(report.state, InstallState::Conflict);
        assert!(report.detail.unwrap().contains("customized"));
        assert!(install(&layout, IntegrationKind::Codex).is_err());
        assert_eq!(fs::read(&layout.codex_hooks).unwrap(), before);
    }

    #[test]
    fn opencode_owned_template_updates_but_foreign_file_does_not() {
        let root = TestDir::new();
        let layout = layout_in(&root);
        fs::create_dir_all(layout.opencode_plugin.parent().unwrap()).unwrap();
        fs::write(&layout.opencode_plugin, b"export const unrelated = true\n").unwrap();
        assert_eq!(
            inspect(&layout, IntegrationKind::OpenCode).state,
            InstallState::Conflict
        );
        assert!(install(&layout, IntegrationKind::OpenCode).is_err());
        assert_eq!(
            fs::read(&layout.opencode_plugin).unwrap(),
            b"export const unrelated = true\n"
        );

        fs::remove_file(&layout.opencode_plugin).unwrap();
        install(&layout, IntegrationKind::OpenCode).expect("install OpenCode plugin");
        assert_eq!(
            inspect(&layout, IntegrationKind::OpenCode).state,
            InstallState::Installed
        );
        let installed = fs::read_to_string(&layout.opencode_plugin).unwrap();
        assert!(installed.contains(MANAGED_MARKER));
        assert!(installed.contains("Remove this line before editing"));
        assert!(installed.contains(path_text(&layout.executable).unwrap()));

        let old_executable = path_text(&layout.executable).unwrap();
        fs::write(
            &layout.opencode_plugin,
            installed.replace(old_executable, "/old/OpenMicro"),
        )
        .unwrap();
        assert_eq!(
            inspect(&layout, IntegrationKind::OpenCode).state,
            InstallState::NeedsUpdate
        );

        fs::write(
            &layout.opencode_plugin,
            installed.replace("export const OpenMicroStatus", "export const LocallyEdited"),
        )
        .unwrap();
        assert_eq!(
            inspect(&layout, IntegrationKind::OpenCode).state,
            InstallState::NeedsUpdate,
            "the explicit managed marker permits future template upgrades"
        );

        let unowned_edit = installed
            .replace(&format!("{MANAGED_MARKER}\n"), "")
            .replace("export const OpenMicroStatus", "export const LocallyEdited");
        fs::write(&layout.opencode_plugin, unowned_edit).unwrap();
        assert_eq!(
            inspect(&layout, IntegrationKind::OpenCode).state,
            InstallState::Conflict,
            "removing the marker protects local edits"
        );
    }

    #[test]
    fn deep_code_refuses_to_replace_an_existing_notifier() {
        let root = TestDir::new();
        let layout = layout_in(&root);
        fs::create_dir_all(layout.deep_code_settings.parent().unwrap()).unwrap();
        fs::write(
            &layout.deep_code_settings,
            br#"{"notify":"/usr/local/bin/my-notifier","model":"deepseek"}"#,
        )
        .unwrap();
        let before = fs::read(&layout.deep_code_settings).unwrap();

        assert_eq!(
            inspect(&layout, IntegrationKind::DeepCode).state,
            InstallState::Conflict
        );
        assert!(install(&layout, IntegrationKind::DeepCode).is_err());
        assert_eq!(fs::read(&layout.deep_code_settings).unwrap(), before);
        assert!(!layout.deep_code_script.exists());
    }

    #[test]
    fn deep_code_install_is_recoverable_and_makes_the_script_executable() {
        let root = TestDir::new();
        let layout = layout_in(&root);
        let receipt = install(&layout, IntegrationKind::DeepCode).expect("install Deep Code");
        assert_eq!(receipt.disposition, InstallDisposition::Installed);
        assert_eq!(
            inspect(&layout, IntegrationKind::DeepCode).state,
            InstallState::Installed
        );
        let settings: Value =
            serde_json::from_slice(&fs::read(&layout.deep_code_settings).unwrap()).unwrap();
        assert_eq!(
            settings["notify"],
            path_text(&layout.deep_code_script).unwrap()
        );
        let script = fs::read_to_string(&layout.deep_code_script).unwrap();
        assert!(script.contains(&shell_word(path_text(&layout.executable).unwrap())));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&layout.deep_code_script)
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o700);
        }
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_settings_keep_the_symlink_permissions_and_exact_backup() {
        use std::os::unix::fs::{symlink, MetadataExt, PermissionsExt};

        let root = TestDir::new();
        let layout = layout_in(&root);
        let real_dir = root.0.join("dotfiles");
        let real_settings = real_dir.join("codex-hooks.json");
        fs::create_dir_all(&real_dir).unwrap();
        let original = br#"{"custom":{"keep":true}}"#;
        fs::write(&real_settings, original).unwrap();
        fs::set_permissions(&real_settings, fs::Permissions::from_mode(0o640)).unwrap();
        let original_inode = fs::metadata(&real_settings).unwrap().ino();
        fs::create_dir_all(layout.codex_hooks.parent().unwrap()).unwrap();
        symlink(&real_settings, &layout.codex_hooks).unwrap();

        let receipt = install(&layout, IntegrationKind::Codex).expect("install via symlink");
        assert!(fs::symlink_metadata(&layout.codex_hooks)
            .unwrap()
            .file_type()
            .is_symlink());
        let installed: Value = serde_json::from_slice(&fs::read(&real_settings).unwrap()).unwrap();
        assert_eq!(installed["custom"]["keep"], true);
        assert!(installed["hooks"].is_object());
        assert_eq!(
            fs::metadata(&real_settings).unwrap().permissions().mode() & 0o777,
            0o640
        );

        let backup = receipt.backups.single().expect("one backup");
        assert_eq!(fs::read(backup).unwrap(), original);
        assert_eq!(fs::metadata(backup).unwrap().ino(), original_inode);
        assert_eq!(
            fs::metadata(backup).unwrap().permissions().mode() & 0o777,
            0o640
        );
    }

    #[cfg(unix)]
    #[test]
    fn new_private_config_is_created_with_mode_0600() {
        use std::os::unix::fs::PermissionsExt;

        let root = TestDir::new();
        let layout = layout_in(&root);
        install(&layout, IntegrationKind::Codex).expect("install Codex hooks");
        assert_eq!(
            fs::metadata(&layout.codex_hooks)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }

    #[test]
    fn stale_snapshot_never_overwrites_a_concurrent_edit() {
        let root = TestDir::new();
        let target = root.0.join("settings.json");
        fs::write(&target, b"before").unwrap();
        let snapshot = FileSnapshot::read(&target).unwrap();
        fs::write(&target, b"edited by another process").unwrap();

        let error = apply_writes(vec![PlannedWrite {
            snapshot,
            contents: b"OpenMicro replacement".to_vec(),
            default_mode: 0o600,
            required_mode: None,
        }])
        .expect_err("stale plan must fail");
        assert!(error.contains("changed"));
        assert_eq!(fs::read(&target).unwrap(), b"edited by another process");
        assert!(!fs::read_dir(&root.0).unwrap().flatten().any(|entry| entry
            .file_name()
            .to_string_lossy()
            .contains("openmicro-tmp")));
    }

    #[test]
    fn atomic_replacement_after_final_check_is_restored_and_backed_up() {
        let root = TestDir::new();
        let target = root.0.join("settings.json");
        fs::write(&target, b"before").unwrap();
        let snapshot = FileSnapshot::read(&target).unwrap();
        let staged = stage_write(PlannedWrite {
            snapshot,
            contents: b"OpenMicro replacement".to_vec(),
            default_mode: 0o600,
            required_mode: None,
        })
        .unwrap();

        assert!(staged.planned.snapshot.still_matches().unwrap());
        let editor_temporary = root.0.join("editor-save.tmp");
        fs::write(&editor_temporary, b"atomic editor replacement").unwrap();
        fs::rename(&editor_temporary, &target).unwrap();

        let error = publish_staged(&staged).expect_err("late atomic replacement must win");
        cleanup_staged(std::slice::from_ref(&staged));
        assert!(error.contains("changed"));
        assert_eq!(fs::read(&target).unwrap(), b"atomic editor replacement");

        let backup = fs::read_dir(&root.0)
            .unwrap()
            .flatten()
            .find(|entry| {
                entry.file_type().is_ok_and(|kind| kind.is_dir())
                    && entry
                        .file_name()
                        .to_string_lossy()
                        .contains("openmicro-backup")
            })
            .map(|entry| entry.path().join("settings.json"))
            .expect("displaced editor save backup");
        assert_eq!(fs::read(backup).unwrap(), b"atomic editor replacement");
    }

    trait Single<T> {
        fn single(&self) -> Option<&T>;
    }

    impl<T> Single<T> for Vec<T> {
        fn single(&self) -> Option<&T> {
            (self.len() == 1).then(|| &self[0])
        }
    }

    #[test]
    fn shell_word_handles_spaces_and_apostrophes() {
        assert_eq!(shell_word("/tmp/OpenMicro"), "/tmp/OpenMicro");
        assert_eq!(
            shell_word("/tmp/Tony's OpenMicro"),
            "'/tmp/Tony'\"'\"'s OpenMicro'"
        );
    }

    #[test]
    fn relative_environment_override_is_rejected() {
        const NAME: &str = "OPENMICRO_TEST_RELATIVE_CONFIG_HOME";
        let previous = env::var_os(NAME);
        env::set_var(NAME, "relative/config");
        let result = nonempty_env_path(NAME);
        match previous {
            Some(value) => env::set_var(NAME, value),
            None => env::remove_var(NAME),
        }
        assert_eq!(result, Err(format!("{NAME} must be an absolute path")));
    }
}
