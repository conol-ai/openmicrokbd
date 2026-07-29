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
    Keystroke {
        mods: u8,
        key: u16,
    },
    Macro {
        steps: Vec<MacroStepEntry>,
    },
    Run {
        command: String,
    },
    Open {
        target: String,
    },
    Media {
        op: MediaOp,
    },
    AppSettings, // open this app's settings sheet (the SETUP key default)
}

/// The user-facing meaning of a key or touch-pad tap.
///
/// `emitted` and `action` remain the execution representation used by the
/// device and host. This semantic value lets the editor present one coherent
/// behavior without exposing that implementation split. It is optional so
/// configurations written by older versions keep working unchanged.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ControlBehavior {
    ApplicationShortcut {
        application: String,
        shortcut: String,
    },
    MacOs {
        command: MacOsControl,
    },
    Keystroke,
    App {
        target: String,
    },
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MacOsControl {
    BrightnessUp,
    BrightnessDown,
    MissionControl,
    Applications,
    Search,
    Dictation,
    Globe,
    LockScreen,
    Sleep,
    VolumeUp,
    VolumeDown,
    Mute,
    PlayPause,
    NextTrack,
    PreviousTrack,
    EmojiPicker,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct InputConfig {
    pub label: String,
    pub icon: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub behavior: Option<ControlBehavior>,
    pub emitted: Slot,
    pub action: Action,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct AnalogTuning {
    pub joy_threshold: u16, // ADC deviation from centre 2048; default 1024
}

impl Default for AnalogTuning {
    fn default() -> Self {
        AnalogTuning {
            joy_threshold: 1024,
        }
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

/// The paired hardware behaviours offered for the rotary encoder.
///
/// The device protocol still stores clockwise and counter-clockwise as two
/// independent slots. This semantic layer deliberately treats them as one
/// setting so the UI cannot accidentally configure a one-sided rotation.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RotatorRotationPreset {
    Volume,
    Brightness,
    Tracks,
    VerticalArrows,
    HorizontalArrows,
}

impl RotatorRotationPreset {
    pub const ALL: [Self; 5] = [
        Self::Volume,
        Self::Brightness,
        Self::Tracks,
        Self::VerticalArrows,
        Self::HorizontalArrows,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Volume => "Volume",
            Self::Brightness => "Screen brightness",
            Self::Tracks => "Track selection",
            Self::VerticalArrows => "Arrow keys · up / down",
            Self::HorizontalArrows => "Arrow keys · left / right",
        }
    }

    pub const fn detail(self) -> &'static str {
        match self {
            Self::Volume => "Clockwise raises · counter-clockwise lowers",
            Self::Brightness => "Clockwise brightens · counter-clockwise dims",
            Self::Tracks => "Clockwise next · counter-clockwise previous",
            Self::VerticalArrows => "Clockwise down · counter-clockwise up",
            Self::HorizontalArrows => "Clockwise right · counter-clockwise left",
        }
    }

    const fn slots(self) -> (Slot, Slot) {
        match self {
            Self::Volume => (
                Slot {
                    kind: SlotKind::Consumer,
                    mods: 0,
                    code: 0xE9,
                },
                Slot {
                    kind: SlotKind::Consumer,
                    mods: 0,
                    code: 0xEA,
                },
            ),
            Self::Brightness => (
                Slot {
                    kind: SlotKind::Consumer,
                    mods: 0,
                    code: 0x6F,
                },
                Slot {
                    kind: SlotKind::Consumer,
                    mods: 0,
                    code: 0x70,
                },
            ),
            Self::Tracks => (
                Slot {
                    kind: SlotKind::Consumer,
                    mods: 0,
                    code: 0xB5,
                },
                Slot {
                    kind: SlotKind::Consumer,
                    mods: 0,
                    code: 0xB6,
                },
            ),
            Self::VerticalArrows => (
                Slot {
                    kind: SlotKind::Keyboard,
                    mods: 0,
                    code: 0x51,
                },
                Slot {
                    kind: SlotKind::Keyboard,
                    mods: 0,
                    code: 0x52,
                },
            ),
            Self::HorizontalArrows => (
                Slot {
                    kind: SlotKind::Keyboard,
                    mods: 0,
                    code: 0x4F,
                },
                Slot {
                    kind: SlotKind::Keyboard,
                    mods: 0,
                    code: 0x50,
                },
            ),
        }
    }

    /// Recognise only the complete preset: both emitted usages and both
    /// no-host-action states must match. Imported/custom pairs remain custom.
    pub fn infer(profile: &Profile) -> Option<Self> {
        let cw = profile.inputs.get(SLOT_ENC_CW)?;
        let ccw = profile.inputs.get(SLOT_ENC_CCW)?;
        Self::ALL.into_iter().find(|preset| {
            let (cw_slot, ccw_slot) = preset.slots();
            is_direct_slot(cw, cw_slot) && is_direct_slot(ccw, ccw_slot)
        })
    }

    /// Apply the paired preset atomically at the profile level. No other
    /// rotator input (notably its press slot) is touched.
    pub fn apply_to(self, profile: &mut Profile) {
        if profile.inputs.len() <= SLOT_ENC_CCW {
            return;
        }
        let (cw, ccw) = match self {
            Self::Volume => (
                consumer_input("Vol +", "volume-2", 0xE9),
                consumer_input("Vol −", "volume-1", 0xEA),
            ),
            Self::Brightness => (
                consumer_input("Bright +", "sun", 0x6F),
                consumer_input("Bright −", "sun", 0x70),
            ),
            Self::Tracks => (
                consumer_input("Next", "skip-forward", 0xB5),
                consumer_input("Previous", "skip-back", 0xB6),
            ),
            Self::VerticalArrows => (
                keyboard_input("Down", "arrow-down", 0, 0x51),
                keyboard_input("Up", "arrow-up", 0, 0x52),
            ),
            Self::HorizontalArrows => (
                keyboard_input("Right", "arrow-right", 0, 0x4F),
                keyboard_input("Left", "arrow-left", 0, 0x50),
            ),
        };
        profile.inputs[SLOT_ENC_CW] = cw;
        profile.inputs[SLOT_ENC_CCW] = ccw;
    }
}

/// The hardware behaviour offered for pressing the rotary encoder.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RotatorPressPreset {
    Mute,
    LockScreen,
    Play,
    Enter,
}

impl RotatorPressPreset {
    pub const ALL: [Self; 4] = [Self::Mute, Self::LockScreen, Self::Play, Self::Enter];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Mute => "Mute",
            Self::LockScreen => "Lock screen",
            Self::Play => "Play",
            Self::Enter => "Enter",
        }
    }

    pub const fn detail(self) -> &'static str {
        match self {
            Self::Mute => "Toggle system audio mute",
            Self::LockScreen => "Lock the computer or start its screen saver",
            Self::Play => "Start media playback",
            Self::Enter => "Send the Enter key",
        }
    }

    const fn slot(self) -> Slot {
        match self {
            Self::Mute => Slot {
                kind: SlotKind::Consumer,
                mods: 0,
                code: 0xE2,
            },
            // USB HID Consumer page: AL Terminal Lock/Screensaver.
            Self::LockScreen => Slot {
                kind: SlotKind::Consumer,
                mods: 0,
                code: 0x019E,
            },
            // Literal Play, rather than the separate Play/Pause toggle (0xCD).
            Self::Play => Slot {
                kind: SlotKind::Consumer,
                mods: 0,
                code: 0xB0,
            },
            Self::Enter => Slot {
                kind: SlotKind::Keyboard,
                mods: 0,
                code: 0x28,
            },
        }
    }

    /// Recognise only a direct device usage with no stale host action.
    pub fn infer(profile: &Profile) -> Option<Self> {
        let press = profile.inputs.get(SLOT_ENC_PRESS)?;
        Self::ALL
            .into_iter()
            .find(|preset| is_direct_slot(press, preset.slot()))
    }

    /// Apply only the encoder press preset; the rotation pair is untouched.
    pub fn apply_to(self, profile: &mut Profile) {
        let Some(press) = profile.inputs.get_mut(SLOT_ENC_PRESS) else {
            return;
        };
        *press = match self {
            Self::Mute => consumer_input("Mute", "volume-x", 0xE2),
            Self::LockScreen => consumer_input("Lock", "lock", 0x019E),
            Self::Play => consumer_input("Play", "play", 0xB0),
            Self::Enter => keyboard_input("Enter", "corner-down-left", 0, 0x28),
        };
    }
}

fn is_direct_slot(input: &InputConfig, emitted: Slot) -> bool {
    input.emitted == emitted && input.action == Action::None
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
        behavior: None,
        emitted: Slot {
            kind: SlotKind::Keyboard,
            mods,
            code,
        },
        action: Action::None,
    }
}

fn consumer_input(label: &str, icon: &str, code: u16) -> InputConfig {
    InputConfig {
        label: label.to_string(),
        icon: icon.to_string(),
        behavior: None,
        emitted: Slot {
            kind: SlotKind::Consumer,
            mods: 0,
            code,
        },
        action: Action::None,
    }
}

fn unbound_input() -> InputConfig {
    InputConfig {
        label: String::new(),
        icon: String::new(),
        behavior: None,
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
    for input in inputs.iter_mut().take(12) {
        input.behavior = Some(ControlBehavior::Keystroke);
    }
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
    let mut touch = consumer_input("Play/Pause", "play", 0xCD);
    touch.behavior = Some(ControlBehavior::MacOs {
        command: MacOsControl::PlayPause,
    });
    inputs.push(touch);
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
                profile
                    .inputs
                    .push(defaults.inputs[profile.inputs.len()].clone());
            }
        }
        // The firmware clamps SET_ANALOG to this range; clamping here keeps
        // app truth and device truth from fighting (an out-of-range value
        // would re-sync forever because the device reads back different).
        profile.analog.joy_threshold = profile.analog.joy_threshold.clamp(200, 1900);

        // Add semantic metadata to older key/touch configurations without
        // changing the compiled slot or action that already works. Ambiguous
        // host automations deliberately remain `None` and appear as an
        // existing setup until the user replaces them.
        for slot in (0..KEY_SLOTS).chain(std::iter::once(SLOT_TOUCH_TAP)) {
            let Some(input) = profile.inputs.get_mut(slot) else {
                continue;
            };
            if input.behavior.is_none() {
                input.behavior = infer_legacy_behavior(input);
            }
        }
    }
    if cfg.active_profile >= cfg.profiles.len() {
        cfg.active_profile = cfg.profiles.len() - 1;
    }
}

fn infer_legacy_behavior(input: &InputConfig) -> Option<ControlBehavior> {
    match (&input.action, input.emitted.kind) {
        (Action::None, SlotKind::Keyboard) => Some(ControlBehavior::Keystroke),
        (Action::None, SlotKind::Consumer) => {
            let command = match input.emitted.code {
                0x006F => MacOsControl::BrightnessUp,
                0x0070 => MacOsControl::BrightnessDown,
                0x029F => MacOsControl::MissionControl,
                0x02A2 => MacOsControl::Applications,
                0x0221 => MacOsControl::Search,
                0x00D8 => MacOsControl::Dictation,
                0x029D => MacOsControl::Globe,
                0x00E9 => MacOsControl::VolumeUp,
                0x00EA => MacOsControl::VolumeDown,
                0x00E2 => MacOsControl::Mute,
                0x00CD => MacOsControl::PlayPause,
                0x00B5 => MacOsControl::NextTrack,
                0x00B6 => MacOsControl::PreviousTrack,
                0x00D9 => MacOsControl::EmojiPicker,
                _ => return None,
            };
            Some(ControlBehavior::MacOs { command })
        }
        (Action::Open { target }, SlotKind::Keyboard)
            if Path::new(target)
                .extension()
                .and_then(|value| value.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("app")) =>
        {
            Some(ControlBehavior::App {
                target: target.clone(),
            })
        }
        _ => None,
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
            LegacyKind::Run => Action::Run {
                command: binding.arg.clone(),
            },
            LegacyKind::Open => Action::Open {
                target: binding.arg.clone(),
            },
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
        assert_eq!(
            p.inputs[0].emitted,
            Slot {
                kind: SlotKind::Keyboard,
                mods: 0,
                code: 0x68
            }
        );
        assert_eq!(
            p.inputs[8].emitted,
            Slot {
                kind: SlotKind::Keyboard,
                mods: 0x02,
                code: 0x68
            }
        );
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
        assert_eq!(
            p.slots()[SLOT_ENC_CW],
            Slot {
                kind: SlotKind::Consumer,
                mods: 0,
                code: 0xE9
            }
        );
    }

    #[test]
    fn all_rotation_presets_apply_as_exact_hid_pairs() {
        assert_eq!(
            RotatorRotationPreset::VerticalArrows.label(),
            "Arrow keys · up / down"
        );
        assert_eq!(
            RotatorRotationPreset::HorizontalArrows.label(),
            "Arrow keys · left / right"
        );
        let cases = [
            (
                RotatorRotationPreset::Volume,
                (SlotKind::Consumer, 0xE9, "Vol +", "volume-2"),
                (SlotKind::Consumer, 0xEA, "Vol −", "volume-1"),
            ),
            (
                RotatorRotationPreset::Brightness,
                (SlotKind::Consumer, 0x6F, "Bright +", "sun"),
                (SlotKind::Consumer, 0x70, "Bright −", "sun"),
            ),
            (
                RotatorRotationPreset::Tracks,
                (SlotKind::Consumer, 0xB5, "Next", "skip-forward"),
                (SlotKind::Consumer, 0xB6, "Previous", "skip-back"),
            ),
            (
                RotatorRotationPreset::VerticalArrows,
                (SlotKind::Keyboard, 0x51, "Down", "arrow-down"),
                (SlotKind::Keyboard, 0x52, "Up", "arrow-up"),
            ),
            (
                RotatorRotationPreset::HorizontalArrows,
                (SlotKind::Keyboard, 0x4F, "Right", "arrow-right"),
                (SlotKind::Keyboard, 0x50, "Left", "arrow-left"),
            ),
        ];
        for (preset, cw_expected, ccw_expected) in cases {
            let mut profile = default_codex_profile();
            let press_before = profile.inputs[SLOT_ENC_PRESS].clone();
            // Applying a preset must replace stale presentation and host action
            // state, not merely swap the emitted usage.
            profile.inputs[SLOT_ENC_CW].label = "custom cw".into();
            profile.inputs[SLOT_ENC_CW].action = Action::Run {
                command: "old".into(),
            };
            profile.inputs[SLOT_ENC_CCW].icon = "custom-icon".into();
            profile.inputs[SLOT_ENC_CCW].action = Action::Media { op: MediaOp::Mute };

            preset.apply_to(&mut profile);

            let cw = &profile.inputs[SLOT_ENC_CW];
            let ccw = &profile.inputs[SLOT_ENC_CCW];
            assert_eq!(
                cw.emitted,
                Slot {
                    kind: cw_expected.0,
                    mods: 0,
                    code: cw_expected.1
                }
            );
            assert_eq!(
                ccw.emitted,
                Slot {
                    kind: ccw_expected.0,
                    mods: 0,
                    code: ccw_expected.1
                }
            );
            assert_eq!((&*cw.label, &*cw.icon), (cw_expected.2, cw_expected.3));
            assert_eq!((&*ccw.label, &*ccw.icon), (ccw_expected.2, ccw_expected.3));
            assert_eq!(cw.action, Action::None);
            assert_eq!(ccw.action, Action::None);
            assert_eq!(profile.inputs[SLOT_ENC_PRESS], press_before);
            assert_eq!(RotatorRotationPreset::infer(&profile), Some(preset));
        }
    }

    #[test]
    fn all_press_presets_apply_as_exact_hid_usages() {
        assert_eq!(
            RotatorPressPreset::ALL,
            [
                RotatorPressPreset::Mute,
                RotatorPressPreset::LockScreen,
                RotatorPressPreset::Play,
                RotatorPressPreset::Enter,
            ]
        );
        assert_eq!(RotatorPressPreset::Enter.label(), "Enter");
        assert_eq!(RotatorPressPreset::Enter.detail(), "Send the Enter key");
        let cases = [
            (
                RotatorPressPreset::Mute,
                SlotKind::Consumer,
                0xE2,
                "Mute",
                "volume-x",
            ),
            (
                RotatorPressPreset::LockScreen,
                SlotKind::Consumer,
                0x019E,
                "Lock",
                "lock",
            ),
            (
                RotatorPressPreset::Play,
                SlotKind::Consumer,
                0xB0,
                "Play",
                "play",
            ),
            (
                RotatorPressPreset::Enter,
                SlotKind::Keyboard,
                0x28,
                "Enter",
                "corner-down-left",
            ),
        ];
        for (preset, kind, code, label, icon) in cases {
            let mut profile = default_codex_profile();
            let rotation_before = [
                profile.inputs[SLOT_ENC_CW].clone(),
                profile.inputs[SLOT_ENC_CCW].clone(),
            ];
            profile.inputs[SLOT_ENC_PRESS].label = "custom press".into();
            profile.inputs[SLOT_ENC_PRESS].action = Action::Open {
                target: "old".into(),
            };

            preset.apply_to(&mut profile);

            let press = &profile.inputs[SLOT_ENC_PRESS];
            assert_eq!(
                press.emitted,
                Slot {
                    kind,
                    mods: 0,
                    code
                }
            );
            assert_eq!((&*press.label, &*press.icon), (label, icon));
            assert_eq!(press.action, Action::None);
            assert_eq!(
                [
                    profile.inputs[SLOT_ENC_CW].clone(),
                    profile.inputs[SLOT_ENC_CCW].clone(),
                ],
                rotation_before
            );
            assert_eq!(RotatorPressPreset::infer(&profile), Some(preset));
        }
    }

    #[test]
    fn custom_rotator_mappings_are_preserved_until_a_preset_is_applied() {
        let mut profile = default_codex_profile();
        // One stale action makes an otherwise familiar pair custom.
        profile.inputs[SLOT_ENC_CCW].action = Action::Run {
            command: "custom".into(),
        };
        // A different emitted kind makes the press custom too.
        profile.inputs[SLOT_ENC_PRESS] = keyboard_input("Custom", "keyboard", 0x08, 0x0F);
        let before_inference = profile.clone();

        assert_eq!(RotatorRotationPreset::infer(&profile), None);
        assert_eq!(RotatorPressPreset::infer(&profile), None);
        assert_eq!(
            profile, before_inference,
            "inference must never normalise custom data"
        );

        let custom_press = profile.inputs[SLOT_ENC_PRESS].clone();
        RotatorRotationPreset::Brightness.apply_to(&mut profile);
        assert_eq!(profile.inputs[SLOT_ENC_PRESS], custom_press);
        assert_eq!(
            RotatorRotationPreset::infer(&profile),
            Some(RotatorRotationPreset::Brightness)
        );

        let custom_rotation = [
            profile.inputs[SLOT_ENC_CW].clone(),
            profile.inputs[SLOT_ENC_CCW].clone(),
        ];
        RotatorPressPreset::LockScreen.apply_to(&mut profile);
        assert_eq!(
            [
                profile.inputs[SLOT_ENC_CW].clone(),
                profile.inputs[SLOT_ENC_CCW].clone(),
            ],
            custom_rotation
        );
        assert_eq!(
            RotatorPressPreset::infer(&profile),
            Some(RotatorPressPreset::LockScreen)
        );

        RotatorRotationPreset::VerticalArrows.apply_to(&mut profile);
        profile.inputs[SLOT_ENC_CW].action = Action::Media { op: MediaOp::Mute };
        assert_eq!(
            RotatorRotationPreset::infer(&profile),
            None,
            "a host action makes even an exact keyboard pair custom"
        );
        assert_eq!(profile.inputs[SLOT_ENC_CW].emitted.kind, SlotKind::Keyboard);
        assert_eq!(profile.inputs[SLOT_ENC_CW].emitted.code, 0x51);
        assert_eq!(profile.inputs[SLOT_ENC_CCW].emitted.code, 0x52);

        RotatorPressPreset::Enter.apply_to(&mut profile);
        profile.inputs[SLOT_ENC_PRESS].action = Action::Run {
            command: "custom enter".into(),
        };
        assert_eq!(
            RotatorPressPreset::infer(&profile),
            None,
            "a stale host action makes the exact Enter usage custom"
        );
        assert_eq!(
            profile.inputs[SLOT_ENC_PRESS].emitted.kind,
            SlotKind::Keyboard
        );
        assert_eq!(profile.inputs[SLOT_ENC_PRESS].emitted.code, 0x28);
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
        assert_eq!(
            inputs[0].action,
            Action::Run {
                command: "echo hi".into()
            }
        );
        assert_eq!(inputs[1].action, Action::None);
        assert_eq!(
            inputs[2].action,
            Action::Open {
                target: "https://example.com".into()
            }
        );
        // Old entry 10 (2U pair) lands on both slots 10 and 11; old 11 -> slot 12.
        assert_eq!(
            inputs[10].action,
            Action::Open {
                target: "raycast://".into()
            }
        );
        assert_eq!(
            inputs[11].action,
            Action::Open {
                target: "raycast://".into()
            }
        );
        assert_eq!(
            inputs[12].action,
            Action::Run {
                command: "say done".into()
            }
        );
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
            profiles: vec![Profile {
                name: "short".into(),
                inputs: vec![],
                analog: AnalogTuning::default(),
            }],
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
    fn current_json_without_behavior_migrates_without_changing_execution() {
        let original = AppConfig::default();
        let mut json = serde_json::to_value(&original).expect("serialize");
        for profile in json["profiles"].as_array_mut().expect("profiles") {
            for input in profile["inputs"].as_array_mut().expect("inputs") {
                input.as_object_mut().expect("input").remove("behavior");
            }
        }
        let mut parsed: AppConfig = serde_json::from_value(json).expect("old current schema");
        let before: Vec<(Slot, Action, String, String)> = parsed.profiles[0]
            .inputs
            .iter()
            .map(|input| {
                (
                    input.emitted,
                    input.action.clone(),
                    input.label.clone(),
                    input.icon.clone(),
                )
            })
            .collect();

        sanitize(&mut parsed);

        let after: Vec<(Slot, Action, String, String)> = parsed.profiles[0]
            .inputs
            .iter()
            .map(|input| {
                (
                    input.emitted,
                    input.action.clone(),
                    input.label.clone(),
                    input.icon.clone(),
                )
            })
            .collect();
        assert_eq!(before, after, "migration must only add advisory metadata");
        assert_eq!(
            parsed.profiles[0].inputs[0].behavior,
            Some(ControlBehavior::Keystroke)
        );
        assert_eq!(
            parsed.profiles[0].inputs[SLOT_TOUCH_TAP].behavior,
            Some(ControlBehavior::MacOs {
                command: MacOsControl::PlayPause
            })
        );
        assert_eq!(
            parsed.profiles[0].inputs[SLOT_TOUCH_SWIPE_L],
            original.profiles[0].inputs[SLOT_TOUCH_SWIPE_L]
        );
        assert_eq!(
            parsed.profiles[0].inputs[SLOT_TOUCH_SWIPE_R],
            original.profiles[0].inputs[SLOT_TOUCH_SWIPE_R]
        );
    }

    #[test]
    fn ambiguous_legacy_automation_stays_an_existing_setup() {
        let mut cfg = AppConfig::default();
        let input = &mut cfg.profiles[0].inputs[0];
        input.behavior = None;
        input.action = Action::Macro {
            steps: vec![MacroStep::Delay { ms: 10 }.into()],
        };
        let before = input.clone();
        sanitize(&mut cfg);
        assert_eq!(cfg.profiles[0].inputs[0], before);
        assert_eq!(cfg.profiles[0].inputs[0].behavior, None);
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
        into.profiles.push(Profile {
            name,
            ..default_codex_profile()
        });
        assert_eq!(unique_name(&into.profiles, "Codex"), "Codex (imported 2)");
    }
}
