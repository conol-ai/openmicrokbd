//! Framework-neutral state transitions used by the GPUI editor.
//!
//! The view layer should only need to choose a direction and redraw. These
//! helpers keep catalog wraparound, default values, and the semantic mapping
//! rules in one place instead of duplicating them in click handlers.

use crate::behaviors::{self, ShortcutApplication, ShortcutPreset};
use crate::config::{
    Action, ControlBehavior, InputConfig, JoystickMode, MacOsControl, MediaOp, Profile,
    RotatorPressPreset, RotatorRotationPreset, Slot, SlotKind, SLOT_JOY_DOWN, SLOT_JOY_LEFT,
    SLOT_JOY_PRESS, SLOT_JOY_RIGHT, SLOT_JOY_UP,
};
use crate::keycodes::{self, CONSUMER_USAGES, KEYBOARD_USAGES};

/// Direction used by every previous/value/next editor control.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CycleDirection {
    Previous,
    Next,
}

fn cycle_index(length: usize, current: Option<usize>, direction: CycleDirection) -> Option<usize> {
    if length == 0 {
        return None;
    }

    Some(match (current.filter(|index| *index < length), direction) {
        (Some(0), CycleDirection::Previous) => length - 1,
        (Some(index), CycleDirection::Previous) => index - 1,
        (Some(index), CycleDirection::Next) if index + 1 == length => 0,
        (Some(index), CycleDirection::Next) => index + 1,
        (None, CycleDirection::Previous) => length - 1,
        (None, CycleDirection::Next) => 0,
    })
}

// -------------------------------------------------------------------------
// Semantic editor used by the physical keys and touch tap.
// -------------------------------------------------------------------------

/// The four curated behavior families shown by the simple editor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SimpleBehaviorKind {
    ApplicationShortcut,
    MacOs,
    Keystroke,
    App,
}

impl SimpleBehaviorKind {
    pub const ALL: [Self; 4] = [
        Self::ApplicationShortcut,
        Self::MacOs,
        Self::Keystroke,
        Self::App,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::ApplicationShortcut => "Application shortcut",
            Self::MacOs => "macOS control",
            Self::Keystroke => "Keystroke",
            Self::App => "Open application",
        }
    }
}

fn behavior_kind(behavior: &ControlBehavior) -> SimpleBehaviorKind {
    match behavior {
        ControlBehavior::ApplicationShortcut { .. } => SimpleBehaviorKind::ApplicationShortcut,
        ControlBehavior::MacOs { .. } => SimpleBehaviorKind::MacOs,
        ControlBehavior::Keystroke => SimpleBehaviorKind::Keystroke,
        ControlBehavior::App { .. } => SimpleBehaviorKind::App,
    }
}

/// Classify a semantic behavior only when its stored execution mapping still
/// agrees with it. `None` means the editor should present an existing/custom
/// mapping rather than mislabel stale imported state.
pub fn classify_simple_behavior(
    input: &InputConfig,
    slot_index: usize,
) -> Option<SimpleBehaviorKind> {
    if !behaviors::behavior_is_consistent(input, slot_index) {
        return None;
    }
    input.behavior.as_ref().map(behavior_kind)
}

/// Human-readable family name, including the legacy/custom fallback.
pub fn simple_behavior_label(input: &InputConfig, slot_index: usize) -> &'static str {
    classify_simple_behavior(input, slot_index)
        .map(SimpleBehaviorKind::label)
        .unwrap_or("Existing mapping")
}

/// Apply one semantic family. Existing values belonging to that family are
/// retained where possible; otherwise the same safe defaults as the original
/// editor are used.
pub fn apply_simple_behavior_kind(
    input: &mut InputConfig,
    slot_index: usize,
    kind: SimpleBehaviorKind,
) -> bool {
    match kind {
        SimpleBehaviorKind::ApplicationShortcut => {
            if let Some(ControlBehavior::ApplicationShortcut {
                application,
                shortcut,
            }) = input.behavior.clone()
            {
                if behaviors::apply_application_shortcut(input, &application, &shortcut) {
                    return true;
                }
            }

            let Some(application) = behaviors::APPLICATION_SHORTCUTS.first() else {
                return false;
            };
            let Some(shortcut) = application.shortcuts.first() else {
                return false;
            };
            behaviors::apply_application_shortcut(input, application.id, shortcut.id)
        }
        SimpleBehaviorKind::MacOs => {
            let command = match input.behavior.as_ref() {
                Some(ControlBehavior::MacOs { command }) => *command,
                _ => MacOsControl::PlayPause,
            };
            behaviors::apply_macos(input, slot_index, command);
            true
        }
        SimpleBehaviorKind::Keystroke => {
            let (mods, key) = if input.emitted.kind == SlotKind::Keyboard {
                (input.emitted.mods, input.emitted.code)
            } else {
                (0, 0x2C) // Space: visible and harmless when testing a mapping.
            };
            behaviors::apply_keystroke(input, mods, key);
            true
        }
        SimpleBehaviorKind::App => {
            let target = match input.behavior.as_ref() {
                Some(ControlBehavior::App { target }) => target.clone(),
                _ => String::new(),
            };
            behaviors::apply_app(input, slot_index, target);
            true
        }
    }
}

/// Move to the adjacent simple behavior family and immediately apply its
/// execution defaults. An existing/custom mapping enters at the directional
/// edge (first on Next, last on Previous).
pub fn cycle_simple_behavior(
    input: &mut InputConfig,
    slot_index: usize,
    direction: CycleDirection,
) -> SimpleBehaviorKind {
    let current = classify_simple_behavior(input, slot_index).and_then(|kind| {
        SimpleBehaviorKind::ALL
            .iter()
            .position(|item| *item == kind)
    });
    let index = cycle_index(SimpleBehaviorKind::ALL.len(), current, direction)
        .expect("simple behavior catalog is non-empty");
    let kind = SimpleBehaviorKind::ALL[index];
    let _ = apply_simple_behavior_kind(input, slot_index, kind);
    kind
}

/// Resolve the currently selected shortcut application, if it is still in
/// the curated catalog.
pub fn selected_shortcut_application(input: &InputConfig) -> Option<&'static ShortcutApplication> {
    let ControlBehavior::ApplicationShortcut { application, .. } = input.behavior.as_ref()? else {
        return None;
    };
    behaviors::shortcut_application(application)
}

/// Resolve the currently selected shortcut preset, if both IDs are valid.
pub fn selected_shortcut_preset(input: &InputConfig) -> Option<&'static ShortcutPreset> {
    let ControlBehavior::ApplicationShortcut {
        application,
        shortcut,
    } = input.behavior.as_ref()?
    else {
        return None;
    };
    behaviors::shortcut_preset(application, shortcut)
}

/// Cycle the application catalog. Changing applications selects that app's
/// first preset, matching the previous editor's behavior.
pub fn cycle_shortcut_application(
    input: &mut InputConfig,
    direction: CycleDirection,
) -> Option<&'static ShortcutApplication> {
    let current = selected_shortcut_application(input).and_then(|selected| {
        behaviors::APPLICATION_SHORTCUTS
            .iter()
            .position(|application| application.id == selected.id)
    });
    let index = cycle_index(behaviors::APPLICATION_SHORTCUTS.len(), current, direction)?;
    let application = &behaviors::APPLICATION_SHORTCUTS[index];
    let shortcut = application.shortcuts.first()?;
    if behaviors::apply_application_shortcut(input, application.id, shortcut.id) {
        Some(application)
    } else {
        None
    }
}

/// Cycle within the selected application's shortcut presets.
pub fn cycle_shortcut_preset(
    input: &mut InputConfig,
    direction: CycleDirection,
) -> Option<&'static ShortcutPreset> {
    let application = selected_shortcut_application(input)
        .or_else(|| behaviors::APPLICATION_SHORTCUTS.first())?;
    let current = selected_shortcut_preset(input).and_then(|selected| {
        application
            .shortcuts
            .iter()
            .position(|shortcut| shortcut.id == selected.id)
    });
    let index = cycle_index(application.shortcuts.len(), current, direction)?;
    let shortcut = &application.shortcuts[index];
    if behaviors::apply_application_shortcut(input, application.id, shortcut.id) {
        Some(shortcut)
    } else {
        None
    }
}

/// Cycle through the curated macOS controls and apply the corresponding
/// device/host mapping atomically.
pub fn cycle_macos_preset(
    input: &mut InputConfig,
    slot_index: usize,
    direction: CycleDirection,
) -> MacOsControl {
    let current = match input.behavior.as_ref() {
        Some(ControlBehavior::MacOs { command }) => behaviors::MACOS_PRESETS
            .iter()
            .position(|preset| preset.command == *command),
        _ => None,
    };
    let index = cycle_index(behaviors::MACOS_PRESETS.len(), current, direction)
        .expect("macOS preset catalog is non-empty");
    let command = behaviors::MACOS_PRESETS[index].command;
    behaviors::apply_macos(input, slot_index, command);
    command
}

pub fn macos_preset_label(command: MacOsControl) -> &'static str {
    behaviors::macos_preset(command).label
}

/// Cycle the full keyboard usage catalog for a semantic Keystroke behavior.
/// The current modifier chord is retained and the host action is cleared by
/// `apply_keystroke`, keeping the behavior representation coherent.
pub fn cycle_keyboard_usage(input: &mut InputConfig, direction: CycleDirection) -> Option<u16> {
    let current = (input.emitted.kind == SlotKind::Keyboard)
        .then_some(input.emitted.code)
        .and_then(|usage| {
            KEYBOARD_USAGES
                .iter()
                .position(|definition| definition.usage == usage)
        });
    let index = cycle_index(KEYBOARD_USAGES.len(), current, direction)?;
    let usage = KEYBOARD_USAGES[index].usage;
    let mods = if input.emitted.kind == SlotKind::Keyboard {
        input.emitted.mods
    } else {
        0
    };
    behaviors::apply_keystroke(input, mods, usage);
    Some(usage)
}

pub fn keyboard_usage_label(usage: u16) -> String {
    keycodes::keyboard_name(usage)
        .map(str::to_owned)
        .unwrap_or_else(|| format!("Keyboard usage 0x{usage:04X}"))
}

// -------------------------------------------------------------------------
// Generic emitted slot editor.
// -------------------------------------------------------------------------

pub const SLOT_KINDS: [SlotKind; 3] = [SlotKind::None, SlotKind::Keyboard, SlotKind::Consumer];

pub const fn slot_kind_label(kind: SlotKind) -> &'static str {
    match kind {
        SlotKind::None => "Nothing",
        SlotKind::Keyboard => "Keyboard key",
        SlotKind::Consumer => "Media / system key",
    }
}

/// Switch an emitted slot kind and assign a useful starting usage. Host
/// actions remain untouched, as they are an independent layer in the generic
/// editor.
pub fn apply_slot_kind(input: &mut InputConfig, kind: SlotKind) {
    if input.emitted.kind == kind {
        return;
    }

    input.emitted.kind = kind;
    input.emitted.code = match kind {
        SlotKind::None => 0,
        SlotKind::Keyboard => 0x68, // F13: non-printing and interceptable.
        SlotKind::Consumer => 0xCD, // Play/Pause.
    };
    if kind != SlotKind::Keyboard {
        input.emitted.mods = 0;
    }
}

pub fn cycle_slot_kind(input: &mut InputConfig, direction: CycleDirection) -> SlotKind {
    let current = SLOT_KINDS
        .iter()
        .position(|kind| *kind == input.emitted.kind);
    let index =
        cycle_index(SLOT_KINDS.len(), current, direction).expect("slot kind catalog is non-empty");
    let kind = SLOT_KINDS[index];
    apply_slot_kind(input, kind);
    kind
}

/// Cycle the picker appropriate to the current emitted kind. Unknown imported
/// usages enter at the directional edge instead of being silently discarded
/// until the user explicitly cycles.
pub fn cycle_emitted_usage(input: &mut InputConfig, direction: CycleDirection) -> Option<u16> {
    let usage = match input.emitted.kind {
        SlotKind::None => return None,
        SlotKind::Keyboard => {
            let current = KEYBOARD_USAGES
                .iter()
                .position(|definition| definition.usage == input.emitted.code);
            let index = cycle_index(KEYBOARD_USAGES.len(), current, direction)?;
            KEYBOARD_USAGES[index].usage
        }
        SlotKind::Consumer => {
            let current = CONSUMER_USAGES
                .iter()
                .position(|(usage, _)| *usage == input.emitted.code);
            let index = cycle_index(CONSUMER_USAGES.len(), current, direction)?;
            CONSUMER_USAGES[index].0
        }
    };
    input.emitted.code = usage;
    Some(usage)
}

pub fn emitted_usage_label(emitted: Slot) -> String {
    match emitted.kind {
        SlotKind::None => "Nothing emitted".to_string(),
        SlotKind::Keyboard => keyboard_usage_label(emitted.code),
        SlotKind::Consumer => keycodes::consumer_name(emitted.code)
            .map(str::to_owned)
            .unwrap_or_else(|| format!("Consumer usage 0x{:04X}", emitted.code)),
    }
}

// -------------------------------------------------------------------------
// Host actions.
// -------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionKind {
    None,
    Keystroke,
    Macro,
    Run,
    Open,
    Media,
    AppSettings,
}

impl ActionKind {
    pub const ALL: [Self; 7] = [
        Self::None,
        Self::Keystroke,
        Self::Macro,
        Self::Run,
        Self::Open,
        Self::Media,
        Self::AppSettings,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::None => "Pass through",
            Self::Keystroke => "Host keystroke",
            Self::Macro => "Macro",
            Self::Run => "Run command",
            Self::Open => "Open file or URL",
            Self::Media => "Host media control",
            Self::AppSettings => "Open OpenMicro settings",
        }
    }
}

pub const fn classify_action(action: &Action) -> ActionKind {
    match action {
        Action::None => ActionKind::None,
        Action::Keystroke { .. } => ActionKind::Keystroke,
        Action::Macro { .. } => ActionKind::Macro,
        Action::Run { .. } => ActionKind::Run,
        Action::Open { .. } => ActionKind::Open,
        Action::Media { .. } => ActionKind::Media,
        Action::AppSettings => ActionKind::AppSettings,
    }
}

/// Change action variants without resetting an already-selected variant's
/// payload. Defaults mirror the established editor and make Media useful
/// immediately while text/recording-based actions begin empty.
pub fn apply_action_kind(action: &mut Action, kind: ActionKind) {
    if classify_action(action) == kind {
        return;
    }

    *action = match kind {
        ActionKind::None => Action::None,
        ActionKind::Keystroke => Action::Keystroke { mods: 0, key: 0 },
        ActionKind::Macro => Action::Macro { steps: Vec::new() },
        ActionKind::Run => Action::Run {
            command: String::new(),
        },
        ActionKind::Open => Action::Open {
            target: String::new(),
        },
        ActionKind::Media => Action::Media {
            op: MediaOp::PlayPause,
        },
        ActionKind::AppSettings => Action::AppSettings,
    };
}

pub fn cycle_action_kind(action: &mut Action, direction: CycleDirection) -> ActionKind {
    let current_kind = classify_action(action);
    let current = ActionKind::ALL
        .iter()
        .position(|kind| *kind == current_kind);
    let index = cycle_index(ActionKind::ALL.len(), current, direction)
        .expect("action kind catalog is non-empty");
    let kind = ActionKind::ALL[index];
    apply_action_kind(action, kind);
    kind
}

pub const MEDIA_OPS: [MediaOp; 8] = [
    MediaOp::VolumeUp,
    MediaOp::VolumeDown,
    MediaOp::Mute,
    MediaOp::PlayPause,
    MediaOp::NextTrack,
    MediaOp::PrevTrack,
    MediaOp::BrightnessUp,
    MediaOp::BrightnessDown,
];

pub const fn media_op_label(op: MediaOp) -> &'static str {
    match op {
        MediaOp::VolumeUp => "Volume up",
        MediaOp::VolumeDown => "Volume down",
        MediaOp::Mute => "Mute",
        MediaOp::PlayPause => "Play / pause",
        MediaOp::NextTrack => "Next track",
        MediaOp::PrevTrack => "Previous track",
        MediaOp::BrightnessUp => "Brightness up",
        MediaOp::BrightnessDown => "Brightness down",
    }
}

pub fn cycle_media_op(op: &mut MediaOp, direction: CycleDirection) -> MediaOp {
    let current = MEDIA_OPS.iter().position(|item| item == op);
    let index = cycle_index(MEDIA_OPS.len(), current, direction)
        .expect("media operation catalog is non-empty");
    *op = MEDIA_OPS[index];
    *op
}

// -------------------------------------------------------------------------
// Rotator and joystick semantic controls.
// -------------------------------------------------------------------------

pub fn cycle_rotator_rotation(
    profile: &mut Profile,
    direction: CycleDirection,
) -> RotatorRotationPreset {
    let current = RotatorRotationPreset::infer(profile).and_then(|selected| {
        RotatorRotationPreset::ALL
            .iter()
            .position(|preset| *preset == selected)
    });
    let index = cycle_index(RotatorRotationPreset::ALL.len(), current, direction)
        .expect("rotator rotation catalog is non-empty");
    let preset = RotatorRotationPreset::ALL[index];
    preset.apply_to(profile);
    preset
}

pub fn cycle_rotator_press(profile: &mut Profile, direction: CycleDirection) -> RotatorPressPreset {
    let current = RotatorPressPreset::infer(profile).and_then(|selected| {
        RotatorPressPreset::ALL
            .iter()
            .position(|preset| *preset == selected)
    });
    let index = cycle_index(RotatorPressPreset::ALL.len(), current, direction)
        .expect("rotator press catalog is non-empty");
    let preset = RotatorPressPreset::ALL[index];
    preset.apply_to(profile);
    preset
}

pub fn cycle_joystick_mode(profile: &mut Profile, direction: CycleDirection) -> JoystickMode {
    let current_mode = JoystickMode::infer(profile);
    let current = JoystickMode::ALL
        .iter()
        .position(|mode| *mode == current_mode);
    let index = cycle_index(JoystickMode::ALL.len(), current, direction)
        .expect("joystick mode catalog is non-empty");
    let mode = JoystickMode::ALL[index];
    mode.apply_to(profile);
    mode
}

/// One editable physical joystick input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JoystickSubslot {
    Up,
    Down,
    Left,
    Right,
    Press,
}

impl JoystickSubslot {
    pub const ALL: [Self; 5] = [Self::Up, Self::Down, Self::Left, Self::Right, Self::Press];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Up => "Up",
            Self::Down => "Down",
            Self::Left => "Left",
            Self::Right => "Right",
            Self::Press => "Press",
        }
    }

    pub const fn slot_index(self) -> usize {
        match self {
            Self::Up => SLOT_JOY_UP,
            Self::Down => SLOT_JOY_DOWN,
            Self::Left => SLOT_JOY_LEFT,
            Self::Right => SLOT_JOY_RIGHT,
            Self::Press => SLOT_JOY_PRESS,
        }
    }

    pub fn from_slot_index(slot_index: usize) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|subslot| subslot.slot_index() == slot_index)
    }
}

/// Cycle all five joystick subslots, wrapping around. A selection outside the
/// joystick enters at the directional edge.
pub fn cycle_joystick_subslot(current_slot: usize, direction: CycleDirection) -> JoystickSubslot {
    let current = JoystickSubslot::from_slot_index(current_slot).and_then(|selected| {
        JoystickSubslot::ALL
            .iter()
            .position(|subslot| *subslot == selected)
    });
    let index = cycle_index(JoystickSubslot::ALL.len(), current, direction)
        .expect("joystick subslot catalog is non-empty");
    JoystickSubslot::ALL[index]
}

/// Mode-aware subslot navigation for the view: Mouse and Grade have no
/// per-input editor, Arrows exposes only its configurable press switch, and
/// Custom exposes all five inputs.
pub fn cycle_editable_joystick_subslot(
    profile: &Profile,
    current_slot: usize,
    direction: CycleDirection,
) -> Option<JoystickSubslot> {
    match JoystickMode::infer(profile) {
        JoystickMode::Mouse | JoystickMode::Grade => None,
        JoystickMode::Arrows => Some(JoystickSubslot::Press),
        JoystickMode::Custom => Some(cycle_joystick_subslot(current_slot, direction)),
    }
}

// -------------------------------------------------------------------------
// HID modifiers.
// -------------------------------------------------------------------------

/// Left-hand HID modifier bits used throughout the configuration editor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HidModifier {
    Control,
    Shift,
    Alt,
    Gui,
}

impl HidModifier {
    pub const ALL: [Self; 4] = [Self::Control, Self::Shift, Self::Alt, Self::Gui];

    pub const fn bit(self) -> u8 {
        match self {
            Self::Control => 0x01,
            Self::Shift => 0x02,
            Self::Alt => 0x04,
            Self::Gui => 0x08,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Control => "Control",
            Self::Shift => "Shift",
            Self::Alt => "Alt / Option",
            Self::Gui => "Command / Super",
        }
    }
}

pub const fn hid_modifier_enabled(mods: u8, modifier: HidModifier) -> bool {
    mods & modifier.bit() != 0
}

/// Set one modifier without disturbing other left- or right-hand HID bits.
/// Returns the resulting enabled state.
pub fn set_hid_modifier(mods: &mut u8, modifier: HidModifier, enabled: bool) -> bool {
    if enabled {
        *mods |= modifier.bit();
    } else {
        *mods &= !modifier.bit();
    }
    hid_modifier_enabled(*mods, modifier)
}

/// Toggle one modifier without disturbing the other HID modifier bits.
/// Returns the resulting enabled state.
pub fn toggle_hid_modifier(mods: &mut u8, modifier: HidModifier) -> bool {
    *mods ^= modifier.bit();
    hid_modifier_enabled(*mods, modifier)
}

/// Toggle a generic emitted keyboard modifier. Consumer/None slots reject the
/// edit because modifier bits have no meaning for those usage pages.
pub fn toggle_emitted_modifier(input: &mut InputConfig, modifier: HidModifier) -> Option<bool> {
    (input.emitted.kind == SlotKind::Keyboard)
        .then(|| toggle_hid_modifier(&mut input.emitted.mods, modifier))
}

/// Toggle a semantic Keystroke modifier while keeping its behavior/action
/// representation coherent.
pub fn toggle_simple_keystroke_modifier(input: &mut InputConfig, modifier: HidModifier) -> bool {
    let mut mods = if input.emitted.kind == SlotKind::Keyboard {
        input.emitted.mods
    } else {
        0
    };
    let enabled = toggle_hid_modifier(&mut mods, modifier);
    let key = if input.emitted.kind == SlotKind::Keyboard {
        input.emitted.code
    } else {
        0x2C
    };
    behaviors::apply_keystroke(input, mods, key);
    enabled
}

/// Toggle the shared modifier mask of an Arrow-mode joystick.
pub fn toggle_joystick_arrow_modifier(
    profile: &mut Profile,
    modifier: HidModifier,
) -> Option<bool> {
    let mut mods = JoystickMode::arrow_mods(profile)?;
    let enabled = toggle_hid_modifier(&mut mods, modifier);
    JoystickMode::set_arrow_mods(profile, mods);
    Some(enabled)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{default_codex_profile, JoyMode};

    #[test]
    fn simple_behavior_wraps_and_applies_family_defaults() {
        let mut input = default_codex_profile().inputs[0].clone();
        assert_eq!(
            classify_simple_behavior(&input, 0),
            Some(SimpleBehaviorKind::Keystroke)
        );

        assert_eq!(
            cycle_simple_behavior(&mut input, 0, CycleDirection::Next),
            SimpleBehaviorKind::App
        );
        assert!(matches!(
            input.behavior,
            Some(ControlBehavior::App { ref target }) if target.is_empty()
        ));
        assert!(matches!(
            input.action,
            Action::Open { ref target } if target.is_empty()
        ));

        assert_eq!(
            cycle_simple_behavior(&mut input, 0, CycleDirection::Next),
            SimpleBehaviorKind::ApplicationShortcut
        );
        let first_app = &behaviors::APPLICATION_SHORTCUTS[0];
        let first_shortcut = &first_app.shortcuts[0];
        assert_eq!(input.emitted.kind, SlotKind::Keyboard);
        assert_eq!(input.emitted.mods, first_shortcut.mods);
        assert_eq!(input.emitted.code, first_shortcut.key);
        assert_eq!(input.action, Action::None);

        assert_eq!(
            cycle_simple_behavior(&mut input, 0, CycleDirection::Previous),
            SimpleBehaviorKind::App
        );
    }

    #[test]
    fn applying_simple_kinds_preserves_values_or_uses_safe_defaults() {
        let mut input = default_codex_profile().inputs[0].clone();
        input.emitted.mods = 0x0A;
        input.emitted.code = 0x16;
        input.action = Action::Run {
            command: "stale".to_string(),
        };
        assert!(apply_simple_behavior_kind(
            &mut input,
            0,
            SimpleBehaviorKind::Keystroke
        ));
        assert_eq!(input.emitted.mods, 0x0A);
        assert_eq!(input.emitted.code, 0x16);
        assert_eq!(input.action, Action::None);

        input.emitted = Slot::default();
        input.behavior = None;
        assert!(apply_simple_behavior_kind(
            &mut input,
            0,
            SimpleBehaviorKind::Keystroke
        ));
        assert_eq!(input.emitted.kind, SlotKind::Keyboard);
        assert_eq!(input.emitted.code, 0x2C);

        input.behavior = None;
        assert!(apply_simple_behavior_kind(
            &mut input,
            0,
            SimpleBehaviorKind::MacOs
        ));
        assert_eq!(
            input.behavior,
            Some(ControlBehavior::MacOs {
                command: MacOsControl::PlayPause
            })
        );
        assert_eq!(input.emitted.kind, SlotKind::Consumer);
        assert_eq!(input.emitted.code, 0xCD);
    }

    #[test]
    fn shortcut_application_and_preset_catalogs_wrap() {
        let mut input = default_codex_profile().inputs[0].clone();
        let last_app = behaviors::APPLICATION_SHORTCUTS.last().unwrap();
        let last_shortcut = last_app.shortcuts.last().unwrap();
        assert!(behaviors::apply_application_shortcut(
            &mut input,
            last_app.id,
            last_shortcut.id
        ));

        let app = cycle_shortcut_application(&mut input, CycleDirection::Next).unwrap();
        assert_eq!(app.id, behaviors::APPLICATION_SHORTCUTS[0].id);
        assert_eq!(
            selected_shortcut_preset(&input).unwrap().id,
            app.shortcuts[0].id
        );

        let app = behaviors::APPLICATION_SHORTCUTS[0];
        let last = app.shortcuts.last().unwrap();
        assert!(behaviors::apply_application_shortcut(
            &mut input, app.id, last.id
        ));
        let preset = cycle_shortcut_preset(&mut input, CycleDirection::Next).unwrap();
        assert_eq!(preset.id, app.shortcuts[0].id);
        let preset = cycle_shortcut_preset(&mut input, CycleDirection::Previous).unwrap();
        assert_eq!(preset.id, last.id);
    }

    #[test]
    fn macos_and_keyboard_catalogs_wrap_and_apply() {
        let mut input = default_codex_profile().inputs[0].clone();
        let last = behaviors::MACOS_PRESETS.last().unwrap().command;
        behaviors::apply_macos(&mut input, 0, last);
        let command = cycle_macos_preset(&mut input, 0, CycleDirection::Next);
        assert_eq!(command, behaviors::MACOS_PRESETS[0].command);

        let last_usage = KEYBOARD_USAGES.last().unwrap().usage;
        behaviors::apply_keystroke(&mut input, 0x09, last_usage);
        assert_eq!(
            cycle_keyboard_usage(&mut input, CycleDirection::Next),
            Some(KEYBOARD_USAGES[0].usage)
        );
        assert_eq!(input.emitted.mods, 0x09);
        assert_eq!(
            cycle_keyboard_usage(&mut input, CycleDirection::Previous),
            Some(last_usage)
        );
    }

    #[test]
    fn generic_slot_and_action_defaults_wrap() {
        let mut input = default_codex_profile().inputs[0].clone();
        apply_slot_kind(&mut input, SlotKind::None);
        assert_eq!(input.emitted, Slot::default());
        assert_eq!(
            cycle_slot_kind(&mut input, CycleDirection::Previous),
            SlotKind::Consumer
        );
        assert_eq!(input.emitted.code, 0xCD);

        input.emitted.code = CONSUMER_USAGES[0].0;
        assert_eq!(
            cycle_emitted_usage(&mut input, CycleDirection::Previous),
            Some(CONSUMER_USAGES.last().unwrap().0)
        );

        let mut action = Action::AppSettings;
        assert_eq!(
            cycle_action_kind(&mut action, CycleDirection::Next),
            ActionKind::None
        );
        assert_eq!(
            cycle_action_kind(&mut action, CycleDirection::Previous),
            ActionKind::AppSettings
        );
        apply_action_kind(&mut action, ActionKind::Media);
        assert_eq!(
            action,
            Action::Media {
                op: MediaOp::PlayPause
            }
        );

        let mut op = MediaOp::BrightnessDown;
        assert_eq!(
            cycle_media_op(&mut op, CycleDirection::Next),
            MediaOp::VolumeUp
        );
        assert_eq!(
            cycle_media_op(&mut op, CycleDirection::Previous),
            MediaOp::BrightnessDown
        );
    }

    #[test]
    fn rotator_joystick_and_subslot_catalogs_wrap() {
        let mut profile = default_codex_profile();
        RotatorRotationPreset::HorizontalArrows.apply_to(&mut profile);
        assert_eq!(
            cycle_rotator_rotation(&mut profile, CycleDirection::Next),
            RotatorRotationPreset::Volume
        );
        assert_eq!(
            RotatorRotationPreset::infer(&profile),
            Some(RotatorRotationPreset::Volume)
        );

        RotatorPressPreset::Enter.apply_to(&mut profile);
        assert_eq!(
            cycle_rotator_press(&mut profile, CycleDirection::Next),
            RotatorPressPreset::Mute
        );

        profile.inputs[SLOT_JOY_LEFT].emitted.code = 0x2C;
        assert_eq!(JoystickMode::infer(&profile), JoystickMode::Custom);
        assert_eq!(
            cycle_joystick_mode(&mut profile, CycleDirection::Next),
            JoystickMode::Mouse
        );
        assert_eq!(profile.analog.joy_mode, JoyMode::Mouse);
        assert_eq!(
            cycle_joystick_mode(&mut profile, CycleDirection::Previous),
            JoystickMode::Custom
        );
        assert_eq!(JoystickMode::infer(&profile), JoystickMode::Custom);

        assert_eq!(
            cycle_joystick_subslot(SLOT_JOY_PRESS, CycleDirection::Next),
            JoystickSubslot::Up
        );
        assert_eq!(
            cycle_joystick_subslot(SLOT_JOY_UP, CycleDirection::Previous),
            JoystickSubslot::Press
        );
        assert_eq!(
            cycle_editable_joystick_subslot(&profile, SLOT_JOY_PRESS, CycleDirection::Next),
            Some(JoystickSubslot::Up)
        );
    }

    #[test]
    fn modifier_toggles_preserve_unrelated_bits() {
        let mut mods = 0x80;
        assert!(toggle_hid_modifier(&mut mods, HidModifier::Control));
        assert_eq!(mods, 0x81);
        assert!(!toggle_hid_modifier(&mut mods, HidModifier::Control));
        assert_eq!(mods, 0x80);

        let mut input = default_codex_profile().inputs[0].clone();
        assert_eq!(
            toggle_emitted_modifier(&mut input, HidModifier::Alt),
            Some(true)
        );
        assert_eq!(input.emitted.mods & HidModifier::Alt.bit(), 0x04);

        let mut profile = default_codex_profile();
        assert_eq!(
            toggle_joystick_arrow_modifier(&mut profile, HidModifier::Gui),
            Some(true)
        );
        assert_eq!(JoystickMode::arrow_mods(&profile), Some(0x08));
    }

    #[test]
    fn labels_are_human_readable_for_known_and_unknown_values() {
        assert_eq!(SimpleBehaviorKind::MacOs.label(), "macOS control");
        assert_eq!(slot_kind_label(SlotKind::Consumer), "Media / system key");
        assert_eq!(ActionKind::Run.label(), "Run command");
        assert_eq!(media_op_label(MediaOp::PrevTrack), "Previous track");
        assert_eq!(JoystickSubslot::Press.label(), "Press");
        assert_eq!(HidModifier::Gui.label(), "Command / Super");
        assert_eq!(keyboard_usage_label(0x04), "A");
        assert_eq!(keyboard_usage_label(0xFFFF), "Keyboard usage 0xFFFF");
    }
}
