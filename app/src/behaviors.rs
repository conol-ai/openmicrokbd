//! User-facing key behaviors and their execution mappings.
//!
//! The editor talks in terms of application shortcuts, macOS controls,
//! keystrokes, and apps. This module translates those choices into the
//! existing device slot plus optional host action without leaking that split
//! into the UI.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use crate::config::{Action, ControlBehavior, InputConfig, MacOsControl, Profile, Slot, SlotKind};
use crate::keycodes::{keyboard_name, mods_label};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShortcutPreset {
    pub id: &'static str,
    pub label: &'static str,
    pub mods: u8,
    pub key: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShortcutApplication {
    pub id: &'static str,
    pub label: &'static str,
    pub shortcuts: &'static [ShortcutPreset],
}

const FINDER_SHORTCUTS: &[ShortcutPreset] = &[
    shortcut("new_window", "New Finder window", 0x08, 0x11),
    shortcut("new_folder", "New folder", 0x0A, 0x11),
    shortcut("go_to_folder", "Go to folder", 0x0A, 0x0A),
    shortcut("get_info", "Get info", 0x08, 0x0C),
    shortcut("quick_look", "Quick Look", 0x00, 0x2C),
    shortcut("move_to_trash", "Move to Trash", 0x08, 0x2A),
];

const SAFARI_SHORTCUTS: &[ShortcutPreset] = &[
    shortcut("new_tab", "New tab", 0x08, 0x17),
    shortcut("close_tab", "Close tab", 0x08, 0x1A),
    shortcut("reopen_tab", "Reopen last closed tab", 0x0A, 0x17),
    shortcut("address", "Focus address bar", 0x08, 0x0F),
    shortcut("downloads", "Show downloads", 0x0C, 0x0F),
];

const CHROME_SHORTCUTS: &[ShortcutPreset] = &[
    shortcut("new_tab", "New tab", 0x08, 0x17),
    shortcut("close_tab", "Close tab", 0x08, 0x1A),
    shortcut("reopen_tab", "Reopen last closed tab", 0x0A, 0x17),
    shortcut("address", "Focus address bar", 0x08, 0x0F),
    shortcut("incognito", "New incognito window", 0x0A, 0x11),
];

const VSCODE_SHORTCUTS: &[ShortcutPreset] = &[
    shortcut("command_palette", "Command Palette", 0x0A, 0x13),
    shortcut("quick_open", "Quick Open", 0x08, 0x13),
    shortcut("new_window", "New window", 0x0A, 0x11),
    shortcut("toggle_terminal", "Toggle terminal", 0x01, 0x35),
    shortcut("find_in_files", "Find in files", 0x0A, 0x09),
];

const XCODE_SHORTCUTS: &[ShortcutPreset] = &[
    shortcut("build", "Build", 0x08, 0x05),
    shortcut("run", "Run", 0x08, 0x15),
    shortcut("test", "Test", 0x08, 0x18),
    shortcut("stop", "Stop", 0x08, 0x37),
    shortcut("open_quickly", "Open Quickly", 0x0A, 0x12),
];

const TERMINAL_SHORTCUTS: &[ShortcutPreset] = &[
    shortcut("new_window", "New window", 0x08, 0x11),
    shortcut("new_tab", "New tab", 0x08, 0x17),
    shortcut("clear", "Clear", 0x08, 0x0E),
    shortcut("close", "Close", 0x08, 0x1A),
    shortcut("find", "Find", 0x08, 0x09),
];

const SLACK_SHORTCUTS: &[ShortcutPreset] = &[
    shortcut("quick_switcher", "Quick switcher", 0x08, 0x0E),
    shortcut("preferences", "Preferences", 0x08, 0x36),
    shortcut("threads", "Threads", 0x0A, 0x17),
    shortcut("history_back", "Previous page", 0x08, 0x2F),
    shortcut("history_forward", "Next page", 0x08, 0x30),
];

const FIGMA_SHORTCUTS: &[ShortcutPreset] = &[
    shortcut("quick_actions", "Quick actions", 0x08, 0x38),
    shortcut("frame", "Frame tool", 0x00, 0x09),
    shortcut("pen", "Pen tool", 0x00, 0x13),
    shortcut("text", "Text tool", 0x00, 0x17),
    shortcut("components", "Show components", 0x0C, 0x0E),
];

const ZOOM_SHORTCUTS: &[ShortcutPreset] = &[
    shortcut("mute", "Mute or unmute", 0x0A, 0x04),
    shortcut("video", "Start or stop video", 0x0A, 0x19),
    shortcut("share", "Start or stop screen share", 0x0A, 0x16),
    shortcut("participants", "Show participants", 0x08, 0x18),
    shortcut("chat", "Show meeting chat", 0x0A, 0x0B),
];

pub const APPLICATION_SHORTCUTS: &[ShortcutApplication] = &[
    ShortcutApplication {
        id: "finder",
        label: "Finder",
        shortcuts: FINDER_SHORTCUTS,
    },
    ShortcutApplication {
        id: "safari",
        label: "Safari",
        shortcuts: SAFARI_SHORTCUTS,
    },
    ShortcutApplication {
        id: "chrome",
        label: "Google Chrome",
        shortcuts: CHROME_SHORTCUTS,
    },
    ShortcutApplication {
        id: "vscode",
        label: "Visual Studio Code",
        shortcuts: VSCODE_SHORTCUTS,
    },
    ShortcutApplication {
        id: "xcode",
        label: "Xcode",
        shortcuts: XCODE_SHORTCUTS,
    },
    ShortcutApplication {
        id: "terminal",
        label: "Terminal",
        shortcuts: TERMINAL_SHORTCUTS,
    },
    ShortcutApplication {
        id: "slack",
        label: "Slack",
        shortcuts: SLACK_SHORTCUTS,
    },
    ShortcutApplication {
        id: "figma",
        label: "Figma",
        shortcuts: FIGMA_SHORTCUTS,
    },
    ShortcutApplication {
        id: "zoom",
        label: "Zoom",
        shortcuts: ZOOM_SHORTCUTS,
    },
];

const fn shortcut(id: &'static str, label: &'static str, mods: u8, key: u16) -> ShortcutPreset {
    ShortcutPreset {
        id,
        label,
        mods,
        key,
    }
}

pub fn shortcut_application(id: &str) -> Option<&'static ShortcutApplication> {
    APPLICATION_SHORTCUTS.iter().find(|app| app.id == id)
}

pub fn shortcut_preset(application: &str, id: &str) -> Option<&'static ShortcutPreset> {
    shortcut_application(application)?
        .shortcuts
        .iter()
        .find(|shortcut| shortcut.id == id)
}

pub fn shortcut_chord_label(shortcut: &ShortcutPreset) -> String {
    let mods = mods_label(shortcut.mods);
    let key = keyboard_name(shortcut.key).unwrap_or("Unknown key");
    if mods.is_empty() {
        key.to_string()
    } else {
        format!("{mods} + {key}")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MacOsPreset {
    pub command: MacOsControl,
    pub label: &'static str,
    pub detail: &'static str,
}

pub const MACOS_PRESETS: &[MacOsPreset] = &[
    macos(
        MacOsControl::BrightnessUp,
        "Brightness up",
        "Increase the built-in display brightness.",
    ),
    macos(
        MacOsControl::BrightnessDown,
        "Brightness down",
        "Decrease the built-in display brightness.",
    ),
    macos(
        MacOsControl::MissionControl,
        "Mission Control",
        "Show all open windows and spaces.",
    ),
    macos(
        MacOsControl::Applications,
        "Applications / Launchpad",
        "Show the macOS applications view.",
    ),
    macos(
        MacOsControl::Search,
        "Spotlight Search",
        "Open or close Spotlight.",
    ),
    macos(
        MacOsControl::Dictation,
        "Dictation",
        "Start or stop macOS Dictation.",
    ),
    macos(
        MacOsControl::Globe,
        "Globe / Fn",
        "Use the native Globe key for input switching and Globe shortcuts.",
    ),
    macos(
        MacOsControl::LockScreen,
        "Lock screen",
        "Lock the current macOS session.",
    ),
    macos(MacOsControl::Sleep, "Sleep", "Put this Mac to sleep."),
    macos(
        MacOsControl::VolumeUp,
        "Volume up",
        "Increase the system output volume.",
    ),
    macos(
        MacOsControl::VolumeDown,
        "Volume down",
        "Decrease the system output volume.",
    ),
    macos(MacOsControl::Mute, "Mute", "Toggle system audio mute."),
    macos(
        MacOsControl::PlayPause,
        "Play / pause",
        "Toggle media playback.",
    ),
    macos(
        MacOsControl::NextTrack,
        "Next track",
        "Skip to the next media item.",
    ),
    macos(
        MacOsControl::PreviousTrack,
        "Previous track",
        "Return to the previous media item.",
    ),
    macos(
        MacOsControl::EmojiPicker,
        "Emoji & symbols",
        "Open or close the character picker.",
    ),
];

const fn macos(command: MacOsControl, label: &'static str, detail: &'static str) -> MacOsPreset {
    MacOsPreset {
        command,
        label,
        detail,
    }
}

pub fn macos_preset(command: MacOsControl) -> &'static MacOsPreset {
    MACOS_PRESETS
        .iter()
        .find(|preset| preset.command == command)
        .unwrap_or(&MACOS_PRESETS[0])
}

/// Apply a curated application chord. Returns false only for stale/unknown
/// catalog IDs, leaving the existing mapping untouched.
pub fn apply_application_shortcut(
    input: &mut InputConfig,
    application: &str,
    shortcut_id: &str,
) -> bool {
    let Some(shortcut) = shortcut_preset(application, shortcut_id) else {
        return false;
    };
    input.behavior = Some(ControlBehavior::ApplicationShortcut {
        application: application.to_string(),
        shortcut: shortcut_id.to_string(),
    });
    input.emitted = keyboard_slot(shortcut.mods, shortcut.key);
    input.action = Action::None;
    true
}

pub fn apply_keystroke(input: &mut InputConfig, mods: u8, key: u16) {
    input.behavior = Some(ControlBehavior::Keystroke);
    input.emitted = keyboard_slot(mods & 0x0f, key);
    input.action = Action::None;
}

pub fn apply_macos(input: &mut InputConfig, slot_index: usize, command: MacOsControl) {
    input.behavior = Some(ControlBehavior::MacOs { command });
    let (emitted, action) = match command {
        MacOsControl::BrightnessUp => (consumer_slot(0x006F), Action::None),
        MacOsControl::BrightnessDown => (consumer_slot(0x0070), Action::None),
        MacOsControl::MissionControl => (keyboard_slot(0x01, 0x52), Action::None),
        MacOsControl::Applications => {
            let target = if Path::new("/System/Applications/Apps.app").exists() {
                "/System/Applications/Apps.app"
            } else {
                "/System/Applications/Launchpad.app"
            };
            (
                host_trigger(slot_index),
                Action::Open {
                    target: target.to_string(),
                },
            )
        }
        // The documented macOS default. Users can remap Spotlight in System
        // Settings, just like any other keyboard shortcut.
        MacOsControl::Search => (keyboard_slot(0x08, 0x2C), Action::None),
        MacOsControl::Dictation => (consumer_slot(0x00D8), Action::None),
        // Apple's accessory keyboard specification assigns the native Globe
        // key to Consumer-page AC Keyboard Layout Select (0x029D).
        MacOsControl::Globe => (consumer_slot(0x029D), Action::None),
        MacOsControl::LockScreen => (keyboard_slot(0x09, 0x14), Action::None),
        MacOsControl::Sleep => (
            host_trigger(slot_index),
            Action::Run {
                command: "pmset sleepnow".to_string(),
            },
        ),
        MacOsControl::VolumeUp => (consumer_slot(0x00E9), Action::None),
        MacOsControl::VolumeDown => (consumer_slot(0x00EA), Action::None),
        MacOsControl::Mute => (consumer_slot(0x00E2), Action::None),
        MacOsControl::PlayPause => (consumer_slot(0x00CD), Action::None),
        MacOsControl::NextTrack => (consumer_slot(0x00B5), Action::None),
        MacOsControl::PreviousTrack => (consumer_slot(0x00B6), Action::None),
        MacOsControl::EmojiPicker => (keyboard_slot(0x09, 0x2C), Action::None),
    };
    input.emitted = emitted;
    input.action = action;
}

pub fn apply_app(input: &mut InputConfig, slot_index: usize, target: String) {
    input.behavior = Some(ControlBehavior::App {
        target: target.clone(),
    });
    input.emitted = host_trigger(slot_index);
    input.action = Action::Open { target };
}

pub fn behavior_is_consistent(input: &InputConfig, slot_index: usize) -> bool {
    let Some(behavior) = input.behavior.clone() else {
        return false;
    };
    let mut expected = input.clone();
    let valid = match behavior {
        ControlBehavior::ApplicationShortcut {
            application,
            shortcut,
        } => apply_application_shortcut(&mut expected, &application, &shortcut),
        ControlBehavior::MacOs { command }
            if matches!(command, MacOsControl::Applications | MacOsControl::Sleep) =>
        {
            apply_macos(&mut expected, slot_index, command);
            return hidden_trigger(input.emitted) && expected.action == input.action;
        }
        ControlBehavior::MacOs { command } => {
            apply_macos(&mut expected, slot_index, command);
            true
        }
        ControlBehavior::Keystroke => {
            return input.emitted.kind == SlotKind::Keyboard && input.action == Action::None;
        }
        ControlBehavior::App { target } => {
            apply_app(&mut expected, slot_index, target);
            return hidden_trigger(input.emitted) && expected.action == input.action;
        }
    };
    valid && expected.emitted == input.emitted && expected.action == input.action
}

/// Keep every host-assisted semantic behavior on a distinct, non-printing
/// chord. A user may freely assign an old hidden chord as a normal keystroke;
/// this allocator moves the hidden trigger instead of letting one press run
/// two behaviors.
pub fn normalize_hidden_triggers(profile: &mut Profile) {
    for slot_index in 0..profile.inputs.len() {
        if !is_host_assisted(&profile.inputs[slot_index]) {
            continue;
        }
        let current = profile.inputs[slot_index].emitted;
        let collision = profile
            .inputs
            .iter()
            .enumerate()
            .any(|(other, input)| other != slot_index && input.emitted == current);
        if hidden_trigger(current) && !collision {
            continue;
        }

        if let Some(candidate) = hidden_trigger_candidates().find(|candidate| {
            profile
                .inputs
                .iter()
                .enumerate()
                .all(|(other, input)| other == slot_index || input.emitted != *candidate)
        }) {
            profile.inputs[slot_index].emitted = candidate;
        }
    }
}

fn is_host_assisted(input: &InputConfig) -> bool {
    matches!(
        input.behavior.as_ref(),
        Some(ControlBehavior::App { .. })
            | Some(ControlBehavior::MacOs {
                command: MacOsControl::Applications | MacOsControl::Sleep
            })
    )
}

fn hidden_trigger(slot: Slot) -> bool {
    slot.kind == SlotKind::Keyboard && (0x68..=0x6F).contains(&slot.code) && slot.mods & 0xF0 == 0
}

fn hidden_trigger_candidates() -> impl Iterator<Item = Slot> {
    const MOD_BANKS: [u8; 16] = [
        0x00, 0x02, 0x03, 0x01, 0x04, 0x08, 0x06, 0x0A, 0x0C, 0x05, 0x09, 0x07, 0x0B, 0x0D, 0x0E,
        0x0F,
    ];
    MOD_BANKS
        .into_iter()
        .flat_map(|mods| (0x68..=0x6F).map(move |code| keyboard_slot(mods, code)))
}

fn keyboard_slot(mods: u8, key: u16) -> Slot {
    Slot {
        kind: SlotKind::Keyboard,
        mods,
        code: key,
    }
}

fn consumer_slot(code: u16) -> Slot {
    Slot {
        kind: SlotKind::Consumer,
        mods: 0,
        code,
    }
}

/// A unique, non-printing chord for host-assisted behavior. There are eight
/// F13–F20 keys in each modifier bank, enough to cover all 24 input slots.
fn host_trigger(slot_index: usize) -> Slot {
    const BANK_MODS: [u8; 3] = [0x00, 0x02, 0x03];
    keyboard_slot(
        BANK_MODS[(slot_index / 8).min(2)],
        0x68 + (slot_index % 8) as u16,
    )
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstalledApp {
    pub name: String,
    pub path: String,
}

/// Discover launchable macOS app bundles from the normal user-facing roots.
/// Exact paths are persisted because two app bundles can share a bundle ID.
pub fn installed_apps() -> Vec<InstalledApp> {
    let mut paths = BTreeSet::new();
    for root in application_roots() {
        collect_app_bundles(&root, 0, 3, &mut paths);
    }

    let mut by_name: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
    for path in paths {
        let Some(stem) = path.file_stem().and_then(|value| value.to_str()) else {
            continue;
        };
        by_name.entry(stem.to_string()).or_default().push(path);
    }

    let mut apps = Vec::new();
    for (name, mut paths) in by_name {
        paths.sort();
        let duplicated = paths.len() > 1;
        for path in paths {
            let display_name = if duplicated {
                let parent = path
                    .parent()
                    .and_then(Path::file_name)
                    .and_then(|value| value.to_str())
                    .unwrap_or("Applications");
                format!("{name} — {parent}")
            } else {
                name.clone()
            };
            apps.push(InstalledApp {
                name: display_name,
                path: path.to_string_lossy().into_owned(),
            });
        }
    }
    apps
}

fn application_roots() -> Vec<PathBuf> {
    let mut roots = vec![
        PathBuf::from("/Applications"),
        PathBuf::from("/System/Applications"),
    ];
    if let Some(home) = dirs::home_dir() {
        roots.push(home.join("Applications"));
    }
    roots
}

fn collect_app_bundles(
    directory: &Path,
    depth: usize,
    max_depth: usize,
    found: &mut BTreeSet<PathBuf>,
) {
    if depth > max_depth {
        return;
    }
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_dir() || file_type.is_symlink() {
            continue;
        }
        let is_app = path
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("app"));
        if is_app {
            found.insert(path);
        } else if depth < max_depth {
            collect_app_bundles(&path, depth + 1, max_depth, found);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::default_codex_profile;

    fn sample_input() -> InputConfig {
        InputConfig {
            label: "Key".to_string(),
            icon: "star".to_string(),
            behavior: None,
            emitted: Slot::default(),
            action: Action::None,
        }
    }

    #[test]
    fn application_shortcut_compiles_to_one_saved_chord() {
        let mut input = sample_input();
        assert!(apply_application_shortcut(
            &mut input,
            "vscode",
            "command_palette"
        ));
        assert_eq!(
            input.emitted,
            Slot {
                kind: SlotKind::Keyboard,
                mods: 0x0A,
                code: 0x13,
            }
        );
        assert_eq!(input.action, Action::None);
        assert_eq!(
            input.behavior,
            Some(ControlBehavior::ApplicationShortcut {
                application: "vscode".to_string(),
                shortcut: "command_palette".to_string(),
            })
        );
    }

    #[test]
    fn host_app_behavior_gets_a_unique_non_printing_trigger() {
        let mut first = sample_input();
        let mut touch = sample_input();
        apply_app(&mut first, 0, "/Applications/Finder.app".to_string());
        apply_app(&mut touch, 21, "/Applications/Music.app".to_string());
        assert_ne!(first.emitted, touch.emitted);
        assert_eq!(first.emitted.kind, SlotKind::Keyboard);
        assert_eq!(touch.emitted.kind, SlotKind::Keyboard);
    }

    #[test]
    fn macos_sleep_keeps_semantics_above_its_execution_mapping() {
        let mut input = sample_input();
        apply_macos(&mut input, 4, MacOsControl::Sleep);
        assert_eq!(
            input.behavior,
            Some(ControlBehavior::MacOs {
                command: MacOsControl::Sleep
            })
        );
        assert!(matches!(input.action, Action::Run { .. }));
    }

    #[test]
    fn macos_globe_uses_apples_native_consumer_usage() {
        let mut input = sample_input();
        apply_macos(&mut input, 4, MacOsControl::Globe);
        assert_eq!(
            input.emitted,
            Slot {
                kind: SlotKind::Consumer,
                mods: 0,
                code: 0x029D,
            }
        );
        assert_eq!(input.action, Action::None);
        assert_eq!(
            input.behavior,
            Some(ControlBehavior::MacOs {
                command: MacOsControl::Globe
            })
        );
        assert!(behavior_is_consistent(&input, 4));
    }

    #[test]
    fn direct_chord_collision_reallocates_only_the_hidden_trigger() {
        let mut profile = default_codex_profile();
        apply_app(
            &mut profile.inputs[0],
            0,
            "/Applications/Finder.app".to_string(),
        );
        let hidden_before = profile.inputs[0].emitted;
        apply_keystroke(
            &mut profile.inputs[1],
            hidden_before.mods,
            hidden_before.code,
        );
        let direct = profile.inputs[1].emitted;

        normalize_hidden_triggers(&mut profile);

        assert_eq!(profile.inputs[1].emitted, direct);
        assert_ne!(profile.inputs[0].emitted, direct);
        assert!(behavior_is_consistent(&profile.inputs[0], 0));
        assert!(behavior_is_consistent(&profile.inputs[1], 1));
    }
}
