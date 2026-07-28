//! Persisted app configuration: profiles, per-input actions, defaults,
//! migration and import/export.
//!
//! The firmware emits a fixed, remappable HID usage per input slot (the
//! `Slot` array synced over the vendor-HID protocol, see device.rs); what a
//! slot *means* on the host — its `Action` — lives here, per machine. Stored
//! as JSON under the OS config dir (e.g. ~/Library/Application
//! Support/OpenMicro/config.json), the same path the pre-profile app used, so
//! `load()` transparently migrates the old `{ bindings: [...] }` schema.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Slot layout — shared contract, imported by every other module.
// ---------------------------------------------------------------------------

pub const SLOT_COUNT: usize = 24;
pub const KEY_SLOTS: usize = 13; // slots 0..=12 are the 13 physical keys
pub const SLOT_ENC_CW: usize = 13;
pub const SLOT_ENC_CCW: usize = 14;
pub const SLOT_ENC_PRESS: usize = 15;
pub const SLOT_JOY_UP: usize = 16;
pub const SLOT_JOY_DOWN: usize = 17;
pub const SLOT_JOY_LEFT: usize = 18;
pub const SLOT_JOY_RIGHT: usize = 19;
pub const SLOT_JOY_PRESS: usize = 20;
pub const SLOT_TOUCH_TAP: usize = 21;
pub const SLOT_TOUCH_SWIPE_L: usize = 22; // no hardware support yet (single-zone pad); config carries it
pub const SLOT_TOUCH_SWIPE_R: usize = 23;

/// Display name of each slot, indexed by slot number. Key names carry the
/// physical grid position (2 + 4 + 4 + 3 keys, top to bottom).
pub const SLOT_NAMES: [&'static str; SLOT_COUNT] = [
    "Key 1 · row 1",
    "Key 2 · row 1",
    "Key 3 · row 2",
    "Key 4 · row 2",
    "Key 5 · row 2",
    "Key 6 · row 2",
    "Key 7 · row 3",
    "Key 8 · row 3",
    "Key 9 · row 3",
    "Key 10 · row 3",
    "Key 11 · row 4",
    "Key 12 · row 4",
    "Key 13 · row 4",
    "Encoder · clockwise",
    "Encoder · counter-clockwise",
    "Encoder · press",
    "Joystick · up",
    "Joystick · down",
    "Joystick · left",
    "Joystick · right",
    "Joystick · press",
    "Touch pad · tap",
    "Touch pad · swipe left",
    "Touch pad · swipe right",
];

// ---------------------------------------------------------------------------
// What the device emits (wire-format mirror).
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "snake_case")]
pub enum SlotKind {
    #[default]
    None,
    Keyboard,
    Consumer,
}

/// What the firmware emits for one input. Mirrors the wire format (4 bytes:
/// kind, mods, code LE). mods = HID modifier bitmask (bit0 LCtrl, bit1 LShift,
/// bit2 LAlt, bit3 LGui); code = HID keyboard usage (kind=Keyboard) or
/// consumer usage (kind=Consumer).
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Slot {
    pub kind: SlotKind,
    pub mods: u8,
    pub code: u16,
}

// ---------------------------------------------------------------------------
// What the host does when a slot fires.
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum MediaOp {
    VolumeUp,
    VolumeDown,
    Mute,
    PlayPause,
    NextTrack,
    PrevTrack,
    BrightnessUp,
    BrightnessDown,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MacroStep {
    Keystroke { mods: u8, key: u16 },
    Delay { ms: u64 },
    Run { command: String },
    Open { target: String },
    Media { op: MediaOp },
}

fn default_true() -> bool {
    true
}

/// One macro step plus its enabled flag (the PRD's per-step disable). The
/// flatten + default keep step JSON from before the flag loading unchanged.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct MacroStepEntry {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(flatten)]
    pub step: MacroStep,
}

impl From<MacroStep> for MacroStepEntry {
    fn from(step: MacroStep) -> Self {
        MacroStepEntry {
            enabled: true,
            step,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Action {
    #[default]
    None,
    Keystroke { mods: u8, key: u16 },
    Macro { steps: Vec<MacroStepEntry> },
    Run { command: String },
    Open { target: String },
    Media { op: MediaOp },
    AppSettings, // open this app's settings sheet (the SETUP key default)
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct InputConfig {
    pub label: String,
    pub icon: String,
    pub emitted: Slot,
    pub action: Action,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct AnalogTuning {
    pub joy_threshold: u16, // ADC deviation from centre 2048; default 1024
}

impl Default for AnalogTuning {
    fn default() -> Self {
        AnalogTuning { joy_threshold: 1024 }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Profile {
    pub name: String,
    pub inputs: Vec<InputConfig>,
    pub analog: AnalogTuning,
}
// invariant: inputs.len() == SLOT_COUNT, index = slot index

impl Profile {
    /// The emitted-code array for device sync, indexed by slot. Slots beyond
    /// `inputs` (only possible pre-sanitize) come back as `SlotKind::None`.
    pub fn slots(&self) -> [Slot; SLOT_COUNT] {
        let mut out = [Slot::default(); SLOT_COUNT];
        for (i, slot) in out.iter_mut().enumerate() {
            if let Some(input) = self.inputs.get(i) {
                *slot = input.emitted;
            }
        }
        out
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AppConfig {
    pub active_profile: usize,
    pub profiles: Vec<Profile>,
    pub launch_at_login: bool, // default true
    pub show_menubar: bool,    // default true
}

impl Default for AppConfig {
    fn default() -> Self {
        AppConfig {
            active_profile: 0,
            profiles: vec![default_codex_profile()],
            launch_at_login: true,
            show_menubar: true,
        }
    }
}

// ---------------------------------------------------------------------------
// Default "Codex" profile.
// ---------------------------------------------------------------------------

fn keyboard_input(label: &str, icon: &str, mods: u8, code: u16) -> InputConfig {
    InputConfig {
        label: label.to_string(),
        icon: icon.to_string(),
        emitted: Slot { kind: SlotKind::Keyboard, mods, code },
        action: Action::None,
    }
}

fn consumer_input(label: &str, icon: &str, code: u16) -> InputConfig {
    InputConfig {
        label: label.to_string(),
        icon: icon.to_string(),
        emitted: Slot { kind: SlotKind::Consumer, mods: 0, code },
        action: Action::None,
    }
}

fn unbound_input() -> InputConfig {
    InputConfig {
        label: String::new(),
        icon: String::new(),
        emitted: Slot::default(),
        action: Action::None,
    }
}

/// The out-of-the-box profile, mirroring the printed Codex keycaps. Icons are
/// Lucide glyph names (see lucide.rs).
pub fn default_codex_profile() -> Profile {
    // Physical keys p0..p12, top-left to bottom-right. p10 and p11 sit under
    // the shared 2U MIC keycap but are separate switches, so both get MIC.
    const KEYS: [(&str, &str); KEY_SLOTS] = [
        ("FAST", "zap"),
        ("APPR", "check"),
        ("REJ", "x"),
        ("SPLIT", "split"),
        ("NEW", "message-square-plus"),
        ("TERM", "square-terminal"),
        ("PLAY", "play"),
        ("GIT", "folder-git-2"),
        ("PR", "git-pull-request-create-arrow"),
        ("DIFF", "git-compare-arrows"),
        ("MIC", "mic"),
        ("MIC", "mic"),
        ("SETUP", "settings"),
    ];

    let mut inputs: Vec<InputConfig> = KEYS
        .iter()
        .enumerate()
        .map(|(i, &(label, icon))| {
            // p0..p7 emit plain F13..F20 (0x68..0x6F); p8..p12 wrap back to
            // F13..F17 with LShift held. Shifted or not, F13+ never types
            // visible text and can be grabbed by a global hotkey on macOS,
            // Windows and Linux — every key is interceptable by construction.
            let (mods, code) = if i < 8 {
                (0x00, 0x68 + i as u16)
            } else {
                (0x02, 0x68 + (i as u16 - 8))
            };
            keyboard_input(label, icon, mods, code)
        })
        .collect();
    // SETUP opens this app's settings sheet (the PRD's one hardwired action).
    inputs[12].action = Action::AppSettings;

    // Encoder: system volume, handled OS-side via consumer usages — works
    // with the app closed.
    inputs.push(consumer_input("Vol +", "volume-2", 0xE9));
    inputs.push(consumer_input("Vol −", "volume-2", 0xEA));
    inputs.push(consumer_input("Mute", "volume-2", 0xE2));
    // Joystick: arrow keys + Enter, so it navigates anything focusable.
    inputs.push(keyboard_input("Up", "gamepad-2", 0, 0x52));
    inputs.push(keyboard_input("Down", "gamepad-2", 0, 0x51));
    inputs.push(keyboard_input("Left", "gamepad-2", 0, 0x50));
    inputs.push(keyboard_input("Right", "gamepad-2", 0, 0x4F));
    inputs.push(keyboard_input("Enter", "gamepad-2", 0, 0x28));
    // Touch pad: tap toggles playback. Swipe slots exist so configs stay
    // stable when multi-zone hardware lands; unbound until then.
    inputs.push(consumer_input("Play/Pause", "play", 0xCD));
    inputs.push(unbound_input());
    inputs.push(unbound_input());

    debug_assert_eq!(inputs.len(), SLOT_COUNT);
    Profile {
        name: "Codex".to_string(),
        inputs,
        analog: AnalogTuning::default(),
    }
}

// ---------------------------------------------------------------------------
// Sanitizing — every AppConfig that enters the app goes through here.
// ---------------------------------------------------------------------------

/// Enforce the invariants the rest of the app relies on: at least one
/// profile, exactly SLOT_COUNT inputs per profile (missing slots filled from
/// the Codex defaults so labels stay sensible), active_profile in range.
fn sanitize(cfg: &mut AppConfig) {
    if cfg.profiles.is_empty() {
        cfg.profiles.push(default_codex_profile());
    }
    for profile in &mut cfg.profiles {
        profile.inputs.truncate(SLOT_COUNT);
        if profile.inputs.len() < SLOT_COUNT {
            let defaults = default_codex_profile();
            while profile.inputs.len() < SLOT_COUNT {
                profile.inputs.push(defaults.inputs[profile.inputs.len()].clone());
            }
        }
        // The firmware clamps SET_ANALOG to this range; clamping here keeps
        // app truth and device truth from fighting (an out-of-range value
        // would re-sync forever because the device reads back different).
        profile.analog.joy_threshold = profile.analog.joy_threshold.clamp(200, 1900);
    }
    if cfg.active_profile >= cfg.profiles.len() {
        cfg.active_profile = cfg.profiles.len() - 1;
    }
}

// ---------------------------------------------------------------------------
// Legacy schema (pre-profiles) and its migration.
// ---------------------------------------------------------------------------

/// The old on-disk schema: 12 `{kind, arg}` bindings indexed by emitted
/// usage F13..F24 — the 2U pair was one entry (10) and the far-right key
/// was entry 11. Kept only so `load()` can migrate existing installs.
#[derive(Deserialize)]
struct LegacyConfig {
    bindings: Vec<LegacyBinding>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct LegacyBinding {
    kind: LegacyKind,
    arg: String,
}

#[derive(Deserialize, Clone, Copy, Default)]
#[serde(rename_all = "snake_case")]
enum LegacyKind {
    #[default]
    None,
    Run,
    Open,
}

fn migrate_legacy(legacy: LegacyConfig) -> AppConfig {
    let mut profile = default_codex_profile();
    for slot in 0..KEY_SLOTS {
        // Old index: 0..=9 map 1:1; entry 10 drove both switches under the
        // 2U keycap (today's slots 10 and 11); entry 11 was the far-right
        // key, now slot 12.
        let old_index = match slot {
            0..=9 => slot,
            10 | 11 => 10,
            _ => 11,
        };
        let Some(binding) = legacy.bindings.get(old_index) else {
            continue;
        };
        profile.inputs[slot].action = match binding.kind {
            // No old binding: keep the Codex default (notably AppSettings on
            // the SETUP key), which the old app hardwired outside the config.
            LegacyKind::None => continue,
            LegacyKind::Run => Action::Run { command: binding.arg.clone() },
            LegacyKind::Open => Action::Open { target: binding.arg.clone() },
        };
    }
    AppConfig {
        active_profile: 0,
        profiles: vec![profile],
        launch_at_login: true,
        show_menubar: true,
    }
}

/// Parse either schema: current first, then legacy (its required `bindings`
/// field means a current-schema file can never false-positive as legacy).
fn parse_any(text: &str) -> Option<AppConfig> {
    if let Ok(cfg) = serde_json::from_str::<AppConfig>(text) {
        return Some(cfg);
    }
    let legacy: LegacyConfig = serde_json::from_str(text).ok()?;
    Some(migrate_legacy(legacy))
}

// ---------------------------------------------------------------------------
// Load / save.
// ---------------------------------------------------------------------------

fn config_path() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("OpenMicro").join("config.json"))
}

/// Load the config, migrating the legacy schema if that's what's on disk.
/// Never fails — but a file that EXISTS and cannot be parsed is first copied
/// aside to `config.json.invalid` before defaults take over, because the app
/// saves on every edit and on quit: without the backup, one corrupt read
/// would silently overwrite whatever the user had.
pub fn load() -> AppConfig {
    let mut cfg = AppConfig::default();
    if let Some(path) = config_path() {
        if let Ok(text) = std::fs::read_to_string(&path) {
            match parse_any(&text) {
                Some(parsed) => cfg = parsed,
                None => {
                    let backup = path.with_extension("json.invalid");
                    match std::fs::write(&backup, &text) {
                        Ok(()) => eprintln!(
                            "config: {} is unreadable — preserved at {}, using defaults",
                            path.display(),
                            backup.display()
                        ),
                        Err(e) => eprintln!(
                            "config: {} is unreadable AND could not be backed up ({e}) — using defaults",
                            path.display()
                        ),
                    }
                }
            }
        }
    }
    sanitize(&mut cfg);
    cfg
}

pub fn save(cfg: &AppConfig) -> Result<(), String> {
    let path = config_path().ok_or("no config dir on this platform")?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let text = serde_json::to_string_pretty(cfg).map_err(|e| e.to_string())?;
    std::fs::write(&path, text).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Export / import / reset.
// ---------------------------------------------------------------------------

/// Write the whole config as pretty JSON — the same shape `import_from`
/// (and `load`) accept.
pub fn export_to(path: &Path, cfg: &AppConfig) -> Result<(), String> {
    let text = serde_json::to_string_pretty(cfg).map_err(|e| e.to_string())?;
    std::fs::write(path, text).map_err(|e| format!("write {}: {e}", path.display()))
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ImportMode {
    Replace,
    Merge,
}

/// Import a config file. `Replace` swaps in the file wholesale; `Merge`
/// appends its profiles (renamed on name collision) and keeps the current
/// active profile. Returns a one-line human summary for the UI.
pub fn import_from(path: &Path, mode: ImportMode, into: &mut AppConfig) -> Result<String, String> {
    let text =
        std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let mut imported = parse_any(&text)
        .ok_or_else(|| format!("{} is not an OpenMicro config file", path.display()))?;
    sanitize(&mut imported);

    match mode {
        ImportMode::Replace => {
            let n = imported.profiles.len();
            *into = imported;
            Ok(format!("Replaced configuration ({n} profile{})", plural(n)))
        }
        ImportMode::Merge => {
            let n = imported.profiles.len();
            let mut renamed = 0;
            for mut profile in imported.profiles {
                if into.profiles.iter().any(|p| p.name == profile.name) {
                    profile.name = unique_name(&into.profiles, &profile.name);
                    renamed += 1;
                }
                into.profiles.push(profile);
            }
            let mut summary = format!("Added {n} profile{}", plural(n));
            if renamed > 0 {
                summary.push_str(&format!(" ({renamed} renamed to avoid a name clash)"));
            }
            Ok(summary)
        }
    }
}

/// First free variant of `base`: "base (imported)", then "base (imported 2)"…
/// Checked against `existing` each time, so batch imports can't collide.
fn unique_name(existing: &[Profile], base: &str) -> String {
    let taken = |name: &str| existing.iter().any(|p| p.name == name);
    let candidate = format!("{base} (imported)");
    if !taken(&candidate) {
        return candidate;
    }
    let mut n = 2u32;
    loop {
        let candidate = format!("{base} (imported {n})");
        if !taken(&candidate) {
            return candidate;
        }
        n += 1;
    }
}

fn plural(n: usize) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

/// Discard everything and return to the out-of-the-box config. The caller
/// decides when to `save()` and when to re-sync the device.
pub fn factory_reset(cfg: &mut AppConfig) {
    *cfg = AppConfig::default();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_profile_shape() {
        let p = default_codex_profile();
        assert_eq!(p.inputs.len(), SLOT_COUNT);
        assert_eq!(p.name, "Codex");
        // p0 = FAST/F13 plain, p8 = PR/F13 shifted, p12 = SETUP -> AppSettings.
        assert_eq!(p.inputs[0].emitted, Slot { kind: SlotKind::Keyboard, mods: 0, code: 0x68 });
        assert_eq!(p.inputs[8].emitted, Slot { kind: SlotKind::Keyboard, mods: 0x02, code: 0x68 });
        assert_eq!(p.inputs[12].action, Action::AppSettings);
        // All 13 key slots emit distinct (mods, code) pairs — even the two
        // switches under the 2U MIC keycap, so the host can tell them apart.
        for a in 0..KEY_SLOTS {
            for b in a + 1..KEY_SLOTS {
                assert_ne!(p.inputs[a].emitted, p.inputs[b].emitted, "slots {a}/{b}");
            }
        }
        assert_eq!(p.inputs[SLOT_TOUCH_TAP].emitted.code, 0xCD);
        assert_eq!(p.inputs[SLOT_TOUCH_SWIPE_L].emitted.kind, SlotKind::None);
        assert_eq!(p.analog.joy_threshold, 1024);
        assert_eq!(p.slots()[SLOT_ENC_CW], Slot { kind: SlotKind::Consumer, mods: 0, code: 0xE9 });
    }

    #[test]
    fn legacy_migration() {
        let old = r#"{ "bindings": [
            {"kind":"run","arg":"echo hi"},
            {"kind":"none","arg":""},
            {"kind":"open","arg":"https://example.com"},
            {"kind":"none","arg":""}, {"kind":"none","arg":""}, {"kind":"none","arg":""},
            {"kind":"none","arg":""}, {"kind":"none","arg":""}, {"kind":"none","arg":""},
            {"kind":"none","arg":""},
            {"kind":"open","arg":"raycast://"},
            {"kind":"run","arg":"say done"}
        ]}"#;
        let mut cfg = parse_any(old).expect("legacy parses");
        sanitize(&mut cfg);
        let inputs = &cfg.profiles[0].inputs;
        assert_eq!(inputs[0].action, Action::Run { command: "echo hi".into() });
        assert_eq!(inputs[1].action, Action::None);
        assert_eq!(inputs[2].action, Action::Open { target: "https://example.com".into() });
        // Old entry 10 (2U pair) lands on both slots 10 and 11; old 11 -> slot 12.
        assert_eq!(inputs[10].action, Action::Open { target: "raycast://".into() });
        assert_eq!(inputs[11].action, Action::Open { target: "raycast://".into() });
        assert_eq!(inputs[12].action, Action::Run { command: "say done".into() });
        // Emitted codes come from the new defaults, not the legacy file.
        assert_eq!(inputs[0].emitted.code, 0x68);
        assert!(cfg.launch_at_login && cfg.show_menubar);
    }

    #[test]
    fn legacy_none_keeps_setup_default() {
        let old = r#"{ "bindings": [] }"#;
        let cfg = parse_any(old).expect("legacy parses");
        assert_eq!(cfg.profiles[0].inputs[12].action, Action::AppSettings);
    }

    #[test]
    fn sanitize_clamps_and_pads() {
        let mut cfg = AppConfig {
            active_profile: 7,
            profiles: vec![Profile { name: "short".into(), inputs: vec![], analog: AnalogTuning::default() }],
            launch_at_login: false,
            show_menubar: false,
        };
        sanitize(&mut cfg);
        assert_eq!(cfg.active_profile, 0);
        assert_eq!(cfg.profiles[0].inputs.len(), SLOT_COUNT);

        cfg.profiles.clear();
        sanitize(&mut cfg);
        assert_eq!(cfg.profiles.len(), 1);
    }

    #[test]
    fn roundtrip_and_merge() {
        let cfg = AppConfig::default();
        let text = serde_json::to_string_pretty(&cfg).unwrap();
        let back = parse_any(&text).expect("new schema roundtrips");
        assert_eq!(back.profiles, cfg.profiles);

        let mut into = AppConfig::default();
        let name = unique_name(&into.profiles, "Codex");
        assert_eq!(name, "Codex (imported)");
        into.profiles.push(Profile { name, ..default_codex_profile() });
        assert_eq!(unique_name(&into.profiles, "Codex"), "Codex (imported 2)");
    }
}
