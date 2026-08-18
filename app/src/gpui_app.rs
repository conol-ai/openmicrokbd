//! Native GPUI frontend for the OpenMicro companion application.
//!
//! The visual language is a quiet 8-bit hardware workstation: whole-pixel
//! measurements, sparse stepped shadows, compact display labels, and native
//! system body text. CJK text and IME composition go through GPUI's platform
//! text stack instead of the former canvas-font path.

use std::borrow::Cow;
use std::path::PathBuf;
use std::time::Duration;

use gpui::{
    div, point, prelude::*, px, relative, size, svg, AnyElement, App, Application, Bounds,
    Context, Div, Entity, Hsla, InteractiveElement, IntoElement, KeyBinding, KeyDownEvent, Menu,
    MenuItem, ParentElement, PathPromptOptions, Render, ScrollHandle, SharedString, Styled,
    Subscription, Timer, Window, WindowAppearance, WindowBounds, WindowOptions,
};
use gpui_component::input::{Input, InputEvent, InputState};
use gpui_component::scroll::ScrollableElement;
use gpui_component::tooltip::Tooltip;
use gpui_component::{Root, StyledExt, TitleBar};

use crate::actions;
use crate::agent_integrations::{
    self, InstallDisposition, InstallState, IntegrationKind, IntegrationReport,
};
use crate::behaviors::{self, InstalledApp};
use crate::config::{LedPattern, self, Action, ControlBehavior, InputConfig, JoystickMode, LanguageSetting, MacroStep,
    MacroStepEntry, MediaOp, RotatorPressPreset, RotatorRotationPreset, SlotKind, ThemeSetting,
    KEY_SLOTS, SLOT_ENC_CCW, SLOT_ENC_CW, SLOT_ENC_PRESS, SLOT_JOY_DOWN, SLOT_JOY_LEFT,
    SLOT_JOY_PRESS, SLOT_JOY_RIGHT, SLOT_JOY_UP, SLOT_TOUCH_SWIPE_L, SLOT_TOUCH_SWIPE_R,
    SLOT_TOUCH_TAP,};
use crate::device::DeviceCmd;
use crate::editor_logic::{self, CycleDirection, SimpleBehaviorKind};
use crate::events;
use crate::gpui_controls::{self as controls, CellVisual};
use crate::host_state::{HostEffect, HostState};
use crate::i18n::{self, tr};
use crate::keycodes;
use crate::macos_updater::{MacOsUpdater, UpdateCheck};
use crate::pixel::{self, BadgeTone};
use crate::release::{self, DownloadKind};
use crate::status::ActivityStatus;

// Quit is a real GPUI action (not just a tray menu id) so macOS gives it the
// standard Cmd+Q route: app menu item + global key binding, wired in run().
gpui::actions!(openmicro, [Quit]);

const CELL_ENCODER: usize = 13;
const CELL_JOYSTICK: usize = 14;
const CELL_TOUCH: usize = 15;
const ICON_PAGE_SIZE: usize = 30;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Sheet {
    #[default]
    None,
    Settings,
    Macro,
    Firmware,
    Applications,
    Icons,
    KeyPicker,
    ShortcutPicker,
}

/// Which field a key picked from the keyboard sheet lands in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum KeyTarget {
    #[default]
    SimpleKey,
    EmittedCode,
}

/// One key on the picker keyboard: a HID usage, or a bare modifier hold
/// (HID modifier bit; stored as {mods: bit, code: 0}, which the firmware
/// reports as a pure modifier press).
#[derive(Clone, Copy, PartialEq)]
enum PickedKey {
    Usage(u16),
    Modifier(u8),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum RecordTarget {
    #[default]
    None,
    Action,
    MacroStep(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AppUpdateDetailState {
    Error,
    ManualDownloading,
    ManualReady,
    SparkleActive,
    Available,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AppUpdateButtonState {
    StartSparkle,
    SparkleBusy,
    DownloadDmg,
    DownloadingDmg,
    OpenDmg,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct AppUpdateControls {
    detail: AppUpdateDetailState,
    sparkle: Option<AppUpdateButtonState>,
    manual: Option<AppUpdateButtonState>,
    dismissible: bool,
}

fn app_update_controls(
    sparkle_available: bool,
    sparkle_active: bool,
    manual_downloading: bool,
    manual_ready: bool,
    has_error: bool,
) -> AppUpdateControls {
    let detail = if has_error {
        AppUpdateDetailState::Error
    } else if sparkle_available && sparkle_active {
        AppUpdateDetailState::SparkleActive
    } else if !sparkle_available && manual_downloading {
        AppUpdateDetailState::ManualDownloading
    } else if !sparkle_available && manual_ready {
        AppUpdateDetailState::ManualReady
    } else {
        AppUpdateDetailState::Available
    };
    let sparkle = sparkle_available.then_some(if sparkle_active {
        AppUpdateButtonState::SparkleBusy
    } else {
        AppUpdateButtonState::StartSparkle
    });
    let manual = (!sparkle_available).then_some(if manual_downloading {
        AppUpdateButtonState::DownloadingDmg
    } else if manual_ready {
        AppUpdateButtonState::OpenDmg
    } else {
        AppUpdateButtonState::DownloadDmg
    });

    AppUpdateControls {
        detail,
        sparkle,
        manual,
        dismissible: !sparkle_active && !manual_downloading,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ModifierTarget {
    Simple,
    Arrow,
    Emitted,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum IconLibrary {
    #[default]
    Lucide,
    Simple,
}

#[derive(Clone, Copy)]
enum PickerIcon {
    Lucide(&'static str),
    Simple(&'static crate::simple_icons::SimpleIcon),
}

const FEATURED_SIMPLE_ICONS: &[&str] = &[
    "apple",
    "google",
    "github",
    "youtube",
    "spotify",
    "discord",
    "instagram",
    "facebook",
    "x",
    "whatsapp",
    "tiktok",
    "twitch",
    "steam",
    "netflix",
    "reddit",
    "notion",
    "figma",
    "docker",
    "android",
    "react",
    "typescript",
];

fn simple_picker_icons(query: &str) -> Vec<&'static crate::simple_icons::SimpleIcon> {
    if !query.trim().is_empty() {
        return crate::simple_icons::search(query);
    }

    let mut icons = Vec::with_capacity(crate::simple_icons::icons().len());
    icons.extend(
        FEATURED_SIMPLE_ICONS
            .iter()
            .filter_map(|slug| crate::simple_icons::find(slug)),
    );
    icons.extend(
        crate::simple_icons::icons()
            .iter()
            .filter(|icon| !FEATURED_SIMPLE_ICONS.contains(&icon.slug.as_str())),
    );
    icons
}

fn icon_picker_page(value: &str) -> usize {
    if let Some(slug) = crate::simple_icons::slug_from_storage(value) {
        return simple_picker_icons("")
            .iter()
            .position(|icon| icon.slug == slug)
            .map(|index| index / ICON_PAGE_SIZE)
            .unwrap_or(0);
    }

    crate::lucide::ICONS
        .binary_search_by(|(name, _)| name.cmp(&value))
        .map(|index| index / ICON_PAGE_SIZE)
        .unwrap_or(0)
}

/// The picker keyboard, ANSI-flavoured: (cap label, key, width units).
#[rustfmt::skip]
const KEY_PICKER_ROWS: &[&[(&str, PickedKey, f32)]] = &[
    &[("esc", PickedKey::Usage(0x29), 1.0), ("F1", PickedKey::Usage(0x3A), 1.0), ("F2", PickedKey::Usage(0x3B), 1.0), ("F3", PickedKey::Usage(0x3C), 1.0), ("F4", PickedKey::Usage(0x3D), 1.0), ("F5", PickedKey::Usage(0x3E), 1.0), ("F6", PickedKey::Usage(0x3F), 1.0), ("F7", PickedKey::Usage(0x40), 1.0), ("F8", PickedKey::Usage(0x41), 1.0), ("F9", PickedKey::Usage(0x42), 1.0), ("F10", PickedKey::Usage(0x43), 1.0), ("F11", PickedKey::Usage(0x44), 1.0), ("F12", PickedKey::Usage(0x45), 1.0)],
    &[("`", PickedKey::Usage(0x35), 1.0), ("1", PickedKey::Usage(0x1E), 1.0), ("2", PickedKey::Usage(0x1F), 1.0), ("3", PickedKey::Usage(0x20), 1.0), ("4", PickedKey::Usage(0x21), 1.0), ("5", PickedKey::Usage(0x22), 1.0), ("6", PickedKey::Usage(0x23), 1.0), ("7", PickedKey::Usage(0x24), 1.0), ("8", PickedKey::Usage(0x25), 1.0), ("9", PickedKey::Usage(0x26), 1.0), ("0", PickedKey::Usage(0x27), 1.0), ("-", PickedKey::Usage(0x2D), 1.0), ("=", PickedKey::Usage(0x2E), 1.0), ("⌫", PickedKey::Usage(0x2A), 1.5)],
    &[("tab", PickedKey::Usage(0x2B), 1.5), ("Q", PickedKey::Usage(0x14), 1.0), ("W", PickedKey::Usage(0x1A), 1.0), ("E", PickedKey::Usage(0x08), 1.0), ("R", PickedKey::Usage(0x15), 1.0), ("T", PickedKey::Usage(0x17), 1.0), ("Y", PickedKey::Usage(0x1C), 1.0), ("U", PickedKey::Usage(0x18), 1.0), ("I", PickedKey::Usage(0x0C), 1.0), ("O", PickedKey::Usage(0x12), 1.0), ("P", PickedKey::Usage(0x13), 1.0), ("[", PickedKey::Usage(0x2F), 1.0), ("]", PickedKey::Usage(0x30), 1.0), ("\\", PickedKey::Usage(0x31), 1.0)],
    &[("caps", PickedKey::Usage(0x39), 1.9), ("A", PickedKey::Usage(0x04), 1.0), ("S", PickedKey::Usage(0x16), 1.0), ("D", PickedKey::Usage(0x07), 1.0), ("F", PickedKey::Usage(0x09), 1.0), ("G", PickedKey::Usage(0x0A), 1.0), ("H", PickedKey::Usage(0x0B), 1.0), ("J", PickedKey::Usage(0x0D), 1.0), ("K", PickedKey::Usage(0x0E), 1.0), ("L", PickedKey::Usage(0x0F), 1.0), (";", PickedKey::Usage(0x33), 1.0), ("'", PickedKey::Usage(0x34), 1.0), ("⏎", PickedKey::Usage(0x28), 1.6)],
    &[("⇧", PickedKey::Modifier(0x02), 2.4), ("Z", PickedKey::Usage(0x1D), 1.0), ("X", PickedKey::Usage(0x1B), 1.0), ("C", PickedKey::Usage(0x06), 1.0), ("V", PickedKey::Usage(0x19), 1.0), ("B", PickedKey::Usage(0x05), 1.0), ("N", PickedKey::Usage(0x11), 1.0), ("M", PickedKey::Usage(0x10), 1.0), (",", PickedKey::Usage(0x36), 1.0), (".", PickedKey::Usage(0x37), 1.0), ("/", PickedKey::Usage(0x38), 1.0), ("⇧ R", PickedKey::Modifier(0x20), 2.1)],
    &[("⌃", PickedKey::Modifier(0x01), 1.4), ("⌥", PickedKey::Modifier(0x04), 1.4), ("⌘", PickedKey::Modifier(0x08), 1.7), ("space", PickedKey::Usage(0x2C), 5.4), ("⌘ R", PickedKey::Modifier(0x80), 1.7), ("⌥ R", PickedKey::Modifier(0x40), 1.4), ("⌃ R", PickedKey::Modifier(0x10), 1.4)],
    &[("F13", PickedKey::Usage(0x68), 1.0), ("F14", PickedKey::Usage(0x69), 1.0), ("F15", PickedKey::Usage(0x6A), 1.0), ("F16", PickedKey::Usage(0x6B), 1.0), ("F17", PickedKey::Usage(0x6C), 1.0), ("F18", PickedKey::Usage(0x6D), 1.0), ("F19", PickedKey::Usage(0x6E), 1.0), ("F20", PickedKey::Usage(0x6F), 1.0), ("ins", PickedKey::Usage(0x49), 1.0), ("del", PickedKey::Usage(0x4C), 1.0), ("home", PickedKey::Usage(0x4A), 1.0), ("end", PickedKey::Usage(0x4D), 1.0), ("pgup", PickedKey::Usage(0x4B), 1.0), ("pgdn", PickedKey::Usage(0x4E), 1.0)],
    &[("←", PickedKey::Usage(0x50), 1.0), ("↑", PickedKey::Usage(0x52), 1.0), ("↓", PickedKey::Usage(0x51), 1.0), ("→", PickedKey::Usage(0x4F), 1.0), ("prtsc", PickedKey::Usage(0x46), 1.2), ("scrlk", PickedKey::Usage(0x47), 1.2), ("pause", PickedKey::Usage(0x48), 1.2)],
];

/// Named single-colour presets the LED pattern spinners cycle through.
const PATTERN_PALETTE: &[(&str, u8, u8, u8)] = &[
    ("pat_red", 255, 0, 0),
    ("pat_orange", 255, 96, 0),
    ("pat_yellow", 255, 200, 0),
    ("pat_green", 0, 255, 60),
    ("pat_cyan", 0, 200, 255),
    ("pat_blue", 0, 80, 255),
    ("pat_purple", 150, 0, 255),
    ("pat_pink", 255, 60, 150),
];

impl PickerIcon {
    fn label(self) -> &'static str {
        match self {
            Self::Lucide(name) => name,
            Self::Simple(icon) => icon.title.as_str(),
        }
    }

    fn storage_value(self) -> String {
        match self {
            Self::Lucide(name) => name.to_string(),
            Self::Simple(icon) => crate::simple_icons::storage_value(&icon.slug),
        }
    }

    fn element_id(self) -> SharedString {
        match self {
            Self::Lucide(name) => SharedString::from(format!("lucide-{name}")),
            Self::Simple(icon) => SharedString::from(format!("simple-{}", icon.slug)),
        }
    }

    fn visual(self, selected: bool) -> AnyElement {
        let color = if selected {
            pixel::accent_highlight_color()
        } else {
            pixel::muted_text_color()
        };
        match self {
            Self::Lucide(name) => lucide_icon_visual(name, 20., color),
            Self::Simple(icon) => svg()
                .w(px(22.))
                .h(px(22.))
                .flex_none()
                .path(crate::simple_icons::asset_path(&icon.slug))
                .text_color(color)
                .into_any_element(),
        }
    }
}

fn resolve_language(setting: LanguageSetting) -> i18n::Lang {
    match setting {
        LanguageSetting::Auto => i18n::detect(),
        LanguageSetting::En => i18n::Lang::En,
        LanguageSetting::ZhHans => i18n::Lang::ZhHans,
        LanguageSetting::ZhHant => i18n::Lang::ZhHant,
        LanguageSetting::Ja => i18n::Lang::Ja,
    }
}

fn resolve_theme(setting: ThemeSetting, appearance: WindowAppearance) -> pixel::ColorScheme {
    match setting {
        ThemeSetting::Light => pixel::ColorScheme::Light,
        ThemeSetting::Dark => pixel::ColorScheme::Dark,
        ThemeSetting::System => match appearance {
            WindowAppearance::Light | WindowAppearance::VibrantLight => pixel::ColorScheme::Light,
            WindowAppearance::Dark | WindowAppearance::VibrantDark => pixel::ColorScheme::Dark,
        },
    }
}

fn cell_for_slot(slot: usize) -> usize {
    match slot {
        0..=12 => slot,
        SLOT_ENC_CW..=SLOT_ENC_PRESS => CELL_ENCODER,
        SLOT_JOY_UP..=SLOT_JOY_PRESS => CELL_JOYSTICK,
        _ => CELL_TOUCH,
    }
}

fn slots_for_cell(cell: usize) -> &'static [usize] {
    const KEYS: [[usize; 1]; KEY_SLOTS] = [
        [0],
        [1],
        [2],
        [3],
        [4],
        [5],
        [6],
        [7],
        [8],
        [9],
        [10],
        [11],
        [12],
    ];
    const ENCODER: [usize; 3] = [SLOT_ENC_CW, SLOT_ENC_CCW, SLOT_ENC_PRESS];
    const JOYSTICK: [usize; 5] = [
        SLOT_JOY_UP,
        SLOT_JOY_DOWN,
        SLOT_JOY_LEFT,
        SLOT_JOY_RIGHT,
        SLOT_JOY_PRESS,
    ];
    const TOUCH: [usize; 3] = [SLOT_TOUCH_TAP, SLOT_TOUCH_SWIPE_L, SLOT_TOUCH_SWIPE_R];
    match cell {
        0..=12 => &KEYS[cell],
        CELL_ENCODER => &ENCODER,
        CELL_JOYSTICK => &JOYSTICK,
        _ => &TOUCH,
    }
}

fn slot_name(slot: usize) -> String {
    match slot {
        0..=12 => {
            let row = match slot {
                0 | 1 => 1,
                2..=5 => 2,
                6..=9 => 3,
                _ => 4,
            };
            tr("slot_key_n")
                .replace("{n}", &(slot + 1).to_string())
                .replace("{r}", &row.to_string())
        }
        SLOT_ENC_CW => tr("slot_enc_cw").into(),
        SLOT_ENC_CCW => tr("slot_enc_ccw").into(),
        SLOT_ENC_PRESS => tr("slot_enc_press").into(),
        SLOT_JOY_UP => tr("slot_joy_up").into(),
        SLOT_JOY_DOWN => tr("slot_joy_down").into(),
        SLOT_JOY_LEFT => tr("slot_joy_left").into(),
        SLOT_JOY_RIGHT => tr("slot_joy_right").into(),
        SLOT_JOY_PRESS => tr("slot_joy_press").into(),
        SLOT_TOUCH_TAP => tr("slot_touch_tap").into(),
        SLOT_TOUCH_SWIPE_L => tr("slot_touch_swipe_l").into(),
        SLOT_TOUCH_SWIPE_R => tr("slot_touch_swipe_r").into(),
        _ => String::new(),
    }
}

fn short_text(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let head: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{head}…")
    } else {
        head
    }
}

fn action_kind_label(action: &Action) -> &'static str {
    match action {
        Action::None => tr("act_none"),
        Action::Keystroke { .. } => tr("act_keystroke"),
        Action::Macro { .. } => tr("act_macro"),
        Action::Run { .. } => tr("act_run"),
        Action::Open { .. } => tr("act_open"),
        Action::Media { .. } => tr("act_media"),
        Action::AppSettings => tr("act_app_settings"),
    }
}

fn media_label(op: MediaOp) -> &'static str {
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

fn key_from_gpui(event: &KeyDownEvent) -> Option<u16> {
    let key = event.keystroke.key.to_ascii_lowercase();
    if key.len() == 1 {
        let byte = key.as_bytes()[0];
        return match byte {
            b'a'..=b'z' => Some(0x04 + (byte - b'a') as u16),
            b'1'..=b'9' => Some(0x1e + (byte - b'1') as u16),
            b'0' => Some(0x27),
            b'-' => Some(0x2d),
            b'=' => Some(0x2e),
            b'[' => Some(0x2f),
            b']' => Some(0x30),
            b'\\' => Some(0x31),
            b';' => Some(0x33),
            b'\'' => Some(0x34),
            b'`' => Some(0x35),
            b',' => Some(0x36),
            b'.' => Some(0x37),
            b'/' => Some(0x38),
            _ => None,
        };
    }
    match key.as_str() {
        "enter" | "return" => Some(0x28),
        "escape" => Some(0x29),
        "backspace" => Some(0x2a),
        "tab" => Some(0x2b),
        "space" => Some(0x2c),
        "home" => Some(0x4a),
        "pageup" => Some(0x4b),
        "delete" => Some(0x4c),
        "end" => Some(0x4d),
        "pagedown" => Some(0x4e),
        "right" | "arrowright" => Some(0x4f),
        "left" | "arrowleft" => Some(0x50),
        "down" | "arrowdown" => Some(0x51),
        "up" | "arrowup" => Some(0x52),
        name if name.starts_with('f') => name[1..]
            .parse::<u16>()
            .ok()
            .filter(|n| (1..=12).contains(n))
            .map(|n| 0x39 + n),
        _ => None,
    }
}

fn gpui_modifiers(event: &KeyDownEvent) -> u8 {
    let mods = event.keystroke.modifiers;
    (mods.control as u8)
        | ((mods.shift as u8) << 1)
        | ((mods.alt as u8) << 2)
        | ((mods.platform as u8) << 3)
}

fn apply_launch_at_login(enable: bool) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|error| error.to_string())?;
    #[cfg(target_os = "macos")]
    let app_path = exe
        .parent()
        .and_then(|macos| macos.parent())
        .and_then(|contents| contents.parent())
        .filter(|bundle| {
            bundle
                .extension()
                .is_some_and(|extension| extension == "app")
        })
        .unwrap_or(&exe);
    #[cfg(not(target_os = "macos"))]
    let app_path = exe.as_path();
    let auto = auto_launch::AutoLaunchBuilder::new()
        .set_app_name("OpenMicro")
        .set_app_path(&app_path.display().to_string())
        .build()
        .map_err(|error| error.to_string())?;
    if enable {
        auto.enable().map_err(|error| error.to_string())
    } else {
        auto.disable().or(Ok(()))
    }
}

fn wrapped_index(current: usize, len: usize, delta: isize) -> usize {
    if len == 0 {
        return 0;
    }
    (current as isize + delta).rem_euclid(len as isize) as usize
}

fn inspector_field(
    label: impl Into<SharedString>,
    detail: impl Into<SharedString>,
    control: impl IntoElement,
) -> Div {
    let label: SharedString = label.into();
    let detail: SharedString = detail.into();
    let mut heading = div()
        .w_full()
        .min_w(px(0.))
        .flex()
        .items_center()
        .gap(px(10.))
        .child(
            div()
                .flex_shrink_0()
                .text_size(px(10.))
                .font_family("Monaco")
                .font_semibold()
                .text_color(pixel::accent_highlight_color())
                .child(label),
        )
        .child(div().flex_1());
    if !detail.is_empty() {
        heading = heading.child(
            div()
                .min_w(px(0.))
                .max_w(px(230.))
                .truncate()
                .text_size(px(11.))
                .text_color(pixel::dim_text_color())
                .child(detail),
        );
    }

    div()
        .w_full()
        .flex()
        .flex_col()
        .gap(px(5.))
        .child(heading)
        .child(control)
}

fn selection_card(
    icon: AnyElement,
    title: impl Into<SharedString>,
    subtitle: Option<SharedString>,
) -> Div {
    let content = div()
        .flex_1()
        .min_w(px(0.))
        .px(px(10.))
        .flex()
        .flex_col()
        .justify_center()
        .gap(px(2.))
        .child(
            div()
                .truncate()
                .text_size(px(13.))
                .font_semibold()
                .text_color(pixel::text_color())
                .child(title.into()),
        );
    let content = if let Some(subtitle) = subtitle {
        content.child(
            div()
                .truncate()
                .font_family("Monaco")
                .text_size(px(9.))
                .text_color(pixel::dim_text_color())
                .child(subtitle),
        )
    } else {
        content
    };

    div()
        .w_full()
        .h(px(46.))
        .min_w(px(0.))
        .flex()
        .items_center()
        .overflow_hidden()
        .bg(pixel::raised_color())
        .rounded(px(2.))
        .cursor_pointer()
        .hover(|style| style.bg(pixel::key_color()))
        .child(
            div()
                .w(px(42.))
                .h_full()
                .flex_shrink_0()
                .flex()
                .items_center()
                .justify_center()
                .child(icon),
        )
        .child(content)
}

fn tiny_button(label: impl Into<SharedString>) -> Div {
    pixel::raised_button_face(label)
        .h(px(32.))
        .px(px(10.))
        .text_size(px(12.))
        .font_family("Monaco")
}

fn paging_button(label: impl Into<SharedString>, enabled: bool) -> Div {
    if enabled {
        tiny_button(label)
    } else {
        div()
            .h(px(32.))
            .px(px(10.))
            .flex()
            .items_center()
            .justify_center()
            .bg(pixel::canvas_color())
            .rounded(px(2.))
            .font_family("Monaco")
            .text_size(px(12.))
            .text_color(pixel::dim_text_color())
            .cursor_default()
            .child(label.into())
    }
}

fn icon_library_tab(label: impl Into<SharedString>, active: bool) -> Div {
    div()
        .h(px(32.))
        .px(px(12.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(2.))
        .font_family("Monaco")
        .font_semibold()
        .text_size(px(11.))
        .text_color(if active {
            pixel::accent_highlight_color()
        } else {
            pixel::muted_text_color()
        })
        .when(active, |tab| {
            tab.bg(pixel::accent_soft_color()).cursor_default()
        })
        .when(!active, |tab| {
            tab.cursor_pointer().hover(|style| {
                style
                    .bg(pixel::raised_color())
                    .text_color(pixel::accent_highlight_color())
            })
        })
        .child(label.into())
}

fn icon_glyph(name: &str) -> SharedString {
    crate::lucide::icon_char(name)
        .map(|glyph| SharedString::from(glyph.to_string()))
        .unwrap_or_else(|| SharedString::from("·"))
}

fn lucide_icon_visual(name: &str, size: f32, color: Hsla) -> AnyElement {
    div()
        .w(px(size))
        .h(px(size))
        .flex()
        .items_center()
        .justify_center()
        .font_family("lucide")
        .text_size(px(size))
        .text_color(color)
        .child(icon_glyph(name))
        .into_any_element()
}

fn shortcut_app_icon(app: &behaviors::ShortcutApplication, size: f32, color: Hsla) -> AnyElement {
    configured_icon_visual(app.icon, size, color)
        .unwrap_or_else(|| lucide_icon_visual("app-window-mac", size, color))
}

/// Catalog order is semantic (legacy apps first); the picker shows a
/// case-insensitively alphabetized list instead.
fn sorted_shortcut_applications() -> Vec<&'static behaviors::ShortcutApplication> {
    let mut apps: Vec<&'static behaviors::ShortcutApplication> =
        behaviors::APPLICATION_SHORTCUTS.iter().collect();
    apps.sort_by_key(|app| app.label.to_lowercase());
    apps
}

fn configured_icon_visual(value: &str, size: f32, color: Hsla) -> Option<AnyElement> {
    if let Some(slug) = crate::simple_icons::slug_from_storage(value) {
        return crate::simple_icons::find(slug).map(|icon| {
            svg()
                .w(px(size + 1.))
                .h(px(size + 1.))
                .flex_none()
                .path(crate::simple_icons::asset_path(&icon.slug))
                .text_color(color)
                .into_any_element()
        });
    }

    crate::lucide::icon_char(value).map(|_| lucide_icon_visual(value, size, color))
}

fn configured_icon_label(value: &str) -> String {
    if value.is_empty() {
        return tr("no_icon").to_string();
    }
    if let Some(slug) = crate::simple_icons::slug_from_storage(value) {
        return crate::simple_icons::find(slug)
            .map(|icon| icon.title.clone())
            .unwrap_or_else(|| slug.to_string());
    }
    value.to_string()
}

fn chrome_icon_button(name: &str) -> Div {
    div()
        .w(px(28.))
        .h(px(26.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(2.))
        .font_family("lucide")
        .text_size(px(14.))
        .text_color(pixel::muted_text_color())
        .cursor_pointer()
        .hover(|style| {
            style
                .bg(pixel::raised_color())
                .text_color(pixel::accent_highlight_color())
        })
        .child(icon_glyph(name))
}

fn logo_mark() -> Div {
    let pip = |lit| {
        div().w(px(5.)).h(px(5.)).bg(if lit {
            pixel::accent_color()
        } else {
            pixel::border_highlight_color()
        })
    };
    div()
        .w(px(20.))
        .h(px(20.))
        .p(px(3.))
        .flex()
        .flex_col()
        .gap(px(2.))
        .bg(pixel::accent_soft_color())
        .rounded(px(2.))
        .child(div().flex().gap(px(2.)).child(pip(false)).child(pip(false)))
        .child(div().flex().gap(px(2.)).child(pip(false)).child(pip(true)))
}

pub struct OpenMicro {
    host: HostState,
    sheet: Sheet,
    key_picker_target: KeyTarget,
    /// Application highlighted in the shortcut picker's left rail.
    shortcut_picker_app: String,
    shortcut_rail_scroll: ScrollHandle,
    shortcut_list_scroll: ScrollHandle,
    recording: RecordTarget,
    advanced: bool,
    macro_draft: Vec<MacroStepEntry>,
    macro_edit_index: Option<usize>,
    installed_apps: Vec<InstalledApp>,
    agent_integrations: Vec<IntegrationReport>,
    agent_integration_feedback: Option<(String, BadgeTone)>,
    app_updater: MacOsUpdater,
    app_updater_active: bool,
    icon_library: IconLibrary,
    icon_query: String,
    icon_page: usize,
    icon_scroll: ScrollHandle,
    confirm_delete: bool,
    confirm_reset: bool,
    syncing_inputs: bool,
    label_input: Entity<InputState>,
    profile_input: Entity<InputState>,
    command_input: Entity<InputState>,
    target_input: Entity<InputState>,
    search_input: Entity<InputState>,
    macro_value_input: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
}

impl OpenMicro {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut host = HostState::new();
        pixel::install_theme(resolve_theme(host.config.theme, window.appearance()), cx);
        i18n::set_lang(resolve_language(host.config.language));
        let _ = host.select_slot(0);

        if let Err(error) = apply_launch_at_login(host.config.launch_at_login) {
            host.logs
                .push_back(format!("launch at login could not be applied: {error}"));
        }

        let label_input =
            cx.new(|cx| InputState::new(window, cx).placeholder(tr("keycap_label_placeholder")));
        let profile_input =
            cx.new(|cx| InputState::new(window, cx).placeholder(tr("profile_name_placeholder")));
        let command_input =
            cx.new(|cx| InputState::new(window, cx).placeholder(tr("shell_command_placeholder")));
        let target_input =
            cx.new(|cx| InputState::new(window, cx).placeholder(tr("open_placeholder")));
        let search_input =
            cx.new(|cx| InputState::new(window, cx).placeholder(tr("icon_search_placeholder")));
        let macro_value_input = cx.new(|cx| InputState::new(window, cx).placeholder("step value"));

        let mut this = Self {
            host,
            sheet: Sheet::None,
            key_picker_target: KeyTarget::SimpleKey,
            shortcut_picker_app: String::new(),
            shortcut_rail_scroll: ScrollHandle::new(),
            shortcut_list_scroll: ScrollHandle::new(),
            recording: RecordTarget::None,
            advanced: false,
            macro_draft: Vec::new(),
            macro_edit_index: None,
            installed_apps: behaviors::installed_apps(),
            agent_integrations: Vec::new(),
            agent_integration_feedback: None,
            app_updater: MacOsUpdater::new(),
            app_updater_active: false,
            icon_library: IconLibrary::Lucide,
            icon_query: String::new(),
            icon_page: 0,
            icon_scroll: ScrollHandle::new(),
            confirm_delete: false,
            confirm_reset: false,
            syncing_inputs: false,
            label_input,
            profile_input,
            command_input,
            target_input,
            search_input,
            macro_value_input,
            _subscriptions: Vec::new(),
        };

        this._subscriptions
            .push(cx.observe_window_appearance(window, |this, window, cx| {
                if this.host.config.theme != ThemeSetting::System {
                    return;
                }
                let scheme = resolve_theme(ThemeSetting::System, window.appearance());
                if scheme != pixel::color_scheme() {
                    pixel::install_theme(scheme, cx);
                }
            }));

        this._subscriptions.push(cx.subscribe(
            &this.label_input,
            |this, input, event: &InputEvent, cx| {
                if !matches!(event, InputEvent::Change) || this.syncing_inputs {
                    return;
                }
                let Some(slot) = this.host.selected_slot else {
                    return;
                };
                this.host.active_profile_mut().inputs[slot].label =
                    input.read(cx).value().to_string();
                this.commit(false, cx);
            },
        ));
        this._subscriptions.push(cx.subscribe(
            &this.profile_input,
            |this, input, event: &InputEvent, cx| {
                if !matches!(event, InputEvent::Change) || this.syncing_inputs {
                    return;
                }
                let name = input.read(cx).value().trim().to_string();
                if !name.is_empty() {
                    this.host.active_profile_mut().name = name;
                    this.commit(false, cx);
                }
            },
        ));
        this._subscriptions.push(cx.subscribe(
            &this.command_input,
            |this, input, event: &InputEvent, cx| {
                if !matches!(event, InputEvent::Change) || this.syncing_inputs {
                    return;
                }
                let Some(slot) = this.host.selected_slot else {
                    return;
                };
                if let Action::Run { command } =
                    &mut this.host.active_profile_mut().inputs[slot].action
                {
                    *command = input.read(cx).value().to_string();
                    this.commit(false, cx);
                }
            },
        ));
        this._subscriptions.push(cx.subscribe(
            &this.target_input,
            |this, input, event: &InputEvent, cx| {
                if !matches!(event, InputEvent::Change) || this.syncing_inputs {
                    return;
                }
                let Some(slot) = this.host.selected_slot else {
                    return;
                };
                let value = input.read(cx).value().to_string();
                match &mut this.host.active_profile_mut().inputs[slot].action {
                    Action::Open { target } => *target = value,
                    _ => return,
                }
                this.commit(false, cx);
            },
        ));
        this._subscriptions.push(cx.subscribe(
            &this.search_input,
            |this, input, event: &InputEvent, cx| {
                if !matches!(event, InputEvent::Change) || this.syncing_inputs {
                    return;
                }
                this.icon_query = input.read(cx).value().to_string();
                this.icon_page = 0;
                this.icon_scroll.set_offset(point(px(0.), px(0.)));
                this.shortcut_list_scroll.set_offset(point(px(0.), px(0.)));
                cx.notify();
            },
        ));
        this._subscriptions.push(cx.subscribe(
            &this.macro_value_input,
            |this, input, event: &InputEvent, cx| {
                if !matches!(event, InputEvent::Change) || this.syncing_inputs {
                    return;
                }
                let Some(index) = this.macro_edit_index else {
                    return;
                };
                let Some(entry) = this.macro_draft.get_mut(index) else {
                    return;
                };
                let value = input.read(cx).value().to_string();
                match &mut entry.step {
                    MacroStep::Delay { ms } => {
                        if let Ok(value) = value.trim().parse::<u64>() {
                            *ms = value.min(60_000);
                        }
                    }
                    MacroStep::Run { command } => *command = value,
                    MacroStep::Open { target } => *target = value,
                    MacroStep::Keystroke { .. } | MacroStep::Media { .. } => return,
                }
                cx.notify();
            },
        ));

        this.sync_inputs(window, cx);
        this.start_event_loop(cx);
        this.start_release_timer(cx);
        cx.on_app_quit(|this, _| {
            let _ = config::save(&this.host.config);
            async {}
        })
        .detach();
        this
    }

    fn start_event_loop(&self, cx: &mut Context<Self>) {
        let receiver = events::receiver();
        cx.spawn(async move |weak, cx| {
            while let Ok(event) = receiver.recv().await {
                let _ = weak.update(cx, |this, cx| {
                    let effects = this.host.handle_event(event);
                    this.apply_host_effects(effects, cx);
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn start_release_timer(&self, cx: &mut Context<Self>) {
        cx.spawn(async move |weak, cx| loop {
            Timer::after(Duration::from_secs(6 * 60 * 60)).await;
            release::spawn_catalog_check();
            if weak.upgrade().is_none() {
                break;
            }
            let _ = weak.update(cx, |_, _| {});
        })
        .detach();
    }

    fn begin_sparkle_update(&mut self, cx: &mut Context<Self>) {
        if self.app_updater_active {
            return;
        }
        self.host.app_update_error = None;
        match self.app_updater.check_for_updates() {
            Ok(UpdateCheck::Started | UpdateCheck::Busy) => {
                self.app_updater_active = true;
                self.watch_sparkle_update(cx);
            }
            Err(error) => self.host.app_update_error = Some(error.to_string()),
        }
        cx.notify();
    }

    fn watch_sparkle_update(&self, cx: &mut Context<Self>) {
        cx.spawn(async move |weak, cx| loop {
            Timer::after(Duration::from_millis(500)).await;
            match weak.update(cx, |this, cx| {
                let finished = !this.app_updater.session_in_progress();
                if finished {
                    this.app_updater_active = false;
                    cx.notify();
                }
                finished
            }) {
                Ok(true) | Err(_) => break,
                Ok(false) => {}
            }
        })
        .detach();
    }

    fn begin_manual_app_download(&mut self, cx: &mut Context<Self>) {
        if self.host.app_downloading {
            return;
        }
        let Some(catalog) = self.host.release.as_ref() else {
            return;
        };
        let Some(asset) = catalog.app_asset().cloned() else {
            self.host.app_update_error =
                Some("no update is available for this platform".into());
            cx.notify();
            return;
        };
        let version = catalog.app.version.clone();
        self.host.app_update_error = None;
        self.host.app_downloading = true;
        self.host.app_download_progress = 0.0;
        release::spawn_download(DownloadKind::App, version, asset);
        cx.notify();
    }

    fn open_manual_app_download(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.host.app_download.as_ref() else {
            return;
        };
        self.host.app_update_error = None;
        if let Err(error) = open::that(path) {
            self.host.app_update_error = Some(format!(
                "cannot open {}: {error}",
                path.display()
            ));
        }
        cx.notify();
    }

    fn apply_host_effects(&mut self, effects: Vec<HostEffect>, cx: &mut Context<Self>) {
        for effect in effects {
            match effect {
                HostEffect::ShowWindow => show_main_window(cx),
                HostEffect::OpenSettings => {
                    self.agent_integration_feedback = None;
                    self.refresh_agent_integrations();
                    self.sheet = Sheet::Settings;
                    show_main_window(cx);
                }
                HostEffect::Quit => {
                    let _ = config::save(&self.host.config);
                    cx.quit();
                }
                HostEffect::ReleaseCellAfter { cell, delay } => {
                    cx.spawn(async move |weak, cx| {
                        Timer::after(delay).await;
                        let _ = weak.update(cx, |this, cx| {
                            this.host.release_cell(cell);
                            cx.notify();
                        });
                    })
                    .detach();
                }
                HostEffect::ExpireActivityAfter {
                    session_id,
                    epoch,
                    delay,
                } => {
                    cx.spawn(async move |weak, cx| {
                        Timer::after(delay).await;
                        let _ = weak.update(cx, |this, cx| {
                            if this.host.expire_activity(&session_id, epoch) {
                                cx.notify();
                            }
                        });
                    })
                    .detach();
                }
            }
        }
    }

    fn commit(&mut self, sync_device: bool, cx: &mut Context<Self>) {
        behaviors::normalize_hidden_triggers(self.host.active_profile_mut());
        if let Err(error) = self.host.persist() {
            self.push_log(format!("could not save config: {error}"));
        }
        if sync_device {
            let _ = self.host.sync_device();
        }
        cx.notify();
    }

    fn push_log(&mut self, line: String) {
        self.host.logs.push_back(line);
        while self.host.logs.len() > 8 {
            self.host.logs.pop_front();
        }
    }

    fn sync_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let profile_name = self.host.active_profile().name.clone();
        let (label, command, target) = self
            .host
            .selected_slot
            .and_then(|slot| self.host.active_profile().inputs.get(slot))
            .map(|input| {
                let command = match &input.action {
                    Action::Run { command } => command.clone(),
                    _ => String::new(),
                };
                let target = match &input.action {
                    Action::Open { target } => target.clone(),
                    _ => String::new(),
                };
                (input.label.clone(), command, target)
            })
            .unwrap_or_default();
        self.syncing_inputs = true;
        self.profile_input
            .update(cx, |input, cx| input.set_value(profile_name, window, cx));
        self.label_input
            .update(cx, |input, cx| input.set_value(label, window, cx));
        self.command_input
            .update(cx, |input, cx| input.set_value(command, window, cx));
        self.target_input
            .update(cx, |input, cx| input.set_value(target, window, cx));
        self.search_input.update(cx, |input, cx| {
            input.set_value(self.icon_query.clone(), window, cx)
        });
        self.syncing_inputs = false;
    }

    fn select_slot(&mut self, slot: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.host.select_slot(slot) {
            self.recording = RecordTarget::None;
            self.advanced = !matches!(
                slot,
                0..=12 | SLOT_ENC_CW..=SLOT_ENC_PRESS | SLOT_TOUCH_TAP
            );
            self.sync_inputs(window, cx);
            cx.notify();
        }
    }

    fn switch_profile(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.sheet != Sheet::None {
            return;
        }
        if self.host.switch_profile(index) {
            self.recording = RecordTarget::None;
            self.sync_inputs(window, cx);
            cx.notify();
        }
    }

    fn cycle_profile(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        let len = self.host.config.profiles.len();
        if len == 0 {
            return;
        }
        let current = self.host.config.active_profile as isize;
        let next = (current + delta).rem_euclid(len as isize) as usize;
        self.switch_profile(next, window, cx);
    }

    fn handle_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.keystroke.key.eq_ignore_ascii_case("escape") {
            if self.recording != RecordTarget::None {
                self.recording = RecordTarget::None;
                cx.notify();
            } else if self.sheet != Sheet::None {
                self.sheet = Sheet::None;
                cx.notify();
            }
            return;
        }
        if self.recording == RecordTarget::None || event.is_held {
            return;
        }
        let Some(key) = key_from_gpui(event) else {
            return;
        };
        let mods = gpui_modifiers(event);
        match self.recording {
            RecordTarget::Action => {
                if let Some(slot) = self.host.selected_slot {
                    self.host.active_profile_mut().inputs[slot].action =
                        Action::Keystroke { mods, key };
                    self.recording = RecordTarget::None;
                    self.commit(false, cx);
                }
            }
            RecordTarget::MacroStep(index) => {
                if let Some(entry) = self.macro_draft.get_mut(index) {
                    entry.step = MacroStep::Keystroke { mods, key };
                    self.recording = RecordTarget::None;
                    self.sync_macro_input(window, cx);
                    cx.notify();
                }
            }
            RecordTarget::None => {}
        }
    }

    fn cycle_rotation(&mut self, delta: isize, cx: &mut Context<Self>) {
        let current = RotatorRotationPreset::infer(self.host.active_profile())
            .and_then(|preset| {
                RotatorRotationPreset::ALL
                    .iter()
                    .position(|item| *item == preset)
            })
            .unwrap_or(0);
        let next = wrapped_index(current, RotatorRotationPreset::ALL.len(), delta);
        RotatorRotationPreset::ALL[next].apply_to(self.host.active_profile_mut());
        self.commit(true, cx);
    }

    fn cycle_rotator_press(&mut self, delta: isize, cx: &mut Context<Self>) {
        let current = RotatorPressPreset::infer(self.host.active_profile())
            .and_then(|preset| {
                RotatorPressPreset::ALL
                    .iter()
                    .position(|item| *item == preset)
            })
            .unwrap_or(0);
        let next = wrapped_index(current, RotatorPressPreset::ALL.len(), delta);
        RotatorPressPreset::ALL[next].apply_to(self.host.active_profile_mut());
        self.commit(true, cx);
    }

    fn cycle_joystick_mode(&mut self, delta: isize, cx: &mut Context<Self>) {
        let current = JoystickMode::ALL
            .iter()
            .position(|item| *item == JoystickMode::infer(self.host.active_profile()))
            .unwrap_or(0);
        let next = wrapped_index(current, JoystickMode::ALL.len(), delta);
        JoystickMode::ALL[next].apply_to(self.host.active_profile_mut());
        self.commit(true, cx);
    }

    fn adjust_joystick_speed(&mut self, delta: i8, cx: &mut Context<Self>) {
        let speed = self.host.active_profile().analog.joy_mouse_speed as i16;
        self.host.active_profile_mut().analog.joy_mouse_speed =
            (speed + delta as i16).clamp(1, 10) as u8;
        self.commit(true, cx);
    }

    fn adjust_threshold(&mut self, delta: i16, cx: &mut Context<Self>) {
        let threshold = self.host.active_profile().analog.joy_threshold as i32;
        self.host.active_profile_mut().analog.joy_threshold =
            (threshold + delta as i32).clamp(200, 1900) as u16;
        self.commit(true, cx);
    }

    fn toggle_arrow_modifier(&mut self, bit: u8, cx: &mut Context<Self>) {
        let mods = JoystickMode::arrow_mods(self.host.active_profile()).unwrap_or(0) ^ bit;
        JoystickMode::set_arrow_mods(self.host.active_profile_mut(), mods);
        self.commit(true, cx);
    }

    fn cycle_subslot(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(slot) = self.host.selected_slot else {
            return;
        };
        let group = slots_for_cell(cell_for_slot(slot));
        let current = group.iter().position(|item| *item == slot).unwrap_or(0);
        let next = group[wrapped_index(current, group.len(), delta)];
        self.select_slot(next, window, cx);
    }

    fn simple_behavior_label(input: &InputConfig, slot: usize) -> &'static str {
        match editor_logic::classify_simple_behavior(input, slot) {
            None => tr("custom_existing"),
            Some(SimpleBehaviorKind::ApplicationShortcut) => tr("beh_shortcuts"),
            Some(SimpleBehaviorKind::MacOs) => tr("beh_macos"),
            Some(SimpleBehaviorKind::Keystroke) => tr("beh_keystroke"),
            Some(SimpleBehaviorKind::App) => tr("beh_app"),
        }
    }

    fn cycle_simple_behavior(&mut self, delta: isize, cx: &mut Context<Self>) {
        let Some(slot) = self.host.selected_slot else {
            return;
        };
        let input = &mut self.host.active_profile_mut().inputs[slot];
        editor_logic::cycle_simple_behavior(
            input,
            slot,
            if delta < 0 {
                CycleDirection::Previous
            } else {
                CycleDirection::Next
            },
        );
        self.commit(true, cx);
    }

    fn open_shortcut_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let current = self.host.selected_slot.and_then(|slot| {
            match &self.host.active_profile().inputs[slot].behavior {
                Some(ControlBehavior::ApplicationShortcut { application, .. })
                    if behaviors::shortcut_application(application).is_some() =>
                {
                    Some(application.clone())
                }
                _ => None,
            }
        });
        self.shortcut_picker_app =
            current.unwrap_or_else(|| behaviors::APPLICATION_SHORTCUTS[0].id.to_string());
        if let Some(index) = sorted_shortcut_applications()
            .iter()
            .position(|app| app.id == self.shortcut_picker_app)
        {
            self.shortcut_rail_scroll.scroll_to_item(index);
        }
        self.shortcut_list_scroll.set_offset(point(px(0.), px(0.)));
        self.icon_query.clear();
        self.sheet = Sheet::ShortcutPicker;
        self.sync_inputs(window, cx);
        self.search_input.update(cx, |input, cx| {
            input.set_placeholder(tr("shortcut_search_placeholder"), window, cx)
        });
        cx.notify();
    }

    fn apply_shortcut_pick(&mut self, application: &str, shortcut: &str, cx: &mut Context<Self>) {
        let Some(slot) = self.host.selected_slot else {
            return;
        };
        if behaviors::apply_application_shortcut(
            &mut self.host.active_profile_mut().inputs[slot],
            application,
            shortcut,
        ) {
            self.sheet = Sheet::None;
            self.commit(true, cx);
        }
    }

    fn cycle_macos(&mut self, delta: isize, cx: &mut Context<Self>) {
        let Some(slot) = self.host.selected_slot else {
            return;
        };
        let command = match self.host.active_profile().inputs[slot].behavior {
            Some(ControlBehavior::MacOs { command }) => command,
            _ => behaviors::MACOS_PRESETS[0].command,
        };
        let current = behaviors::MACOS_PRESETS
            .iter()
            .position(|item| item.command == command)
            .unwrap_or(0);
        let preset = &behaviors::MACOS_PRESETS
            [wrapped_index(current, behaviors::MACOS_PRESETS.len(), delta)];
        behaviors::apply_macos(
            &mut self.host.active_profile_mut().inputs[slot],
            slot,
            preset.command,
        );
        self.commit(true, cx);
    }

    fn cycle_keyboard_usage(&mut self, delta: isize, cx: &mut Context<Self>) {
        let Some(slot) = self.host.selected_slot else {
            return;
        };
        let input = &self.host.active_profile().inputs[slot];
        let current = keycodes::KEYBOARD_USAGES
            .iter()
            .position(|item| item.usage == input.emitted.code)
            .unwrap_or(0);
        let usage = keycodes::KEYBOARD_USAGES
            [wrapped_index(current, keycodes::KEYBOARD_USAGES.len(), delta)]
        .usage;
        let mods = input.emitted.mods;
        behaviors::apply_keystroke(
            &mut self.host.active_profile_mut().inputs[slot],
            mods,
            usage,
        );
        self.commit(true, cx);
    }

    fn toggle_simple_modifier(&mut self, bit: u8, cx: &mut Context<Self>) {
        let Some(slot) = self.host.selected_slot else {
            return;
        };
        let input = &self.host.active_profile().inputs[slot];
        let mods = input.emitted.mods ^ bit;
        let key = input.emitted.code;
        behaviors::apply_keystroke(&mut self.host.active_profile_mut().inputs[slot], mods, key);
        self.commit(true, cx);
    }

    fn cycle_emitted_kind(&mut self, delta: isize, cx: &mut Context<Self>) {
        let Some(slot) = self.host.selected_slot else {
            return;
        };
        let input = &mut self.host.active_profile_mut().inputs[slot];
        let current = match input.emitted.kind {
            SlotKind::None => 0,
            SlotKind::Keyboard => 1,
            SlotKind::Consumer => 2,
        };
        input.emitted.kind = match wrapped_index(current, 3, delta) {
            0 => SlotKind::None,
            1 => SlotKind::Keyboard,
            _ => SlotKind::Consumer,
        };
        match input.emitted.kind {
            SlotKind::None => {
                input.emitted.mods = 0;
                input.emitted.code = 0;
            }
            SlotKind::Keyboard => input.emitted.code = 0x68,
            SlotKind::Consumer => {
                input.emitted.mods = 0;
                input.emitted.code = 0xcd;
            }
        }
        input.behavior = None;
        self.commit(true, cx);
    }

    fn cycle_emitted_code(&mut self, delta: isize, cx: &mut Context<Self>) {
        let Some(slot) = self.host.selected_slot else {
            return;
        };
        let input = &self.host.active_profile().inputs[slot];
        let next = match input.emitted.kind {
            SlotKind::None => return,
            SlotKind::Keyboard => {
                let current = keycodes::KEYBOARD_USAGES
                    .iter()
                    .position(|item| item.usage == input.emitted.code)
                    .unwrap_or(0);
                keycodes::KEYBOARD_USAGES
                    [wrapped_index(current, keycodes::KEYBOARD_USAGES.len(), delta)]
                .usage
            }
            SlotKind::Consumer => {
                let current = keycodes::CONSUMER_USAGES
                    .iter()
                    .position(|item| item.0 == input.emitted.code)
                    .unwrap_or(0);
                keycodes::CONSUMER_USAGES
                    [wrapped_index(current, keycodes::CONSUMER_USAGES.len(), delta)]
                .0
            }
        };
        self.host.active_profile_mut().inputs[slot].emitted.code = next;
        self.host.active_profile_mut().inputs[slot].behavior = None;
        self.commit(true, cx);
    }

    fn toggle_emitted_modifier(&mut self, bit: u8, cx: &mut Context<Self>) {
        let Some(slot) = self.host.selected_slot else {
            return;
        };
        let input = &mut self.host.active_profile_mut().inputs[slot];
        input.emitted.mods ^= bit;
        input.behavior = None;
        self.commit(true, cx);
    }

    fn cycle_action_kind(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(slot) = self.host.selected_slot else {
            return;
        };
        let current = match self.host.active_profile().inputs[slot].action {
            Action::None => 0,
            Action::Keystroke { .. } => 1,
            Action::Macro { .. } => 2,
            Action::Run { .. } => 3,
            Action::Open { .. } => 4,
            Action::Media { .. } => 5,
            Action::AppSettings => 6,
        };
        let input = &mut self.host.active_profile_mut().inputs[slot];
        input.action = match wrapped_index(current, 7, delta) {
            0 => Action::None,
            1 => Action::Keystroke { mods: 0, key: 0x04 },
            2 => Action::Macro { steps: Vec::new() },
            3 => Action::Run {
                command: String::new(),
            },
            4 => Action::Open {
                target: String::new(),
            },
            5 => Action::Media {
                op: MediaOp::PlayPause,
            },
            _ => Action::AppSettings,
        };
        input.behavior = None;
        self.commit(false, cx);
        self.sync_inputs(window, cx);
    }

    fn cycle_media_action(&mut self, delta: isize, cx: &mut Context<Self>) {
        const OPS: [MediaOp; 8] = [
            MediaOp::VolumeUp,
            MediaOp::VolumeDown,
            MediaOp::Mute,
            MediaOp::PlayPause,
            MediaOp::NextTrack,
            MediaOp::PrevTrack,
            MediaOp::BrightnessUp,
            MediaOp::BrightnessDown,
        ];
        let Some(slot) = self.host.selected_slot else {
            return;
        };
        let current = match self.host.active_profile().inputs[slot].action {
            Action::Media { op } => OPS.iter().position(|item| *item == op).unwrap_or(0),
            _ => return,
        };
        self.host.active_profile_mut().inputs[slot].action = Action::Media {
            op: OPS[wrapped_index(current, OPS.len(), delta)],
        };
        self.commit(false, cx);
    }

    fn cycle_language(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        const LANGUAGES: [LanguageSetting; 5] = [
            LanguageSetting::Auto,
            LanguageSetting::En,
            LanguageSetting::ZhHans,
            LanguageSetting::ZhHant,
            LanguageSetting::Ja,
        ];
        let current = LANGUAGES
            .iter()
            .position(|item| *item == self.host.config.language)
            .unwrap_or(0);
        self.host.config.language = LANGUAGES[wrapped_index(current, LANGUAGES.len(), delta)];
        i18n::set_lang(resolve_language(self.host.config.language));
        self.agent_integration_feedback = None;
        self.commit(false, cx);
        self.label_input.update(cx, |input, cx| {
            input.set_placeholder(tr("keycap_label_placeholder"), window, cx)
        });
        self.profile_input.update(cx, |input, cx| {
            input.set_placeholder(tr("profile_name_placeholder"), window, cx)
        });
        self.command_input.update(cx, |input, cx| {
            input.set_placeholder(tr("shell_command_placeholder"), window, cx)
        });
        self.target_input.update(cx, |input, cx| {
            input.set_placeholder(tr("open_placeholder"), window, cx)
        });
        self.search_input.update(cx, |input, cx| {
            input.set_placeholder(tr("icon_search_placeholder"), window, cx)
        });
    }

    fn apply_configured_theme(&self, window: &mut Window, cx: &mut Context<Self>) {
        let scheme = resolve_theme(self.host.config.theme, window.appearance());
        if scheme != pixel::color_scheme() {
            pixel::install_theme(scheme, cx);
        }
    }

    fn cycle_theme(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        const THEMES: [ThemeSetting; 3] = [
            ThemeSetting::System,
            ThemeSetting::Light,
            ThemeSetting::Dark,
        ];
        let current = THEMES
            .iter()
            .position(|item| *item == self.host.config.theme)
            .unwrap_or(0);
        self.host.config.theme = THEMES[wrapped_index(current, THEMES.len(), delta)];
        self.apply_configured_theme(window, cx);
        self.commit(false, cx);
    }

    fn language_label(&self) -> &'static str {
        match self.host.config.language {
            LanguageSetting::Auto => tr("language_auto"),
            LanguageSetting::En => "English",
            LanguageSetting::ZhHans => "简体中文",
            LanguageSetting::ZhHant => "繁體中文",
            LanguageSetting::Ja => "日本語",
        }
    }

    fn theme_label(&self) -> &'static str {
        match self.host.config.theme {
            ThemeSetting::System => tr("theme_system"),
            ThemeSetting::Light => tr("theme_light"),
            ThemeSetting::Dark => tr("theme_dark"),
        }
    }

    fn pattern_label(pattern: LedPattern) -> String {
        match pattern {
            LedPattern::Rainbow => tr("pat_rainbow").to_string(),
            LedPattern::White => tr("pat_white").to_string(),
            LedPattern::Solid { r, g, b } => PATTERN_PALETTE
                .iter()
                .find(|(_, pr, pg, pb)| (*pr, *pg, *pb) == (r, g, b))
                .map(|(key, ..)| tr(key).to_string())
                .unwrap_or_else(|| tr("custom_existing").to_string()),
        }
    }

    fn status_color_label(pattern: LedPattern) -> String {
        match pattern {
            // Keep the original runtime defaults human-readable even though
            // their tuned RGB values are a little softer than the idle
            // palette presets.
            LedPattern::Solid { r: 0, g: 96, b: 255 } => tr("pat_blue").to_string(),
            LedPattern::Solid {
                r: 255,
                g: 150,
                b: 0,
            } => tr("pat_yellow").to_string(),
            LedPattern::Solid { r: 0, g: 210, b: 90 } => tr("pat_green").to_string(),
            LedPattern::Solid { r: 255, g: 30, b: 50 } => tr("pat_red").to_string(),
            other => Self::pattern_label(other),
        }
    }

    fn status_color_value(pattern: LedPattern) -> String {
        match pattern {
            LedPattern::Solid { r, g, b } => format!(
                "{}  #{r:02X}{g:02X}{b:02X}",
                Self::status_color_label(pattern)
            ),
            other => Self::status_color_label(other),
        }
    }

    fn cycle_pattern(&mut self, key_chain: bool, delta: i32, cx: &mut Context<Self>) {
        let n = (PATTERN_PALETTE.len() + 2) as i32;
        let current = if key_chain {
            self.host.config.led_key_pattern
        } else {
            self.host.config.led_ambient_pattern
        };
        let index = match current {
            LedPattern::Rainbow => 0,
            LedPattern::White => 1,
            LedPattern::Solid { r, g, b } => PATTERN_PALETTE
                .iter()
                .position(|(_, pr, pg, pb)| (*pr, *pg, *pb) == (r, g, b))
                .map(|i| i as i32 + 2)
                .unwrap_or(0),
        };
        let next = (index + delta).rem_euclid(n);
        let pattern = match next {
            0 => LedPattern::Rainbow,
            1 => LedPattern::White,
            i => {
                let (_, r, g, b) = PATTERN_PALETTE[(i - 2) as usize];
                LedPattern::Solid { r, g, b }
            }
        };
        if key_chain {
            self.host.config.led_key_pattern = pattern;
        } else {
            self.host.config.led_ambient_pattern = pattern;
        }
        self.commit(true, cx);
    }

    fn cycle_status_color(
        &mut self,
        status: ActivityStatus,
        delta: isize,
        cx: &mut Context<Self>,
    ) {
        let current = self.host.config.activity_status_colors.get(status);
        let current_index = PATTERN_PALETTE
            .iter()
            .position(|(_, r, g, b)| {
                matches!(current, LedPattern::Solid { r: cr, g: cg, b: cb } if (cr, cg, cb) == (*r, *g, *b))
            })
            .unwrap_or_else(|| {
                // Tuned defaults (and imported custom colours) start from
                // the nearest named preset when the user first cycles them.
                let (cr, cg, cb) = match current {
                    LedPattern::Solid { r, g, b } => (r as i32, g as i32, b as i32),
                    LedPattern::White => (255, 255, 255),
                    LedPattern::Rainbow => (0, 80, 255),
                };
                PATTERN_PALETTE
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, (_, r, g, b))| {
                        let dr = cr - *r as i32;
                        let dg = cg - *g as i32;
                        let db = cb - *b as i32;
                        dr * dr + dg * dg + db * db
                    })
                    .map(|(index, _)| index)
                    .unwrap_or(0)
            });
        let next = wrapped_index(current_index, PATTERN_PALETTE.len(), delta);
        let (_, r, g, b) = PATTERN_PALETTE[next];
        self.host.config.activity_status_colors.set(
            status,
            LedPattern::Solid { r, g, b },
        );
        self.host.refresh_activity_led();
        self.commit(false, cx);
    }

    fn refresh_agent_integrations(&mut self) {
        self.agent_integrations = agent_integrations::scan_system_all();
    }

    fn integration_note(kind: IntegrationKind) -> &'static str {
        match kind {
            IntegrationKind::Codex => tr("agent_codex_integration_note"),
            IntegrationKind::ClaudeCode => tr("agent_claude_code_integration_note"),
            IntegrationKind::OpenCode => tr("agent_opencode_integration_note"),
            IntegrationKind::DeepCode => tr("agent_deep_code_integration_note"),
        }
    }

    fn integration_state_label(state: InstallState) -> (&'static str, BadgeTone) {
        match state {
            InstallState::NotInstalled => (tr("integration_not_installed"), BadgeTone::Neutral),
            InstallState::Installed => (tr("integration_installed"), BadgeTone::Success),
            InstallState::NeedsUpdate => (tr("integration_needs_update"), BadgeTone::Warning),
            InstallState::Conflict => (tr("integration_conflict"), BadgeTone::Danger),
            InstallState::Unavailable => (tr("integration_unavailable"), BadgeTone::Neutral),
        }
    }

    fn install_agent_integration(
        &mut self,
        kind: IntegrationKind,
        cx: &mut Context<Self>,
    ) {
        let name = kind.display_name();
        match agent_integrations::install_system(kind) {
            Ok(receipt) => {
                let outcome = match receipt.disposition {
                    InstallDisposition::AlreadyInstalled => tr("integration_already_installed"),
                    InstallDisposition::Installed => tr("integration_install_success"),
                    InstallDisposition::Updated => tr("integration_update_success"),
                };
                let backup_detail = if receipt.backups.is_empty() {
                    String::new()
                } else {
                    format!(
                        " · {}: {}",
                        tr("integration_backups"),
                        receipt
                            .backups
                            .iter()
                            .map(|path| path.display().to_string())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                };
                self.agent_integration_feedback = Some((
                    format!(
                        "{outcome}: {name} · {} · {}{backup_detail}",
                        Self::integration_note(kind),
                        tr("integration_restart_agent")
                    ),
                    BadgeTone::Success,
                ));
            }
            Err(error) => {
                self.agent_integration_feedback = Some((
                    format!("{}: {name} — {error}", tr("integration_install_failed")),
                    BadgeTone::Danger,
                ));
            }
        }
        self.refresh_agent_integrations();
        cx.notify();
    }

    fn render_agent_integrations(&self, cx: &mut Context<Self>) -> Div {
        let mut integrations = div()
            .w_full()
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(
                div()
                    .font_family("Monaco")
                    .text_size(px(11.))
                    .font_semibold()
                    .text_color(pixel::accent_highlight_color())
                    .child(tr("agent_integrations")),
            )
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(pixel::muted_text_color())
                    .child(tr("agent_integrations_note")),
            );

        if self.agent_integrations.iter().all(|report| {
            report.state == InstallState::Unavailable
        }) {
            if let Some(detail) = self
                .agent_integrations
                .first()
                .and_then(|report| report.detail.clone())
            {
                return integrations.child(controls::status_rail(
                    tr("integration_unavailable"),
                    detail,
                    BadgeTone::Warning,
                ));
            }
        }

        for (index, report) in self.agent_integrations.iter().enumerate() {
            let kind = report.kind;
            let (state_label, state_tone) = Self::integration_state_label(report.state);
            let target = if report.target.as_os_str().is_empty() {
                "—".to_string()
            } else {
                report.target.display().to_string()
            };
            let target_tooltip = SharedString::from(target.clone());
            let mut actions = div()
                .flex_shrink_0()
                .flex()
                .items_center()
                .gap(px(6.))
                .child(pixel::badge(state_label, state_tone));
            match report.state {
                InstallState::NotInstalled => {
                    actions = actions.child(
                        tiny_button(tr("integration_install"))
                            .id(("install-agent-integration", index))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.install_agent_integration(kind, cx)
                            })),
                    );
                }
                InstallState::NeedsUpdate => {
                    actions = actions.child(
                        tiny_button(tr("integration_reinstall"))
                            .id(("update-agent-integration", index))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.install_agent_integration(kind, cx)
                            })),
                    );
                }
                InstallState::Installed
                | InstallState::Conflict
                | InstallState::Unavailable => {}
            }

            let row = div()
                .w_full()
                .min_h(px(40.))
                .px(px(10.))
                .flex()
                .items_center()
                .gap(px(8.))
                .bg(pixel::canvas_color())
                .rounded(px(2.))
                .child(
                    div()
                        .id(("agent-integration-target", index))
                        .flex_1()
                        .min_w(px(0.))
                        .truncate()
                        .font_family("Monaco")
                        .text_size(px(10.))
                        .text_color(pixel::dim_text_color())
                        .child(target)
                        .tooltip(move |_, cx| {
                            cx.new(|_| Tooltip::new(target_tooltip.clone())).into()
                        }),
                )
                .child(actions);
            let mut control = div().w_full().flex().flex_col().gap(px(4.)).child(row);
            if let Some(detail) = report.detail.clone() {
                control = control.child(controls::status_rail(
                    state_label,
                    detail,
                    state_tone,
                ));
            }
            integrations = integrations.child(inspector_field(
                kind.display_name(),
                Self::integration_note(kind),
                control,
            ));
        }

        if let Some((message, tone)) = self.agent_integration_feedback.clone() {
            integrations = integrations.child(controls::status_rail(
                tr("agent_integrations"),
                message,
                tone,
            ));
        }
        integrations
    }

    fn open_key_picker(&mut self, target: KeyTarget, cx: &mut Context<Self>) {
        self.key_picker_target = target;
        self.sheet = Sheet::KeyPicker;
        cx.notify();
    }

    fn apply_picked_key(&mut self, pick: PickedKey, cx: &mut Context<Self>) {
        let Some(slot) = self.host.selected_slot else {
            return;
        };
        let existing_mods = self.host.active_profile().inputs[slot].emitted.mods;
        // A modifier cap assigns a bare-modifier hold; a normal cap keeps
        // any chord modifiers already configured on the slot.
        let (mods, code) = match pick {
            PickedKey::Usage(usage) => (existing_mods, usage),
            PickedKey::Modifier(bit) => (bit, 0),
        };
        match self.key_picker_target {
            KeyTarget::SimpleKey => {
                behaviors::apply_keystroke(
                    &mut self.host.active_profile_mut().inputs[slot],
                    mods,
                    code,
                );
            }
            KeyTarget::EmittedCode => {
                let input = &mut self.host.active_profile_mut().inputs[slot];
                input.emitted.kind = SlotKind::Keyboard;
                input.emitted.mods = mods;
                input.emitted.code = code;
                input.behavior = None;
            }
        }
        self.sheet = Sheet::None;
        self.commit(true, cx);
    }

    fn render_key_picker_sheet(&self, cx: &mut Context<Self>) -> Div {
        let mut body = div().w_full().flex().flex_col().gap(px(5.));
        for (row_index, row) in KEY_PICKER_ROWS.iter().enumerate() {
            let mut line = div().w_full().flex().gap(px(4.));
            for (col_index, (label, key, width)) in row.iter().enumerate() {
                let pick = *key;
                line = line.child(
                    div()
                        .h(px(30.))
                        .w(px(30. * width + 4. * (width - 1.)))
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(pixel::raised_color())
                        .rounded(px(2.))
                        .cursor_pointer()
                        .hover(|style| style.bg(pixel::key_color()))
                        .font_family("Monaco")
                        .text_size(px(10.))
                        .text_color(pixel::text_color())
                        .id(("picker-key", row_index * 32 + col_index))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.apply_picked_key(pick, cx)
                        }))
                        .child(SharedString::from(*label)),
                );
            }
            body = body.child(line);
        }
        controls::modal_frame()
            .child(controls::modal_header(
                tr("pick_a_key"),
                Some(tr("key_picker_note").into()),
            ))
            .child(div().w_full().p(px(16.)).child(body))
            .child(
                div().w_full().px(px(16.)).pb(px(14.)).flex().justify_end().child(
                    tiny_button(tr("cancel"))
                        .id("key-picker-cancel")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.sheet = Sheet::None;
                            cx.notify();
                        })),
                ),
            )
    }

    fn adjust_brightness(&mut self, delta: i16, cx: &mut Context<Self>) {
        let current = self.host.config.led_brightness as i16;
        self.host.config.led_brightness = (current + delta).clamp(0, 255) as u8;
        if let Some(tx) = &self.host.device_tx {
            let _ = tx.send(DeviceCmd::SetLedBrightness {
                brightness: self.host.config.led_brightness,
            });
        }
        self.commit(true, cx);
    }

    /// Native open-file panel. This must stay asynchronous: a blocking dialog
    /// (rfd's `pick_file`) spins the main run loop inside the gpui handler
    /// while `App` is mutably borrowed, and the first gpui task the loop
    /// drains aborts with a `BorrowMutError`.
    fn pick_file(
        window: &Window,
        cx: &mut Context<Self>,
        on_pick: impl FnOnce(&mut Self, PathBuf, &mut Window, &mut Context<Self>) + 'static,
    ) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: None,
        });
        cx.spawn_in(window, async move |weak, cx| {
            if let Ok(Ok(Some(mut paths))) = paths.await {
                if let Some(path) = paths.pop() {
                    let _ = weak.update_in(cx, |this, window, cx| {
                        on_pick(this, path, window, cx);
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    fn choose_firmware_image(&mut self, window: &Window, cx: &mut Context<Self>) {
        Self::pick_file(window, cx, |this, path, _, _| {
            // The panel has no extension filter; keep rfd's old .bin guarantee.
            if path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("bin"))
            {
                this.host.firmware_image = Some(path);
                this.host.firmware_expected_version = None;
                this.host.update_error = None;
            } else {
                this.host.update_error = Some("Choose a firmware .bin first.".into());
            }
        });
    }

    fn install_firmware(&mut self, cx: &mut Context<Self>) {
        if self.host.updating || self.host.firmware_downloading {
            return;
        }
        if let Some(path) = self.host.firmware_image.clone() {
            let expected = self.host.firmware_expected_version.clone();
            let _ = self.host.start_firmware_update(path, expected);
            cx.notify();
            return;
        }
        if let Some(catalog) = self.host.release.as_ref() {
            let version = catalog.firmware.version.clone();
            let asset = catalog.firmware.asset.clone();
            self.host.firmware_downloading = true;
            self.host.firmware_download_progress = 0.0;
            self.host.install_after_download = true;
            release::spawn_download(DownloadKind::Firmware, version, asset);
            cx.notify();
            return;
        }
        match release::bundled_firmware() {
            Ok(Some((version, path))) => {
                let _ = self.host.start_firmware_update(path, Some(version));
            }
            Ok(None) => {
                self.host.update_error = Some("Choose a firmware .bin first.".into());
            }
            Err(error) => self.host.update_error = Some(error),
        }
        cx.notify();
    }

    fn import_config(
        &mut self,
        mode: config::ImportMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        Self::pick_file(window, cx, move |this, path, window, cx| {
            match config::import_from(&path, mode, &mut this.host.config) {
                Ok(detail) => {
                    this.push_log(detail);
                    let _ = this.host.persist();
                    let _ = this.host.sync_device();
                    this.sync_inputs(window, cx);
                    this.apply_configured_theme(window, cx);
                }
                Err(error) => this.push_log(format!("import failed: {error}")),
            }
        });
    }

    fn export_config(&mut self, cx: &mut Context<Self>) {
        // Async for the same reason as pick_file.
        let directory = dirs::download_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| PathBuf::from("."));
        let path = cx.prompt_for_new_path(&directory, Some("openmicro-profiles.json"));
        cx.spawn(async move |weak, cx| {
            if let Ok(Ok(Some(path))) = path.await {
                let _ = weak.update(cx, |this, cx| {
                    match config::export_to(&path, &this.host.config) {
                        Ok(()) => this.push_log(format!("exported {}", path.display())),
                        Err(error) => this.push_log(format!("export failed: {error}")),
                    }
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn delete_active_profile(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.host.config.profiles.len() <= 1 {
            return;
        }
        if !self.confirm_delete {
            self.confirm_delete = true;
            cx.notify();
            return;
        }
        self.confirm_delete = false;
        let index = self.host.config.active_profile;
        self.host.config.profiles.remove(index);
        self.host.config.active_profile = index.min(self.host.config.profiles.len() - 1);
        let _ = self.host.persist();
        let _ = self.host.sync_device();
        self.sync_inputs(window, cx);
        cx.notify();
    }

    fn reset_factory(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.confirm_reset {
            self.confirm_reset = true;
            cx.notify();
            return;
        }
        self.confirm_reset = false;
        config::factory_reset(&mut self.host.config);
        let _ = self.host.persist();
        let _ = self.host.sync_device();
        self.sync_inputs(window, cx);
        self.apply_configured_theme(window, cx);
        cx.notify();
    }

    fn macro_step_value(step: &MacroStep) -> String {
        match step {
            MacroStep::Keystroke { mods, key } => {
                let name = keycodes::keyboard_name(*key).unwrap_or("Unknown key");
                let modifiers = keycodes::mods_label(*mods);
                if modifiers.is_empty() {
                    name.to_string()
                } else {
                    format!("{modifiers} + {name}")
                }
            }
            MacroStep::Delay { ms } => ms.to_string(),
            MacroStep::Run { command } => command.clone(),
            MacroStep::Open { target } => target.clone(),
            MacroStep::Media { op } => media_label(*op).to_string(),
        }
    }

    fn macro_step_kind(step: &MacroStep) -> &'static str {
        match step {
            MacroStep::Keystroke { .. } => tr("act_keystroke"),
            MacroStep::Delay { .. } => "Delay",
            MacroStep::Run { .. } => tr("act_run"),
            MacroStep::Open { .. } => tr("act_open"),
            MacroStep::Media { .. } => tr("act_media"),
        }
    }

    fn sync_macro_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let value = self
            .macro_edit_index
            .and_then(|index| self.macro_draft.get(index))
            .map(|entry| Self::macro_step_value(&entry.step))
            .unwrap_or_default();
        self.syncing_inputs = true;
        self.macro_value_input
            .update(cx, |input, cx| input.set_value(value, window, cx));
        self.syncing_inputs = false;
    }

    fn select_macro_step(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index < self.macro_draft.len() {
            self.macro_edit_index = Some(index);
            self.sync_macro_input(window, cx);
            cx.notify();
        }
    }

    fn add_macro_step(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.macro_draft.push(MacroStepEntry {
            enabled: true,
            step: MacroStep::Delay { ms: 100 },
        });
        self.macro_edit_index = Some(self.macro_draft.len() - 1);
        self.sync_macro_input(window, cx);
        cx.notify();
    }

    fn cycle_macro_kind(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.macro_edit_index else {
            return;
        };
        let Some(entry) = self.macro_draft.get_mut(index) else {
            return;
        };
        let current = match entry.step {
            MacroStep::Keystroke { .. } => 0,
            MacroStep::Delay { .. } => 1,
            MacroStep::Run { .. } => 2,
            MacroStep::Open { .. } => 3,
            MacroStep::Media { .. } => 4,
        };
        entry.step = match wrapped_index(current, 5, delta) {
            0 => MacroStep::Keystroke { mods: 0, key: 0x04 },
            1 => MacroStep::Delay { ms: 100 },
            2 => MacroStep::Run {
                command: String::new(),
            },
            3 => MacroStep::Open {
                target: String::new(),
            },
            _ => MacroStep::Media {
                op: MediaOp::PlayPause,
            },
        };
        self.recording = RecordTarget::None;
        self.sync_macro_input(window, cx);
        cx.notify();
    }

    fn cycle_macro_media(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        const OPS: [MediaOp; 8] = [
            MediaOp::VolumeUp,
            MediaOp::VolumeDown,
            MediaOp::Mute,
            MediaOp::PlayPause,
            MediaOp::NextTrack,
            MediaOp::PrevTrack,
            MediaOp::BrightnessUp,
            MediaOp::BrightnessDown,
        ];
        let Some(index) = self.macro_edit_index else {
            return;
        };
        let Some(entry) = self.macro_draft.get_mut(index) else {
            return;
        };
        let MacroStep::Media { op } = entry.step else {
            return;
        };
        let current = OPS.iter().position(|item| *item == op).unwrap_or(0);
        entry.step = MacroStep::Media {
            op: OPS[wrapped_index(current, OPS.len(), delta)],
        };
        self.sync_macro_input(window, cx);
        cx.notify();
    }

    fn remove_macro_step(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.macro_edit_index else {
            return;
        };
        if index < self.macro_draft.len() {
            self.macro_draft.remove(index);
        }
        self.macro_edit_index = if self.macro_draft.is_empty() {
            None
        } else {
            Some(index.min(self.macro_draft.len() - 1))
        };
        self.sync_macro_input(window, cx);
        cx.notify();
    }

    fn move_macro_step(&mut self, delta: isize, cx: &mut Context<Self>) {
        let Some(index) = self.macro_edit_index else {
            return;
        };
        if self.macro_draft.len() < 2 {
            return;
        }
        let next = (index as isize + delta).clamp(0, self.macro_draft.len() as isize - 1) as usize;
        if next != index {
            self.macro_draft.swap(index, next);
            self.macro_edit_index = Some(next);
            cx.notify();
        }
    }

    fn save_macro(&mut self, cx: &mut Context<Self>) {
        if let Some(slot) = self.host.selected_slot {
            self.host.active_profile_mut().inputs[slot].action = Action::Macro {
                steps: self.macro_draft.clone(),
            };
            self.commit(false, cx);
        }
        self.recording = RecordTarget::None;
        self.sheet = Sheet::None;
        cx.notify();
    }

    fn render_header(&self, cx: &mut Context<Self>) -> TitleBar {
        let profile = self.host.active_profile().name.clone();
        let (connection, connection_color) = if self.host.connected {
            (tr("connected"), pixel::success_color())
        } else {
            (tr("editing_offline"), pixel::dim_text_color())
        };

        TitleBar::new()
            .pr(px(10.))
            .bg(pixel::panel_color())
            .border_color(pixel::border_color())
            .child(
                div()
                    .w_full()
                    .h_full()
                    .flex()
                    .items_center()
                    .gap(px(9.))
                    .child(logo_mark())
                    .child(
                        div()
                            .font_family("Monaco")
                            .font_semibold()
                            .text_size(px(12.))
                            .text_color(pixel::text_color())
                            .child("OPENMICRO"),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .h(px(22.))
                            .px(px(7.))
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .child(div().w(px(6.)).h(px(6.)).bg(connection_color))
                            .child(
                                div()
                                    .text_size(px(10.))
                                    .text_color(pixel::muted_text_color())
                                    .child(connection),
                            ),
                    )
                    .child(
                        chrome_icon_button("chevron-left")
                            .id("profile-previous")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.cycle_profile(-1, window, cx)
                            })),
                    )
                    .child(
                        div()
                            .w(px(154.))
                            .h(px(26.))
                            .min_w(px(0.))
                            .px(px(10.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .truncate()
                            .text_size(px(12.))
                            .font_semibold()
                            .text_color(pixel::text_color())
                            .child(profile),
                    )
                    .child(
                        chrome_icon_button("chevron-right")
                            .id("profile-next")
                            .on_click(
                                cx.listener(|this, _, window, cx| {
                                    this.cycle_profile(1, window, cx)
                                }),
                            ),
                    )
                    .child(
                        chrome_icon_button("plus")
                            .id("profile-add")
                            .on_click(cx.listener(|this, _, window, cx| {
                                let number = this.host.config.profiles.len() + 1;
                                let mut profile = config::default_codex_profile();
                                profile.name = format!("Profile {number}");
                                this.host.config.profiles.push(profile);
                                let index = this.host.config.profiles.len() - 1;
                                this.switch_profile(index, window, cx);
                            })),
                    )
                    .child(chrome_icon_button("settings").id("open-settings").on_click(
                        cx.listener(|this, _, _, cx| {
                            this.agent_integration_feedback = None;
                            this.refresh_agent_integrations();
                            this.sheet = Sheet::Settings;
                            cx.notify();
                        }),
                    )),
            )
    }

    fn render_banners(&self, cx: &mut Context<Self>) -> Div {
        let mut banners = div().w_full().flex().flex_col();
        if let Some(catalog) = &self.host.release {
            let app_available = release::is_newer(&catalog.app.version, release::APP_VERSION)
                && !self.host.app_banner_dismissed;
            if app_available {
                let automatic = self.app_updater.uses_signed_updates();
                let controls = app_update_controls(
                    automatic,
                    self.app_updater_active,
                    self.host.app_downloading,
                    self.host.app_download.is_some(),
                    self.host.app_update_error.is_some(),
                );
                let progress =
                    (self.host.app_download_progress * 100.0).round() as u32;
                let detail = match controls.detail {
                    AppUpdateDetailState::Error => format!(
                        "{} · {}",
                        tr("app_update_failed"),
                        self.host.app_update_error.as_deref().unwrap_or_default()
                    ),
                    AppUpdateDetailState::ManualDownloading => format!(
                        "OpenMicro {} · {} {}%",
                        catalog.app.version,
                        tr("app_update_downloading_dmg"),
                        progress
                    ),
                    AppUpdateDetailState::ManualReady => format!(
                        "OpenMicro {} · {}",
                        catalog.app.version,
                        tr("app_update_ready")
                    ),
                    AppUpdateDetailState::SparkleActive => format!(
                        "OpenMicro {} · {}",
                        catalog.app.version,
                        tr("app_update_in_progress")
                    ),
                    AppUpdateDetailState::Available => format!(
                        "OpenMicro {} · {}",
                        catalog.app.version,
                        tr("app_update_available")
                    ),
                };

                let mut actions = div()
                    .flex()
                    .flex_shrink_0()
                    .items_center()
                    .gap(px(6.));
                if let Some(sparkle) = controls.sparkle {
                    actions = actions.child(match sparkle {
                        AppUpdateButtonState::StartSparkle => tiny_button(tr("app_update_action"))
                            .id("start-app-self-update")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.begin_sparkle_update(cx)
                            }))
                            .into_any_element(),
                        AppUpdateButtonState::SparkleBusy => {
                            paging_button(tr("app_update_in_progress"), false).into_any_element()
                        }
                        _ => unreachable!("invalid Sparkle update control"),
                    });
                }
                if let Some(manual) = controls.manual {
                    actions = actions.child(match manual {
                        AppUpdateButtonState::DownloadDmg => {
                            tiny_button(tr("app_update_download_dmg"))
                                .id("download-app-update")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.begin_manual_app_download(cx)
                                }))
                                .into_any_element()
                        }
                        AppUpdateButtonState::DownloadingDmg => paging_button(
                            format!("{} {}%", tr("app_update_downloading_dmg"), progress),
                            false,
                        )
                        .into_any_element(),
                        AppUpdateButtonState::OpenDmg => tiny_button(tr("app_update_open_dmg"))
                            .id("open-app-update")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.open_manual_app_download(cx)
                            }))
                            .into_any_element(),
                        _ => unreachable!("invalid manual update control"),
                    });
                }
                if controls.dismissible {
                    actions = actions.child(
                        tiny_button(tr("later"))
                            .id("dismiss-app-update")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.host.app_banner_dismissed = true;
                                cx.notify();
                            })),
                    );
                }
                banners = banners.child(
                    div()
                        .min_h(px(44.))
                        .px(px(18.))
                        .flex()
                        .items_center()
                        .gap(px(12.))
                        .bg(pixel::raised_color())
                        .border_b_1()
                        .border_color(pixel::accent_color())
                        .child(pixel::badge(tr("app_update_badge"), BadgeTone::Accent))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .truncate()
                                .text_size(px(13.))
                                .text_color(pixel::text_color())
                                .child(detail),
                        )
                        .child(actions),
                );
            } else if let Some((installed, _)) = self.host.last_conn.as_ref() {
                let firmware_available = release::is_newer(&catalog.firmware.version, installed)
                    && !self.host.firmware_banner_dismissed;
                if firmware_available {
                    let detail = format!(
                        "Firmware {} available // installed {}",
                        catalog.firmware.version, installed
                    );
                    banners = banners.child(
                        div()
                            .min_h(px(44.))
                            .px(px(18.))
                            .flex()
                            .items_center()
                            .gap(px(12.))
                            .bg(pixel::raised_color())
                            .border_b_1()
                            .border_color(pixel::accent_color())
                            .child(pixel::badge("FW UPDATE", BadgeTone::Warning))
                            .child(
                                div()
                                    .flex_1()
                                    .text_size(px(13.))
                                    .text_color(pixel::text_color())
                                    .child(detail),
                            )
                            .child(
                                tiny_button(tr("update_now"))
                                    .id("open-firmware-update")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.sheet = Sheet::Firmware;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                tiny_button(tr("later"))
                                    .id("dismiss-firmware-update")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.host.firmware_banner_dismissed = true;
                                        cx.notify();
                                    })),
                            ),
                    );
                }
            }
        }

        if let Some(slot) = self.host.selected_slot {
            let action = &self.host.active_profile().inputs[slot].action;
            if actions::needs_permission(action) && !actions::accessibility_trusted() {
                banners = banners.child(
                    div()
                        .min_h(px(42.))
                        .px(px(18.))
                        .flex()
                        .items_center()
                        .gap(px(12.))
                        .bg(pixel::canvas_color())
                        .border_b_1()
                        .border_color(pixel::danger_color())
                        .child(pixel::badge("PERMISSION", BadgeTone::Danger))
                        .child(
                            div()
                                .flex_1()
                                .text_size(px(13.))
                                .text_color(pixel::text_color())
                                .child(tr("perm_banner")),
                        )
                        .child(
                            tiny_button(tr("open_settings"))
                                .id("open-permission-settings")
                                .on_click(|_, _, _| actions::open_permission_settings()),
                        ),
                );
            }
        }
        banners
    }

    fn render_device_cell(&self, cell: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let slots = slots_for_cell(cell);
        let primary = slots[0];
        let input = &self.host.active_profile().inputs[primary];
        let selected = self
            .host
            .selected_slot
            .is_some_and(|slot| slots.contains(&slot));
        let warning = self.host.intercept.as_ref().is_some_and(|intercept| {
            use crate::intercept::SlotStatus;
            slots.iter().any(|slot| {
                matches!(
                    intercept.status[*slot],
                    SlotStatus::DeadOnThisOs
                        | SlotStatus::NothingEmitted
                        | SlotStatus::Failed
                        | SlotStatus::Unavailable
                )
            })
        });
        let live = self.host.pressed_cells[cell];
        let icon = configured_icon_visual(
            &input.icon,
            15.,
            if selected {
                pixel::accent_color()
            } else {
                pixel::muted_text_color()
            },
        );
        let label = match cell {
            0..=12 => {
                if input.label.trim().is_empty() {
                    format!("KEY {:02}", cell + 1)
                } else {
                    short_text(&input.label, 12)
                }
            }
            CELL_ENCODER => tr("dial_rotator").to_string(),
            CELL_JOYSTICK => tr("dial_joystick").to_string(),
            _ => tr("dial_touch").to_string(),
        };
        let detail = match cell {
            0..=12 => {
                if input.action == Action::None {
                    keycodes::slot_label(&input.emitted)
                } else {
                    actions::describe(&input.action)
                }
            }
            CELL_ENCODER => RotatorRotationPreset::infer(self.host.active_profile())
                .map(|preset| preset.label().to_string())
                .unwrap_or_else(|| "Custom rotation".into()),
            CELL_JOYSTICK => JoystickMode::infer(self.host.active_profile())
                .label()
                .to_string(),
            _ => {
                if input.action == Action::None {
                    keycodes::slot_label(&input.emitted)
                } else {
                    actions::describe(&input.action)
                }
            }
        };
        controls::keycap_device_cell(
            label,
            icon,
            short_text(&detail, 18),
            CellVisual {
                selected,
                live,
                warning,
            },
        )
        .id(("device-cell", cell))
        .cursor_pointer()
        .on_click(cx.listener(move |this, _, window, cx| this.select_slot(primary, window, cx)))
    }

    fn render_device_map(&self, cx: &mut Context<Self>) -> Div {
        let board = div()
            .w(px(440.))
            .flex_none()
            .flex()
            .flex_col()
            .gap(px(8.))
            .p(px(16.))
            .bg(pixel::deck_color())
            .rounded(px(3.))
            .child(
                div()
                    .w_full()
                    .flex()
                    .gap(px(8.))
                    .child(self.render_device_cell(CELL_ENCODER, cx))
                    .child(self.render_device_cell(0, cx))
                    .child(self.render_device_cell(1, cx))
                    .child(self.render_device_cell(CELL_JOYSTICK, cx)),
            )
            .child(
                div()
                    .w_full()
                    .flex()
                    .gap(px(8.))
                    .child(self.render_device_cell(2, cx))
                    .child(self.render_device_cell(3, cx))
                    .child(self.render_device_cell(4, cx))
                    .child(self.render_device_cell(5, cx)),
            )
            .child(
                div()
                    .w_full()
                    .flex()
                    .gap(px(8.))
                    .child(self.render_device_cell(6, cx))
                    .child(self.render_device_cell(7, cx))
                    .child(self.render_device_cell(8, cx))
                    .child(self.render_device_cell(9, cx)),
            )
            .child(
                div()
                    .w_full()
                    .flex()
                    .gap(px(8.))
                    .child(self.render_device_cell(CELL_TOUCH, cx))
                    .child(self.render_device_cell(10, cx))
                    .child(self.render_device_cell(11, cx))
                    .child(self.render_device_cell(12, cx)),
            );

        div()
            .h_full()
            .flex_1()
            .min_w(px(440.))
            .min_h(px(0.))
            .overflow_hidden()
            .flex()
            .items_center()
            .justify_center()
            .child(board)
    }

    fn render_rotator_editor(&self, cx: &mut Context<Self>) -> Div {
        let rotation = RotatorRotationPreset::infer(self.host.active_profile());
        let press = RotatorPressPreset::infer(self.host.active_profile());
        let rotation_label = rotation
            .map(|preset| preset.label())
            .unwrap_or(tr("custom_existing"));
        let press_label = press
            .map(|preset| preset.label())
            .unwrap_or(tr("custom_existing"));

        div()
            .w_full()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(pixel::section_header(1, tr("choose_what_the_rotator_does")))
            .child(
                div()
                    .text_size(px(13.))
                    .text_color(pixel::muted_text_color())
                    .child(tr("rotation_sets_both_directions")),
            )
            .child(inspector_field(
                tr("rotation").to_uppercase(),
                tr("clockwise_and_counter_clockwise"),
                controls::cycle_control(
                    rotation_label,
                    ("rotator-rotation", 0usize).into(),
                    ("rotator-rotation", 1usize).into(),
                    cx.listener(|this, _, _, cx| this.cycle_rotation(-1, cx)),
                    cx.listener(|this, _, _, cx| this.cycle_rotation(1, cx)),
                ),
            ))
            .child(controls::status_rail(
                "ROT // A+B",
                rotation
                    .map(|preset| preset.detail())
                    .unwrap_or(tr("rot_custom_detail")),
                if rotation.is_some() {
                    BadgeTone::Accent
                } else {
                    BadgeTone::Neutral
                },
            ))
            .child(pixel::divider())
            .child(inspector_field(
                tr("press").to_uppercase(),
                tr("push_the_rotator_down"),
                controls::cycle_control(
                    press_label,
                    ("rotator-press", 0usize).into(),
                    ("rotator-press", 1usize).into(),
                    cx.listener(|this, _, _, cx| this.cycle_rotator_press(-1, cx)),
                    cx.listener(|this, _, _, cx| this.cycle_rotator_press(1, cx)),
                ),
            ))
            .child(controls::status_rail(
                "PRESS // C",
                press
                    .map(|preset| preset.detail())
                    .unwrap_or(tr("press_custom_detail")),
                if press.is_some() {
                    BadgeTone::Accent
                } else {
                    BadgeTone::Neutral
                },
            ))
    }

    fn render_modifier_row(
        &self,
        mods: u8,
        id: &'static str,
        target: ModifierTarget,
        cx: &mut Context<Self>,
    ) -> Div {
        let mut row = div().flex().flex_wrap().gap(px(6.));
        for (index, (label, bit)) in [
            ("CTRL", 0x01u8),
            ("SHIFT", 0x02),
            ("ALT", 0x04),
            ("CMD", 0x08),
        ]
        .into_iter()
        .enumerate()
        {
            row = row.child(
                controls::modifier_chip(label, mods & bit != 0)
                    .id((id, index))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| match target {
                        ModifierTarget::Simple => this.toggle_simple_modifier(bit, cx),
                        ModifierTarget::Arrow => this.toggle_arrow_modifier(bit, cx),
                        ModifierTarget::Emitted => this.toggle_emitted_modifier(bit, cx),
                    })),
            );
        }
        row
    }

    fn render_simple_editor(&self, slot: usize, cx: &mut Context<Self>) -> Div {
        let input = &self.host.active_profile().inputs[slot];
        let behavior_label = Self::simple_behavior_label(input, slot);
        let icon_name = configured_icon_label(&input.icon);
        let icon_preview = configured_icon_visual(&input.icon, 17., pixel::accent_color())
            .unwrap_or_else(|| lucide_icon_visual("image-off", 17., pixel::muted_text_color()));
        let mut editor = div()
            .w_full()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(Input::new(&self.label_input).w_full())
            .child(
                selection_card(icon_preview, icon_name, None)
                    .id("simple-change-icon")
                    .on_click(cx.listener(|this, _, window, cx| {
                        let selected_icon = this
                            .host
                            .selected_slot
                            .and_then(|slot| this.host.active_profile().inputs.get(slot))
                            .map(|input| input.icon.clone())
                            .unwrap_or_default();
                        this.icon_library =
                            if crate::simple_icons::slug_from_storage(&selected_icon).is_some() {
                                IconLibrary::Simple
                            } else {
                                IconLibrary::Lucide
                            };
                        this.icon_query.clear();
                        this.icon_page = icon_picker_page(&selected_icon);
                        this.icon_scroll.set_offset(point(px(0.), px(0.)));
                        this.sheet = Sheet::Icons;
                        this.sync_inputs(window, cx);
                        let placeholder = match this.icon_library {
                            IconLibrary::Lucide => tr("icon_search_placeholder"),
                            IconLibrary::Simple => tr("brand_search_placeholder"),
                        };
                        this.search_input.update(cx, |input, cx| {
                            input.set_placeholder(placeholder, window, cx)
                        });
                        cx.notify();
                    })),
            )
            .child(controls::cycle_control(
                behavior_label,
                ("simple-behavior", 0usize).into(),
                ("simple-behavior", 1usize).into(),
                cx.listener(|this, _, _, cx| this.cycle_simple_behavior(-1, cx)),
                cx.listener(|this, _, _, cx| this.cycle_simple_behavior(1, cx)),
            ));

        match &input.behavior {
            Some(ControlBehavior::ApplicationShortcut {
                application,
                shortcut,
            }) => {
                // A stale/foreign id renders as unknown with the slot's real
                // chord rather than masquerading as the catalog's first entry.
                let field = match behaviors::shortcut_preset(application, shortcut) {
                    Some(preset) => {
                        let app = behaviors::shortcut_application(application)
                            .unwrap_or(&behaviors::APPLICATION_SHORTCUTS[0]);
                        inspector_field(
                            tr("shortcut"),
                            behaviors::shortcut_chord_label(preset),
                            selection_card(
                                shortcut_app_icon(app, 17., pixel::accent_color()),
                                preset.label(),
                                Some(app.label.into()),
                            )
                            .id("open-shortcut-picker")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_shortcut_picker(window, cx)
                            })),
                        )
                    }
                    None => inspector_field(
                        tr("shortcut"),
                        keycodes::emitted_key_label(input.emitted.mods, input.emitted.code),
                        selection_card(
                            lucide_icon_visual("circle-help", 17., pixel::dim_text_color()),
                            tr("unknown_shortcut"),
                            Some(SharedString::from(application.clone())),
                        )
                        .id("open-shortcut-picker")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.open_shortcut_picker(window, cx)
                        })),
                    ),
                };
                editor = editor.child(field);
            }
            Some(ControlBehavior::MacOs { command }) => {
                let preset = behaviors::macos_preset(*command);
                editor = editor.child(inspector_field(
                    tr("control"),
                    preset.detail,
                    controls::cycle_control(
                        preset.label,
                        ("macos-preset", 0usize).into(),
                        ("macos-preset", 1usize).into(),
                        cx.listener(|this, _, _, cx| this.cycle_macos(-1, cx)),
                        cx.listener(|this, _, _, cx| this.cycle_macos(1, cx)),
                    ),
                ));
            }
            Some(ControlBehavior::Keystroke) => {
                editor = editor
                    .child(inspector_field(
                        tr("modifiers"),
                        "Optional chord modifiers",
                        self.render_modifier_row(
                            input.emitted.mods,
                            "simple-modifier",
                            ModifierTarget::Simple,
                            cx,
                        ),
                    ))
                    .child(inspector_field(
                        tr("key"),
                        keycodes::mods_label(input.emitted.mods),
                        tiny_button(keycodes::emitted_key_label(
                            input.emitted.mods,
                            input.emitted.code,
                        ))
                        .w_full()
                        .id("open-key-picker-simple")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.open_key_picker(KeyTarget::SimpleKey, cx)
                        })),
                    ));
            }
            Some(ControlBehavior::App { target }) => {
                let app_name = if target.is_empty() {
                    tr("choose_application").to_string()
                } else {
                    PathBuf::from(target)
                        .file_stem()
                        .and_then(|name| name.to_str())
                        .unwrap_or(target)
                        .to_string()
                };
                let app_row = div().w_full().flex().items_center().gap(px(8.)).child(
                    div().flex_1().min_w(px(0.)).child(
                        selection_card(
                            lucide_icon_visual("app-window-mac", 17., pixel::accent_color()),
                            app_name,
                            if target.is_empty() {
                                None
                            } else {
                                Some(short_text(target, 44).into())
                            },
                        )
                        .id("open-app-picker")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.icon_query.clear();
                            this.icon_page = 0;
                            this.sheet = Sheet::Applications;
                            this.sync_inputs(window, cx);
                            this.search_input.update(cx, |input, cx| {
                                input.set_placeholder(tr("app_search_placeholder"), window, cx)
                            });
                            cx.notify();
                        })),
                    ),
                );
                let app_row = if target.is_empty() {
                    app_row
                } else {
                    app_row.child(
                        chrome_icon_button("external-link")
                            .id("test-selected-app")
                            .on_click(cx.listener(|this, _, _, _| {
                                if let Some(slot) = this.host.selected_slot {
                                    actions::execute(
                                        &this.host.active_profile().inputs[slot].action,
                                    );
                                }
                            })),
                    )
                };
                editor = editor.child(app_row);
            }
            None => {}
        }

        editor
            .child(
                controls::toggle_face(tr("advanced"), self.advanced, true)
                    .id("toggle-advanced")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.advanced = !this.advanced;
                        cx.notify();
                    })),
            )
            .when(self.advanced, |editor| {
                editor.child(self.render_advanced_editor(slot, cx))
            })
    }

    fn render_advanced_editor(&self, slot: usize, cx: &mut Context<Self>) -> Div {
        let input = &self.host.active_profile().inputs[slot];
        let kind_label = match input.emitted.kind {
            SlotKind::None => tr("kind_nothing"),
            SlotKind::Keyboard => tr("kind_keycode"),
            SlotKind::Consumer => tr("kind_media"),
        };
        let usage_label = match input.emitted.kind {
            SlotKind::None => tr("kind_nothing").to_string(),
            SlotKind::Keyboard => keycodes::keyboard_name(input.emitted.code)
                .unwrap_or("Unknown key")
                .to_string(),
            SlotKind::Consumer => keycodes::consumer_name(input.emitted.code)
                .unwrap_or("Unknown media code")
                .to_string(),
        };
        let group = slots_for_cell(cell_for_slot(slot));

        let mut editor = div()
            .w_full()
            .flex()
            .flex_col()
            .gap(px(12.))
            .pt(px(4.))
            .child(pixel::section_header(3, tr("device_output")))
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(pixel::muted_text_color())
                    .child(tr("stored_on_the_pad_and_works")),
            );

        if group.len() > 1 {
            editor = editor.child(inspector_field(
                tr("direction_gesture"),
                "Choose the sub-control represented by this hardware cell",
                controls::cycle_control(
                    slot_name(slot),
                    ("advanced-subslot", 0usize).into(),
                    ("advanced-subslot", 1usize).into(),
                    cx.listener(|this, _, window, cx| this.cycle_subslot(-1, window, cx)),
                    cx.listener(|this, _, window, cx| this.cycle_subslot(1, window, cx)),
                ),
            ));
        }

        editor = editor
            .child(inspector_field(
                "OUTPUT TYPE",
                keycodes::slot_label(&input.emitted),
                controls::cycle_control(
                    kind_label,
                    ("emitted-kind", 0usize).into(),
                    ("emitted-kind", 1usize).into(),
                    cx.listener(|this, _, _, cx| this.cycle_emitted_kind(-1, cx)),
                    cx.listener(|this, _, _, cx| this.cycle_emitted_kind(1, cx)),
                ),
            ))
            .when(input.emitted.kind != SlotKind::None, |editor| {
                editor.child(inspector_field(
                    if input.emitted.kind == SlotKind::Keyboard {
                        tr("keycode_2")
                    } else {
                        tr("media_code_2")
                    },
                    format!("USB HID usage 0x{:04X}", input.emitted.code),
                    if input.emitted.kind == SlotKind::Keyboard {
                        tiny_button(keycodes::emitted_key_label(
                            input.emitted.mods,
                            input.emitted.code,
                        ))
                        .w_full()
                        .id("open-key-picker-emitted")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.open_key_picker(KeyTarget::EmittedCode, cx)
                        }))
                        .into_any_element()
                    } else {
                        controls::cycle_control(
                            usage_label,
                            ("emitted-code", 0usize).into(),
                            ("emitted-code", 1usize).into(),
                            cx.listener(|this, _, _, cx| this.cycle_emitted_code(-1, cx)),
                            cx.listener(|this, _, _, cx| this.cycle_emitted_code(1, cx)),
                        )
                        .into_any_element()
                    },
                ))
            })
            .when(input.emitted.kind == SlotKind::Keyboard, |editor| {
                editor.child(inspector_field(
                    tr("modifiers_2"),
                    keycodes::mods_label(input.emitted.mods),
                    self.render_modifier_row(
                        input.emitted.mods,
                        "emitted-modifier",
                        ModifierTarget::Emitted,
                        cx,
                    ),
                ))
            })
            .child(pixel::divider())
            .child(pixel::section_header(4, tr("desktop_action")))
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(pixel::muted_text_color())
                    .child(tr("optional_automation_run_by_the")),
            )
            .child(inspector_field(
                tr("when_pressed"),
                actions::describe(&input.action),
                controls::cycle_control(
                    action_kind_label(&input.action),
                    ("action-kind", 0usize).into(),
                    ("action-kind", 1usize).into(),
                    cx.listener(|this, _, window, cx| this.cycle_action_kind(-1, window, cx)),
                    cx.listener(|this, _, window, cx| this.cycle_action_kind(1, window, cx)),
                ),
            ));

        match &input.action {
            Action::None => {
                editor = editor.child(controls::status_rail(
                    "PASS THROUGH",
                    "The operating system receives the device output directly.",
                    BadgeTone::Neutral,
                ));
            }
            Action::Keystroke { mods, key } => {
                let chord = format!(
                    "{}{}",
                    if *mods == 0 {
                        String::new()
                    } else {
                        format!("{} + ", keycodes::mods_label(*mods))
                    },
                    keycodes::keyboard_name(*key).unwrap_or("Unknown key")
                );
                editor = editor.child(
                    div()
                        .flex()
                        .gap(px(8.))
                        .child(controls::field_readout("HOST CHORD", chord))
                        .child(
                            tiny_button(if self.recording == RecordTarget::Action {
                                tr("press_keys")
                            } else {
                                tr("record_shortcut")
                            })
                            .id("record-host-keystroke")
                            .on_click(cx.listener(
                                |this, _, _, cx| {
                                    this.recording = RecordTarget::Action;
                                    cx.notify();
                                },
                            )),
                        )
                        .child(tiny_button(tr("test")).id("test-host-keystroke").on_click(
                            cx.listener(|this, _, _, _| {
                                if let Some(slot) = this.host.selected_slot {
                                    actions::execute(
                                        &this.host.active_profile().inputs[slot].action,
                                    );
                                }
                            }),
                        )),
                );
            }
            Action::Macro { steps } => {
                let count = steps.len();
                editor = editor.child(
                    div()
                        .flex()
                        .gap(px(8.))
                        .child(controls::field_readout(
                            "SEQUENCE",
                            if count == 1 {
                                tr("one_step").to_string()
                            } else {
                                tr("n_steps").replace("{n}", &count.to_string())
                            },
                        ))
                        .child(tiny_button(tr("edit_steps")).id("edit-macro").on_click(
                            cx.listener(|this, _, window, cx| {
                                if let Some(slot) = this.host.selected_slot {
                                    if let Action::Macro { steps } =
                                        &this.host.active_profile().inputs[slot].action
                                    {
                                        this.macro_draft = steps.clone();
                                    }
                                }
                                this.macro_edit_index = (!this.macro_draft.is_empty()).then_some(0);
                                this.sync_macro_input(window, cx);
                                this.sheet = Sheet::Macro;
                                cx.notify();
                            }),
                        ))
                        .child(
                            tiny_button(tr("test_run"))
                                .id("test-macro")
                                .on_click(cx.listener(|this, _, _, _| {
                                    if let Some(slot) = this.host.selected_slot {
                                        actions::execute(
                                            &this.host.active_profile().inputs[slot].action,
                                        );
                                    }
                                })),
                        ),
                );
            }
            Action::Run { .. } => {
                editor = editor.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(8.))
                        .child(Input::new(&self.command_input).w_full())
                        .child(tiny_button(tr("test")).id("test-run-command").on_click(
                            cx.listener(|this, _, _, _| {
                                if let Some(slot) = this.host.selected_slot {
                                    actions::execute(
                                        &this.host.active_profile().inputs[slot].action,
                                    );
                                }
                            }),
                        )),
                );
            }
            Action::Open { .. } => {
                editor = editor.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(8.))
                        .child(Input::new(&self.target_input).w_full())
                        .child(
                            div()
                                .flex()
                                .gap(px(8.))
                                .child(tiny_button(tr("browse")).id("browse-open-target").on_click(
                                    cx.listener(|_, _, window, cx| {
                                        Self::pick_file(window, cx, |this, path, window, cx| {
                                            if let Some(slot) = this.host.selected_slot {
                                                if let Action::Open { target } =
                                                    &mut this.host.active_profile_mut().inputs[slot]
                                                        .action
                                                {
                                                    *target = path.display().to_string();
                                                }
                                                this.commit(false, cx);
                                                this.sync_inputs(window, cx);
                                            }
                                        });
                                    }),
                                ))
                                .child(tiny_button(tr("test")).id("test-open-target").on_click(
                                    cx.listener(|this, _, _, _| {
                                        if let Some(slot) = this.host.selected_slot {
                                            actions::execute(
                                                &this.host.active_profile().inputs[slot].action,
                                            );
                                        }
                                    }),
                                )),
                        ),
                );
            }
            Action::Media { op } => {
                editor = editor.child(
                    div()
                        .flex()
                        .gap(px(8.))
                        .child(controls::cycle_control(
                            media_label(*op),
                            ("action-media", 0usize).into(),
                            ("action-media", 1usize).into(),
                            cx.listener(|this, _, _, cx| this.cycle_media_action(-1, cx)),
                            cx.listener(|this, _, _, cx| this.cycle_media_action(1, cx)),
                        ))
                        .child(tiny_button(tr("test")).id("test-media-action").on_click(
                            cx.listener(|this, _, _, _| {
                                if let Some(slot) = this.host.selected_slot {
                                    actions::execute(
                                        &this.host.active_profile().inputs[slot].action,
                                    );
                                }
                            }),
                        )),
                );
            }
            Action::AppSettings => {
                editor = editor.child(controls::status_rail(
                    "OPENMICRO",
                    "Pressing this control opens the Settings sheet.",
                    BadgeTone::Info,
                ));
            }
        }

        if (SLOT_JOY_UP..=SLOT_JOY_PRESS).contains(&slot) {
            editor = editor.child(pixel::divider()).child(inspector_field(
                tr("deflection").to_uppercase(),
                tr("lower_values_respond_sooner"),
                controls::cycle_control(
                    self.host.active_profile().analog.joy_threshold.to_string(),
                    ("joy-threshold", 0usize).into(),
                    ("joy-threshold", 1usize).into(),
                    cx.listener(|this, _, _, cx| this.adjust_threshold(-50, cx)),
                    cx.listener(|this, _, _, cx| this.adjust_threshold(50, cx)),
                ),
            ));
        }

        editor
    }

    fn render_joystick_editor(&self, slot: usize, cx: &mut Context<Self>) -> Div {
        let mode = JoystickMode::infer(self.host.active_profile());
        let arrow_mods = JoystickMode::arrow_mods(self.host.active_profile()).unwrap_or(0);
        let profile = self.host.active_profile();
        let mut editor = div()
            .w_full()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(pixel::section_header(
                1,
                tr("choose_what_the_joystick_does"),
            ))
            .child(
                div()
                    .text_size(px(13.))
                    .text_color(pixel::muted_text_color())
                    .child(tr("one_mode_covers_all_four_directions")),
            )
            .child(inspector_field(
                tr("mode").to_uppercase(),
                tr("mouse_pointer_arrow_keys_or"),
                controls::cycle_control(
                    mode.label(),
                    ("joystick-mode", 0usize).into(),
                    ("joystick-mode", 1usize).into(),
                    cx.listener(|this, _, _, cx| this.cycle_joystick_mode(-1, cx)),
                    cx.listener(|this, _, _, cx| this.cycle_joystick_mode(1, cx)),
                ),
            ));

        match mode {
            JoystickMode::Mouse => {
                editor = editor
                    .child(controls::status_rail(
                        "ANALOG // MOUSE",
                        tr("deflection_moves_the_pointer"),
                        BadgeTone::Info,
                    ))
                    .child(inspector_field(
                        tr("pointer_speed").to_uppercase(),
                        "1 = precise // 10 = fast",
                        controls::cycle_control(
                            profile.analog.joy_mouse_speed.to_string(),
                            ("joystick-speed", 0usize).into(),
                            ("joystick-speed", 1usize).into(),
                            cx.listener(|this, _, _, cx| this.adjust_joystick_speed(-1, cx)),
                            cx.listener(|this, _, _, cx| this.adjust_joystick_speed(1, cx)),
                        ),
                    ));
            }
            JoystickMode::Grade => {
                editor = editor
                    .child(controls::status_rail(
                        "ANALOG // GRADE",
                        tr("joy_grade_detail"),
                        BadgeTone::Info,
                    ))
                    .child(inspector_field(
                        tr("drag_speed").to_uppercase(),
                        "1 = precise // 10 = fast",
                        controls::cycle_control(
                            profile.analog.joy_mouse_speed.to_string(),
                            ("joystick-speed", 0usize).into(),
                            ("joystick-speed", 1usize).into(),
                            cx.listener(|this, _, _, cx| this.adjust_joystick_speed(-1, cx)),
                            cx.listener(|this, _, _, cx| this.adjust_joystick_speed(1, cx)),
                        ),
                    ))
                    .child(
                        div()
                            .text_size(px(13.))
                            .text_color(pixel::muted_text_color())
                            .child(tr("joy_grade_howto")),
                    );
            }
            JoystickMode::Arrows => {
                editor = editor
                    .child(controls::status_rail(
                        "DIGITAL // ARROWS",
                        tr("joy_arrows_detail"),
                        BadgeTone::Success,
                    ))
                    .child(inspector_field(
                        tr("held_modifiers").to_uppercase(),
                        tr("sent_with_every_arrow_press"),
                        self.render_modifier_row(
                            arrow_mods,
                            "arrow-modifier",
                            ModifierTarget::Arrow,
                            cx,
                        ),
                    ));
            }
            JoystickMode::Custom => {
                editor = editor.child(controls::status_rail(
                    "DIGITAL // CUSTOM",
                    tr("joy_custom_detail"),
                    BadgeTone::Accent,
                ));
            }
        }

        editor = editor.child(inspector_field(
            tr("deflection").to_uppercase(),
            tr("lower_values_respond_sooner"),
            controls::cycle_control(
                profile.analog.joy_threshold.to_string(),
                ("joystick-threshold", 0usize).into(),
                ("joystick-threshold", 1usize).into(),
                cx.listener(|this, _, _, cx| this.adjust_threshold(-50, cx)),
                cx.listener(|this, _, _, cx| this.adjust_threshold(50, cx)),
            ),
        ));

        if !matches!(mode, JoystickMode::Mouse | JoystickMode::Grade) {
            editor = editor.child(pixel::divider()).child(
                controls::toggle_face(
                    if mode == JoystickMode::Custom {
                        "CUSTOM SLOT EDITOR"
                    } else {
                        tr("advanced")
                    },
                    self.advanced || mode == JoystickMode::Custom,
                    true,
                )
                .id("toggle-joystick-advanced")
                .on_click(cx.listener(|this, _, _, cx| {
                    this.advanced = !this.advanced;
                    cx.notify();
                })),
            );
            if self.advanced || mode == JoystickMode::Custom {
                editor = editor.child(self.render_advanced_editor(slot, cx));
            }
        }
        editor
    }

    fn render_inspector(&self, cx: &mut Context<Self>) -> Div {
        let Some(slot) = self.host.selected_slot else {
            return div()
                .w(px(360.))
                .min_w(px(340.))
                .child(controls::empty_hint(
                    tr("choose_a_control"),
                    tr("select_a_key_dial_joystick_direction"),
                ));
        };
        let cell = cell_for_slot(slot);
        let body = if cell == CELL_ENCODER {
            self.render_rotator_editor(cx)
        } else if cell == CELL_JOYSTICK {
            self.render_joystick_editor(slot, cx)
        } else if slot < KEY_SLOTS || slot == SLOT_TOUCH_TAP {
            self.render_simple_editor(slot, cx)
        } else {
            self.render_advanced_editor(slot, cx)
        };

        div()
            .w(px(360.))
            .min_w(px(340.))
            .max_h(relative(1.))
            .overflow_hidden()
            .child(
                div()
                    .w_full()
                    .py(px(8.))
                    .max_h(relative(1.))
                    .overflow_y_scrollbar()
                    .child(body),
            )
    }

    fn render_settings_sheet(&self, cx: &mut Context<Self>) -> Div {
        let brightness = ((self.host.config.led_brightness as f32 / 255.0) * 100.0).round() as u32;
        controls::modal_frame()
            .max_h(relative(0.9))
            .child(controls::modal_header(
                tr("settings_2"),
                Some(tr("app_behavior_profile_data_and").into()),
            ))
            .child(
                div()
                    .w_full()
                    .flex_1()
                    .min_h(px(0.))
                    .p(px(16.))
                    .overflow_y_scrollbar()
                    .flex()
                    .flex_col()
                    .gap(px(14.))
                    .child(inspector_field(
                        tr("profile"),
                        "Rename the active profile or remove it",
                        Input::new(&self.profile_input).w_full(),
                    ))
                    .child(
                        tiny_button(if self.confirm_delete {
                            "CONFIRM DELETE PROFILE"
                        } else {
                            "DELETE ACTIVE PROFILE"
                        })
                        .id("delete-active-profile")
                        .on_click(
                            cx.listener(|this, _, window, cx| {
                                this.delete_active_profile(window, cx)
                            }),
                        ),
                    )
                    .child(pixel::divider())
                    .child(
                        controls::toggle_face(
                            tr("launch_at_login"),
                            self.host.config.launch_at_login,
                            true,
                        )
                        .id("toggle-launch-login")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.host.config.launch_at_login = !this.host.config.launch_at_login;
                            if let Err(error) =
                                apply_launch_at_login(this.host.config.launch_at_login)
                            {
                                this.push_log(format!("launch at login: {error}"));
                            }
                            this.commit(false, cx);
                        })),
                    )
                    .child(
                        controls::toggle_face(
                            tr("show_menubar_icon"),
                            self.host.config.show_menubar,
                            true,
                        )
                        .id("toggle-menubar")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.host.config.show_menubar = !this.host.config.show_menubar;
                            if let Some(menubar) = &mut this.host.menubar {
                                menubar.set_visible(this.host.config.show_menubar);
                            }
                            this.commit(false, cx);
                        })),
                    )
                    .child(inspector_field(
                        tr("language_eyebrow"),
                        tr("language_applies"),
                        controls::cycle_control(
                            self.language_label(),
                            ("settings-language", 0usize).into(),
                            ("settings-language", 1usize).into(),
                            cx.listener(|this, _, window, cx| this.cycle_language(-1, window, cx)),
                            cx.listener(|this, _, window, cx| this.cycle_language(1, window, cx)),
                        ),
                    ))
                    .child(inspector_field(
                        tr("appearance_eyebrow"),
                        tr("theme_applies"),
                        controls::cycle_control(
                            self.theme_label(),
                            ("settings-theme", 0usize).into(),
                            ("settings-theme", 1usize).into(),
                            cx.listener(|this, _, window, cx| this.cycle_theme(-1, window, cx)),
                            cx.listener(|this, _, window, cx| this.cycle_theme(1, window, cx)),
                        ),
                    ))
                    .child(pixel::divider())
                    .child(inspector_field(
                        tr("backlight_brightness").to_uppercase(),
                        tr("dims_the_per_key_backlight_and"),
                        controls::cycle_control(
                            format!("{brightness}%"),
                            ("settings-brightness", 0usize).into(),
                            ("settings-brightness", 1usize).into(),
                            cx.listener(|this, _, _, cx| this.adjust_brightness(-13, cx)),
                            cx.listener(|this, _, _, cx| this.adjust_brightness(13, cx)),
                        ),
                    ))
                    .child(inspector_field(
                        tr("backlight_pattern").to_uppercase(),
                        tr("pattern_key_note"),
                        controls::cycle_control(
                            Self::pattern_label(self.host.config.led_key_pattern),
                            ("settings-key-pattern", 0usize).into(),
                            ("settings-key-pattern", 1usize).into(),
                            cx.listener(|this, _, _, cx| this.cycle_pattern(true, -1, cx)),
                            cx.listener(|this, _, _, cx| this.cycle_pattern(true, 1, cx)),
                        ),
                    ))
                    .child(inspector_field(
                        tr("ambient_pattern").to_uppercase(),
                        tr("pattern_ambient_note"),
                        controls::cycle_control(
                            Self::pattern_label(self.host.config.led_ambient_pattern),
                            ("settings-ambient-pattern", 0usize).into(),
                            ("settings-ambient-pattern", 1usize).into(),
                            cx.listener(|this, _, _, cx| this.cycle_pattern(false, -1, cx)),
                            cx.listener(|this, _, _, cx| this.cycle_pattern(false, 1, cx)),
                        ),
                    ))
                    .child(pixel::divider())
                    .child(
                        div()
                            .font_family("Monaco")
                            .text_size(px(11.))
                            .font_semibold()
                            .text_color(pixel::accent_highlight_color())
                            .child(tr("agent_status_colors")),
                    )
                    .child(inspector_field(
                        tr("agent_working_color"),
                        tr("agent_working_color_note"),
                        controls::cycle_control(
                            Self::status_color_value(
                                self.host.config.activity_status_colors.working,
                            ),
                            ("settings-agent-working", 0usize).into(),
                            ("settings-agent-working", 1usize).into(),
                            cx.listener(|this, _, _, cx| {
                                this.cycle_status_color(ActivityStatus::Working, -1, cx)
                            }),
                            cx.listener(|this, _, _, cx| {
                                this.cycle_status_color(ActivityStatus::Working, 1, cx)
                            }),
                        ),
                    ))
                    .child(inspector_field(
                        tr("agent_attention_color"),
                        tr("agent_attention_color_note"),
                        controls::cycle_control(
                            Self::status_color_value(
                                self.host.config.activity_status_colors.attention,
                            ),
                            ("settings-agent-attention", 0usize).into(),
                            ("settings-agent-attention", 1usize).into(),
                            cx.listener(|this, _, _, cx| {
                                this.cycle_status_color(ActivityStatus::Attention, -1, cx)
                            }),
                            cx.listener(|this, _, _, cx| {
                                this.cycle_status_color(ActivityStatus::Attention, 1, cx)
                            }),
                        ),
                    ))
                    .child(inspector_field(
                        tr("agent_success_color"),
                        tr("agent_success_color_note"),
                        controls::cycle_control(
                            Self::status_color_value(
                                self.host.config.activity_status_colors.success,
                            ),
                            ("settings-agent-success", 0usize).into(),
                            ("settings-agent-success", 1usize).into(),
                            cx.listener(|this, _, _, cx| {
                                this.cycle_status_color(ActivityStatus::Success, -1, cx)
                            }),
                            cx.listener(|this, _, _, cx| {
                                this.cycle_status_color(ActivityStatus::Success, 1, cx)
                            }),
                        ),
                    ))
                    .child(inspector_field(
                        tr("agent_error_color"),
                        tr("agent_error_color_note"),
                        controls::cycle_control(
                            Self::status_color_value(self.host.config.activity_status_colors.error),
                            ("settings-agent-error", 0usize).into(),
                            ("settings-agent-error", 1usize).into(),
                            cx.listener(|this, _, _, cx| {
                                this.cycle_status_color(ActivityStatus::Error, -1, cx)
                            }),
                            cx.listener(|this, _, _, cx| {
                                this.cycle_status_color(ActivityStatus::Error, 1, cx)
                            }),
                        ),
                    ))
                    .child(pixel::divider())
                    .child(self.render_agent_integrations(cx))
                    .child(pixel::divider())
                    .child(
                        tiny_button(tr("firmware"))
                            .id("settings-firmware")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.sheet = Sheet::Firmware;
                                cx.notify();
                            })),
                    )
                    .child(pixel::divider())
                    .child(
                        div()
                            .font_family("Monaco")
                            .text_size(px(11.))
                            .font_semibold()
                            .text_color(pixel::accent_highlight_color())
                            .child(tr("profile_data")),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(pixel::muted_text_color())
                            .child(tr("your_human_readable_json_config")),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap(px(8.))
                            .child(
                                tiny_button(tr("export"))
                                    .id("export-config")
                                    .on_click(cx.listener(|this, _, _, cx| this.export_config(cx))),
                            )
                            .child(
                                tiny_button(tr("import_replace"))
                                    .id("import-replace")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.import_config(config::ImportMode::Replace, window, cx)
                                    })),
                            )
                            .child(tiny_button(tr("import_merge")).id("import-merge").on_click(
                                cx.listener(|this, _, window, cx| {
                                    this.import_config(config::ImportMode::Merge, window, cx)
                                }),
                            )),
                    )
                    .child(controls::status_rail(
                        tr("accessibility"),
                        if actions::accessibility_trusted() {
                            tr("perm_granted")
                        } else {
                            tr("perm_missing")
                        },
                        if actions::accessibility_trusted() {
                            BadgeTone::Success
                        } else {
                            BadgeTone::Danger
                        },
                    ))
                    .child(
                        tiny_button(tr("open_system_settings"))
                            .id("settings-permission")
                            .on_click(|_, _, _| actions::open_permission_settings()),
                    )
                    .child(pixel::divider())
                    .child(
                        tiny_button(if self.confirm_reset {
                            tr("reset_confirm")
                        } else {
                            tr("reset_factory")
                        })
                        .id("factory-reset")
                        .on_click(
                            cx.listener(|this, _, window, cx| this.reset_factory(window, cx)),
                        ),
                    ),
            )
            .child(
                div().w_full().p(px(14.)).flex().justify_end().child(
                    tiny_button(tr("done"))
                        .id("close-settings")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.confirm_delete = false;
                            this.confirm_reset = false;
                            this.sheet = Sheet::None;
                            cx.notify();
                        })),
                ),
            )
    }

    fn render_applications_sheet(&self, cx: &mut Context<Self>) -> Div {
        let query = self.icon_query.trim().to_lowercase();
        let matches: Vec<(usize, &InstalledApp)> = self
            .installed_apps
            .iter()
            .enumerate()
            .filter(|(_, app)| {
                query.is_empty()
                    || app.name.to_lowercase().contains(&query)
                    || app.path.to_lowercase().contains(&query)
            })
            .collect();
        let mut list = div().w_full().flex().flex_col().gap(px(6.));
        for (index, app) in matches.iter().take(80) {
            let app_index = *index;
            list = list.child(
                div()
                    .id(("installed-app", app_index))
                    .w_full()
                    .min_h(px(48.))
                    .px(px(12.))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .bg(pixel::raised_color())
                    .cursor_pointer()
                    .hover(|style| style.bg(pixel::canvas_color()))
                    .child(
                        div()
                            .w(px(26.))
                            .h(px(26.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(pixel::canvas_color())
                            .font_family("lucide")
                            .text_size(px(16.))
                            .text_color(pixel::accent_color())
                            .child(icon_glyph("app-window-mac")),
                    )
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .child(
                                div()
                                    .text_size(px(14.))
                                    .text_color(pixel::text_color())
                                    .child(app.name.clone()),
                            )
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(pixel::dim_text_color())
                                    .child(short_text(&app.path, 70)),
                            ),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let Some(slot) = this.host.selected_slot else {
                            return;
                        };
                        let Some(app) = this.installed_apps.get(app_index) else {
                            return;
                        };
                        let target = app.path.clone();
                        behaviors::apply_app(
                            &mut this.host.active_profile_mut().inputs[slot],
                            slot,
                            target,
                        );
                        this.commit(true, cx);
                        this.sheet = Sheet::None;
                        cx.notify();
                    })),
            );
        }
        if matches.is_empty() {
            list = list.child(controls::empty_hint(
                "NO APPLICATIONS",
                "Try another search or refresh the application scan.",
            ));
        }

        controls::modal_frame()
            .w(px(680.))
            .max_h(relative(0.9))
            .child(controls::modal_header(
                tr("choose_an_application_2"),
                Some(tr("app_picker_meta").into()),
            ))
            .child(
                div()
                    .p(px(14.))
                    .flex()
                    .gap(px(8.))
                    .child(Input::new(&self.search_input).w_full().cleanable(true))
                    .child(
                        tiny_button(tr("refresh"))
                            .id("refresh-apps")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.installed_apps = behaviors::installed_apps();
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .w_full()
                    .flex_1()
                    .min_h(px(0.))
                    .px(px(14.))
                    .pb(px(14.))
                    .overflow_y_scrollbar()
                    .child(list),
            )
            .child(
                div()
                    .p(px(14.))
                    .flex()
                    .justify_between()
                    .child(tiny_button(tr("clear")).id("clear-app-selection").on_click(
                        cx.listener(|this, _, _, cx| {
                            let Some(slot) = this.host.selected_slot else {
                                return;
                            };
                            behaviors::apply_app(
                                &mut this.host.active_profile_mut().inputs[slot],
                                slot,
                                String::new(),
                            );
                            this.commit(true, cx);
                            this.sheet = Sheet::None;
                        }),
                    ))
                    .child(tiny_button(tr("cancel")).id("cancel-app-picker").on_click(
                        cx.listener(|this, _, _, cx| {
                            this.sheet = Sheet::None;
                            cx.notify();
                        }),
                    )),
            )
    }

    fn shortcut_pick_row(
        &self,
        id: (&'static str, usize),
        app_id: &'static str,
        preset: &'static behaviors::ShortcutPreset,
        subtitle: Option<SharedString>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let preset_id = preset.id;
        let mut title = div()
            .flex_1()
            .min_w(px(0.))
            .flex()
            .flex_col()
            .gap(px(2.))
            .child(
                div()
                    .truncate()
                    .text_size(px(13.))
                    .text_color(pixel::text_color())
                    .child(SharedString::from(preset.label())),
            );
        if let Some(subtitle) = subtitle {
            title = title.child(
                div()
                    .truncate()
                    .text_size(px(10.))
                    .text_color(pixel::dim_text_color())
                    .child(subtitle),
            );
        }
        div()
            .id(id)
            .w_full()
            .min_h(px(42.))
            .px(px(12.))
            .py(px(6.))
            .flex()
            .items_center()
            .gap(px(10.))
            .bg(pixel::raised_color())
            .rounded(px(2.))
            .cursor_pointer()
            .hover(|style| style.bg(pixel::canvas_color()))
            .child(title)
            .child(
                div()
                    .flex_shrink_0()
                    .font_family("Monaco")
                    .text_size(px(10.))
                    .text_color(pixel::accent_highlight_color())
                    .child(behaviors::shortcut_chord_label(preset)),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.apply_shortcut_pick(app_id, preset_id, cx)
            }))
    }

    fn render_shortcut_picker_sheet(&self, cx: &mut Context<Self>) -> Div {
        let query = self.icon_query.trim().to_lowercase();

        let body = if query.is_empty() {
            let selected = behaviors::shortcut_application(&self.shortcut_picker_app)
                .unwrap_or(&behaviors::APPLICATION_SHORTCUTS[0]);
            // Rows are direct children of the tracked element so
            // scroll_to_item can index them when the sheet opens.
            let mut rail = div()
                .id("shortcut-rail-scroll")
                .size_full()
                .track_scroll(&self.shortcut_rail_scroll)
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .gap(px(4.));
            for (index, app) in sorted_shortcut_applications().into_iter().enumerate() {
                let active = app.id == selected.id;
                let app_id = app.id;
                rail = rail.child(
                    div()
                        .id(("shortcut-picker-app", index))
                        .w_full()
                        .min_h(px(34.))
                        .px(px(10.))
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .rounded(px(2.))
                        .bg(if active {
                            pixel::key_color()
                        } else {
                            pixel::raised_color()
                        })
                        .cursor_pointer()
                        .hover(|style| style.bg(pixel::canvas_color()))
                        .child(shortcut_app_icon(
                            app,
                            15.,
                            if active {
                                pixel::accent_color()
                            } else {
                                pixel::dim_text_color()
                            },
                        ))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .truncate()
                                .text_size(px(12.))
                                .text_color(pixel::text_color())
                                .child(SharedString::from(app.label)),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.shortcut_picker_app = app_id.to_string();
                            this.shortcut_list_scroll.set_offset(point(px(0.), px(0.)));
                            cx.notify();
                        })),
                );
            }
            let mut items = div()
                .id("shortcut-list-scroll")
                .size_full()
                .track_scroll(&self.shortcut_list_scroll)
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .gap(px(5.));
            for (index, preset) in selected.shortcuts.iter().enumerate() {
                items = items.child(self.shortcut_pick_row(
                    ("shortcut-pick", index),
                    selected.id,
                    preset,
                    None,
                    cx,
                ));
            }
            div()
                .w_full()
                .h_full()
                .flex()
                .gap(px(10.))
                .child(
                    div()
                        .w(px(220.))
                        .flex_shrink_0()
                        .h_full()
                        .relative()
                        .child(rail)
                        .vertical_scrollbar(&self.shortcut_rail_scroll),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .h_full()
                        .relative()
                        .child(items)
                        .vertical_scrollbar(&self.shortcut_list_scroll),
                )
        } else {
            let mut items = div()
                .id("shortcut-search-scroll")
                .size_full()
                .track_scroll(&self.shortcut_list_scroll)
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .gap(px(5.));
            let mut index = 0usize;
            for app in sorted_shortcut_applications() {
                let app_matches = app.label.to_lowercase().contains(&query);
                for preset in app.shortcuts {
                    // Users search in whichever language they think in, so
                    // match every localization of the label.
                    if app_matches
                        || preset
                            .labels
                            .iter()
                            .any(|label| label.to_lowercase().contains(&query))
                    {
                        items = items.child(self.shortcut_pick_row(
                            ("shortcut-search", index),
                            app.id,
                            preset,
                            Some(SharedString::from(app.label)),
                            cx,
                        ));
                        index += 1;
                    }
                }
            }
            if index == 0 {
                items = items.child(controls::empty_hint(
                    "NO SHORTCUTS",
                    "Try another application or shortcut name.",
                ));
            }
            div()
                .w_full()
                .h_full()
                .relative()
                .child(items)
                .vertical_scrollbar(&self.shortcut_list_scroll)
        };

        controls::modal_frame()
            .w(px(680.))
            .h(relative(0.85))
            .child(controls::modal_header(
                tr("pick_a_shortcut"),
                Some(tr("shortcut_picker_meta").into()),
            ))
            .child(
                div()
                    .p(px(14.))
                    .pb(px(10.))
                    .child(Input::new(&self.search_input).w_full().cleanable(true)),
            )
            .child(
                div()
                    .w_full()
                    .flex_1()
                    .min_h(px(0.))
                    .px(px(14.))
                    .child(body),
            )
            .child(
                div().w_full().p(px(14.)).flex().justify_end().child(
                    tiny_button(tr("cancel"))
                        .id("shortcut-picker-cancel")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.sheet = Sheet::None;
                            cx.notify();
                        })),
                ),
            )
    }

    fn render_icons_sheet(&self, cx: &mut Context<Self>) -> Div {
        let query = self.icon_query.trim().to_lowercase();
        let filtered: Vec<PickerIcon> = match self.icon_library {
            IconLibrary::Lucide => crate::lucide::ICONS
                .iter()
                .map(|(name, _)| *name)
                .filter(|name| query.is_empty() || name.contains(&query))
                .map(PickerIcon::Lucide)
                .collect(),
            IconLibrary::Simple => simple_picker_icons(&query)
                .into_iter()
                .map(PickerIcon::Simple)
                .collect(),
        };
        let pages = filtered.len().div_ceil(ICON_PAGE_SIZE).max(1);
        let page = self.icon_page.min(pages - 1);
        let start = page * ICON_PAGE_SIZE;
        let selected_value = self
            .host
            .selected_slot
            .and_then(|slot| self.host.active_profile().inputs.get(slot))
            .map(|input| input.icon.clone())
            .unwrap_or_default();
        let mut grid = div().w_full().flex().flex_wrap().gap(px(8.));
        for entry in filtered.iter().skip(start).take(ICON_PAGE_SIZE).copied() {
            let storage_value = entry.storage_value();
            let selected = storage_value == selected_value;
            let tooltip_label = SharedString::from(entry.label().to_string());
            grid = grid.child(
                div()
                    .id(entry.element_id())
                    .w(px(116.))
                    .h(px(64.))
                    .p(px(7.))
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap(px(5.))
                    .when(selected, |tile| tile.bg(pixel::accent_soft_color()))
                    .cursor_pointer()
                    .hover(move |style| {
                        style.bg(if selected {
                            pixel::accent_soft_color()
                        } else {
                            pixel::raised_color()
                        })
                    })
                    .child(entry.visual(selected))
                    .child(
                        div()
                            .font_family("Monaco")
                            .text_size(px(9.))
                            .text_color(if selected {
                                pixel::accent_highlight_color()
                            } else {
                                pixel::muted_text_color()
                            })
                            .child(short_text(entry.label(), 18)),
                    )
                    .tooltip(move |_, cx| cx.new(|_| Tooltip::new(tooltip_label.clone())).into())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(slot) = this.host.selected_slot {
                            this.host.active_profile_mut().inputs[slot].icon =
                                storage_value.clone();
                            this.commit(false, cx);
                        }
                        this.sheet = Sheet::None;
                        cx.notify();
                    })),
            );
        }
        if filtered.is_empty() {
            grid = grid.child(match self.icon_library {
                IconLibrary::Lucide => controls::empty_hint(
                    "NO ICONS",
                    "Try a broader English icon name such as mic, git, or arrow.",
                ),
                IconLibrary::Simple => {
                    controls::empty_hint(tr("no_brand_icons"), tr("brand_search_hint"))
                }
            });
        }

        let picker_hint = match self.icon_library {
            IconLibrary::Lucide => tr("icon_search_placeholder"),
            IconLibrary::Simple => tr("brand_search_placeholder"),
        };
        let count_label = match self.icon_library {
            IconLibrary::Lucide => "ICONS",
            IconLibrary::Simple => "BRAND LOGOS",
        };
        let can_go_previous = page > 0;
        let can_go_next = page + 1 < pages;

        controls::modal_frame()
            .w(px(680.))
            .max_h(relative(0.9))
            .child(controls::modal_header(
                tr("choose_icon"),
                Some(picker_hint.into()),
            ))
            .child(
                div()
                    .px(px(14.))
                    .pt(px(10.))
                    .flex()
                    .gap(px(4.))
                    .child(
                        icon_library_tab(
                            tr("lucide_icons"),
                            self.icon_library == IconLibrary::Lucide,
                        )
                        .id("icons-library-lucide")
                        .when(
                            self.icon_library != IconLibrary::Lucide,
                            |tab| {
                                tab.on_click(cx.listener(|this, _, window, cx| {
                                    this.icon_library = IconLibrary::Lucide;
                                    this.icon_page = 0;
                                    this.icon_scroll.set_offset(point(px(0.), px(0.)));
                                    this.search_input.update(cx, |input, cx| {
                                        input.set_placeholder(
                                            tr("icon_search_placeholder"),
                                            window,
                                            cx,
                                        )
                                    });
                                    cx.notify();
                                }))
                            },
                        ),
                    )
                    .child(
                        icon_library_tab(
                            tr("simple_icons"),
                            self.icon_library == IconLibrary::Simple,
                        )
                        .id("icons-library-simple")
                        .when(
                            self.icon_library != IconLibrary::Simple,
                            |tab| {
                                tab.on_click(cx.listener(|this, _, window, cx| {
                                    this.icon_library = IconLibrary::Simple;
                                    this.icon_page = 0;
                                    this.icon_scroll.set_offset(point(px(0.), px(0.)));
                                    this.search_input.update(cx, |input, cx| {
                                        input.set_placeholder(
                                            tr("brand_search_placeholder"),
                                            window,
                                            cx,
                                        )
                                    });
                                    cx.notify();
                                }))
                            },
                        ),
                    ),
            )
            .child(
                div()
                    .px(px(14.))
                    .py(px(10.))
                    .flex()
                    .child(Input::new(&self.search_input).w_full().cleanable(true)),
            )
            .child(
                div()
                    .w_full()
                    .flex_1()
                    .min_h(px(0.))
                    .px(px(14.))
                    .pb(px(14.))
                    .relative()
                    .child(
                        div()
                            .id("icon-results-scroll")
                            .size_full()
                            .track_scroll(&self.icon_scroll)
                            .overflow_y_scroll()
                            .child(grid),
                    )
                    .vertical_scrollbar(&self.icon_scroll),
            )
            .child(
                div()
                    .p(px(14.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        paging_button("<", can_go_previous)
                            .id("icons-previous")
                            .when(can_go_previous, |button| {
                                button.on_click(cx.listener(|this, _, _, cx| {
                                    this.icon_page = this.icon_page.saturating_sub(1);
                                    this.icon_scroll.set_offset(point(px(0.), px(0.)));
                                    cx.notify();
                                }))
                            }),
                    )
                    .child(
                        div()
                            .flex_1()
                            .text_center()
                            .font_family("Monaco")
                            .text_size(px(11.))
                            .text_color(pixel::muted_text_color())
                            .child(format!(
                                "PAGE {} / {} // {} {}",
                                page + 1,
                                pages,
                                filtered.len(),
                                count_label
                            )),
                    )
                    .child(paging_button(">", can_go_next).id("icons-next").when(
                        can_go_next,
                        |button| {
                            button.on_click(cx.listener(move |this, _, _, cx| {
                                this.icon_page = (this.icon_page + 1).min(pages - 1);
                                this.icon_scroll.set_offset(point(px(0.), px(0.)));
                                cx.notify();
                            }))
                        },
                    ))
                    .child(
                        tiny_button(tr("no_icon"))
                            .id("clear-icon")
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Some(slot) = this.host.selected_slot {
                                    this.host.active_profile_mut().inputs[slot].icon.clear();
                                    this.commit(false, cx);
                                }
                                this.sheet = Sheet::None;
                                cx.notify();
                            })),
                    )
                    .child(tiny_button(tr("cancel")).id("cancel-icon-picker").on_click(
                        cx.listener(|this, _, _, cx| {
                            this.sheet = Sheet::None;
                            cx.notify();
                        }),
                    )),
            )
    }

    fn render_firmware_sheet(&self, cx: &mut Context<Self>) -> Div {
        let installed = self
            .host
            .last_conn
            .as_ref()
            .map(|(version, _)| version.clone())
            .unwrap_or_else(|| "—".into());
        let available = self
            .host
            .release
            .as_ref()
            .map(|catalog| catalog.firmware.version.clone())
            .unwrap_or_else(|| "not checked".into());
        let image = self
            .host
            .firmware_image
            .as_ref()
            .and_then(|path| path.file_name())
            .and_then(|name| name.to_str())
            .map(str::to_owned)
            .unwrap_or_else(|| tr("no_image_selected").to_string());
        let progress = if self.host.updating {
            self.host.update_progress as f32
        } else if self.host.firmware_downloading {
            self.host.firmware_download_progress as f32
        } else {
            0.0
        };
        let phase = self
            .host
            .update_phase
            .clone()
            .or_else(|| self.host.update_error.clone())
            .or_else(|| self.host.release_error.clone())
            .unwrap_or_else(|| "Ready for a signed release or recovery image.".into());
        let mut logs = div().w_full().flex().flex_col();
        for (index, line) in self.host.logs.iter().rev().take(6).enumerate() {
            logs = logs.child(controls::log_line(
                format!("LOG {:02}", index + 1),
                line.clone(),
                BadgeTone::Neutral,
            ));
        }
        if self.host.logs.is_empty() {
            logs = logs.child(controls::log_line(
                "LOG 00",
                "No update activity yet.",
                BadgeTone::Neutral,
            ));
        }

        controls::modal_frame()
            .w(px(640.))
            .max_h(relative(0.9))
            .child(controls::modal_header(
                tr("firmware"),
                Some(tr("safely_update_or_recover_your").into()),
            ))
            .child(
                div()
                    .w_full()
                    .flex_1()
                    .min_h(px(0.))
                    .p(px(16.))
                    .overflow_y_scrollbar()
                    .flex()
                    .flex_col()
                    .gap(px(14.))
                    .child(
                        div()
                            .flex()
                            .gap(px(8.))
                            .child(controls::field_readout(tr("installed"), installed))
                            .child(controls::field_readout("AVAILABLE", available)),
                    )
                    .child(controls::status_rail(
                        "SAFE FLASH",
                        tr("profiles_and_key_configs_survive"),
                        BadgeTone::Success,
                    ))
                    .child(pixel::divider())
                    .child(inspector_field(
                        "FIRMWARE IMAGE",
                        tr("bin_hint"),
                        controls::field_readout("IMAGE", image),
                    ))
                    .child(
                        div()
                            .flex()
                            .gap(px(8.))
                            .child(
                                tiny_button(tr("choose_bin"))
                                    .id("choose-firmware-bin")
                                    .on_click(
                                        cx.listener(|this, _, window, cx| {
                                            this.choose_firmware_image(window, cx)
                                        }),
                                    ),
                            )
                            .child(
                                tiny_button(if self.host.updating {
                                    tr("installing")
                                } else if self.host.firmware_downloading {
                                    tr("downloading")
                                } else {
                                    tr("install")
                                })
                                .id("install-firmware")
                                .on_click(cx.listener(|this, _, _, cx| this.install_firmware(cx))),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(10.))
                            .child(div().flex_1().child(pixel::progress_segments(progress, 24)))
                            .child(pixel::badge(
                                format!("{}%", (progress * 100.0).round() as u32),
                                if self.host.update_error.is_some() {
                                    BadgeTone::Danger
                                } else if self.host.updating || self.host.firmware_downloading {
                                    BadgeTone::Accent
                                } else {
                                    BadgeTone::Neutral
                                },
                            )),
                    )
                    .child(controls::status_rail(
                        "UPDATE STATE",
                        phase,
                        if self.host.update_error.is_some() {
                            BadgeTone::Danger
                        } else if self.host.updating {
                            BadgeTone::Accent
                        } else {
                            BadgeTone::Info
                        },
                    ))
                    .child(logs)
                    .child(pixel::divider())
                    .child(
                        div()
                            .font_family("Monaco")
                            .text_size(px(11.))
                            .font_semibold()
                            .text_color(pixel::accent_highlight_color())
                            .child(tr("advanced")),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(pixel::muted_text_color())
                            .child(tr("drops_the_pad_into_its_rom_bootloader")),
                    )
                    .child(tiny_button(tr("reboot_into_dfu")).id("enter-dfu").on_click(
                        cx.listener(|this, _, _, cx| {
                            if let Some(tx) = &this.host.device_tx {
                                let _ = tx.send(DeviceCmd::EnterDfuOnly);
                            }
                            this.push_log("requested ROM DFU mode".into());
                            cx.notify();
                        }),
                    ))
                    .child(controls::status_rail(
                        "POWER WARNING",
                        tr("keep_the_pad_powered_if_the"),
                        BadgeTone::Warning,
                    )),
            )
            .child(
                div().p(px(14.)).flex().justify_end().child(
                    tiny_button(tr("close"))
                        .id("close-firmware")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.sheet = Sheet::None;
                            cx.notify();
                        })),
                ),
            )
    }

    fn render_macro_sheet(&self, cx: &mut Context<Self>) -> Div {
        let mut rows = div().w_full().flex().flex_col().gap(px(6.));
        for (index, entry) in self.macro_draft.iter().enumerate() {
            let selected = self.macro_edit_index == Some(index);
            rows = rows.child(
                div()
                    .id(("macro-step", index))
                    .w_full()
                    .min_h(px(46.))
                    .px(px(10.))
                    .flex()
                    .items_center()
                    .gap(px(9.))
                    .bg(if selected {
                        pixel::raised_color()
                    } else {
                        pixel::canvas_color()
                    })
                    .when(selected, |row| {
                        row.border_2().border_color(pixel::accent_color())
                    })
                    .cursor_pointer()
                    .child(pixel::badge(
                        format!("{:02}", index + 1),
                        if selected {
                            BadgeTone::Accent
                        } else {
                            BadgeTone::Neutral
                        },
                    ))
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .font_semibold()
                                    .text_color(pixel::text_color())
                                    .child(Self::macro_step_kind(&entry.step)),
                            )
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(pixel::dim_text_color())
                                    .child(short_text(&Self::macro_step_value(&entry.step), 46)),
                            ),
                    )
                    .child(pixel::badge(
                        if entry.enabled { "ON" } else { "OFF" },
                        if entry.enabled {
                            BadgeTone::Success
                        } else {
                            BadgeTone::Neutral
                        },
                    ))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.select_macro_step(index, window, cx)
                    })),
            );
        }
        if self.macro_draft.is_empty() {
            rows = rows.child(controls::empty_hint(
                "EMPTY MACRO",
                "Add a step, choose its type, then edit its value.",
            ));
        }

        let mut step_editor = div().w_full().flex().flex_col().gap(px(12.));
        if let Some(index) = self.macro_edit_index {
            if let Some(entry) = self.macro_draft.get(index) {
                step_editor = step_editor
                    .child(inspector_field(
                        "STEP TYPE",
                        format!("Step {} of {}", index + 1, self.macro_draft.len()),
                        controls::cycle_control(
                            Self::macro_step_kind(&entry.step),
                            ("macro-kind", 0usize).into(),
                            ("macro-kind", 1usize).into(),
                            cx.listener(|this, _, window, cx| {
                                this.cycle_macro_kind(-1, window, cx)
                            }),
                            cx.listener(|this, _, window, cx| this.cycle_macro_kind(1, window, cx)),
                        ),
                    ))
                    .child(
                        controls::toggle_face(tr("enabled"), entry.enabled, true)
                            .id("toggle-macro-step")
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Some(index) = this.macro_edit_index {
                                    if let Some(entry) = this.macro_draft.get_mut(index) {
                                        entry.enabled = !entry.enabled;
                                    }
                                }
                                cx.notify();
                            })),
                    );
                match &entry.step {
                    MacroStep::Keystroke { .. } => {
                        step_editor = step_editor
                            .child(controls::field_readout(
                                "KEYSTROKE",
                                Self::macro_step_value(&entry.step),
                            ))
                            .child(
                                tiny_button(if self.recording == RecordTarget::MacroStep(index) {
                                    tr("press_keys")
                                } else {
                                    tr("record")
                                })
                                .id("record-macro-step")
                                .on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        this.recording = RecordTarget::MacroStep(index);
                                        cx.notify();
                                    },
                                )),
                            );
                    }
                    MacroStep::Delay { .. } | MacroStep::Run { .. } | MacroStep::Open { .. } => {
                        step_editor = step_editor.child(inspector_field(
                            match entry.step {
                                MacroStep::Delay { .. } => "DELAY (MS)",
                                MacroStep::Run { .. } => "SHELL COMMAND",
                                _ => "URL / FILE / APPLICATION",
                            },
                            match entry.step {
                                MacroStep::Delay { .. } => "0–60000 milliseconds",
                                MacroStep::Run { .. } => "Runs detached through the system shell",
                                _ => "Opened by the operating system",
                            },
                            Input::new(&self.macro_value_input).w_full(),
                        ));
                    }
                    MacroStep::Media { op } => {
                        step_editor = step_editor.child(inspector_field(
                            "MEDIA CONTROL",
                            "Host-synthesized system media key",
                            controls::cycle_control(
                                media_label(*op),
                                ("macro-media", 0usize).into(),
                                ("macro-media", 1usize).into(),
                                cx.listener(|this, _, window, cx| {
                                    this.cycle_macro_media(-1, window, cx)
                                }),
                                cx.listener(|this, _, window, cx| {
                                    this.cycle_macro_media(1, window, cx)
                                }),
                            ),
                        ));
                    }
                }
                step_editor =
                    step_editor.child(
                        div()
                            .flex()
                            .gap(px(8.))
                            .child(tiny_button("UP").id("macro-step-up").on_click(
                                cx.listener(|this, _, _, cx| this.move_macro_step(-1, cx)),
                            ))
                            .child(tiny_button("DOWN").id("macro-step-down").on_click(
                                cx.listener(|this, _, _, cx| this.move_macro_step(1, cx)),
                            ))
                            .child(tiny_button("DELETE").id("macro-step-delete").on_click(
                                cx.listener(|this, _, window, cx| {
                                    this.remove_macro_step(window, cx)
                                }),
                            )),
                    );
            }
        }

        controls::modal_frame()
            .w(px(720.))
            .max_h(relative(0.9))
            .child(controls::modal_header(
                "MACRO WORKBENCH",
                Some(tr("build_a_short_dependable_action").into()),
            ))
            .child(
                div()
                    .w_full()
                    .flex_1()
                    .min_h(px(0.))
                    .p(px(16.))
                    .overflow_y_scrollbar()
                    .flex()
                    .gap(px(16.))
                    .child(
                        div()
                            .w(px(300.))
                            .flex()
                            .flex_col()
                            .gap(px(8.))
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(pixel::muted_text_color())
                                    .child(tr("steps_run_in_order_delays_are")),
                            )
                            .child(rows)
                            .child(tiny_button(tr("add_step")).id("add-macro-step").on_click(
                                cx.listener(|this, _, window, cx| this.add_macro_step(window, cx)),
                            )),
                    )
                    .child(div().flex_1().child(step_editor)),
            )
            .child(
                div()
                    .p(px(14.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(tiny_button(tr("test_run")).id("test-macro-draft").on_click(
                        cx.listener(|this, _, _, _| {
                            actions::execute(&Action::Macro {
                                steps: this.macro_draft.clone(),
                            });
                        }),
                    ))
                    .child(div().flex_1())
                    .child(
                        tiny_button(tr("cancel"))
                            .id("cancel-macro")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.recording = RecordTarget::None;
                                this.sheet = Sheet::None;
                                cx.notify();
                            })),
                    )
                    .child(
                        tiny_button(tr("done"))
                            .id("save-macro")
                            .on_click(cx.listener(|this, _, _, cx| this.save_macro(cx))),
                    ),
            )
    }

    fn render_sheet(&self, cx: &mut Context<Self>) -> Div {
        let modal = match self.sheet {
            Sheet::Settings => self.render_settings_sheet(cx),
            Sheet::Macro => self.render_macro_sheet(cx),
            Sheet::Firmware => self.render_firmware_sheet(cx),
            Sheet::Applications => self.render_applications_sheet(cx),
            Sheet::Icons => self.render_icons_sheet(cx),
            Sheet::KeyPicker => self.render_key_picker_sheet(cx),
            Sheet::ShortcutPicker => self.render_shortcut_picker_sheet(cx),
            Sheet::None => div(),
        };
        div()
            .absolute()
            .top_0()
            .right_0()
            .bottom_0()
            .left_0()
            .occlude()
            .on_any_mouse_down(|_, _, cx| cx.stop_propagation())
            .p(px(24.))
            .flex()
            .items_center()
            .justify_center()
            .bg(pixel::overlay_color())
            .child(modal)
    }
}

impl Render for OpenMicro {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("openmicro-root")
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .bg(pixel::canvas_color())
            .font_family(".SystemUIFont")
            .text_size(px(14.))
            .text_color(pixel::text_color())
            .capture_key_down(
                cx.listener(|this, event, window, cx| this.handle_key_down(event, window, cx)),
            )
            .child(self.render_header(cx))
            .child(self.render_banners(cx))
            .child(
                div()
                    .w_full()
                    .flex_1()
                    .min_h(px(0.))
                    .p(px(12.))
                    .flex()
                    .items_start()
                    .gap(px(12.))
                    .overflow_hidden()
                    .child(self.render_device_map(cx))
                    .child(self.render_inspector(cx)),
            )
            .when(self.recording != RecordTarget::None, |root| {
                root.child(
                    div()
                        .absolute()
                        .top(px(48.))
                        .left(relative(0.35))
                        .px(px(16.))
                        .h(px(36.))
                        .flex()
                        .items_center()
                        .bg(pixel::accent_color())
                        .font_family("Monaco")
                        .font_semibold()
                        .text_size(px(12.))
                        .text_color(pixel::on_accent_color())
                        .child("RECORDING // PRESS A KEY CHORD // ESC CANCELS"),
                )
            })
            .when(self.sheet != Sheet::None, |root| {
                root.child(self.render_sheet(cx))
            })
    }
}

fn show_main_window(cx: &mut App) {
    cx.activate(true);
    if let Some(handle) = cx.windows().into_iter().next() {
        let _ = handle.update(cx, |_, window, _| window.activate_window());
    }
}

pub fn run() {
    let app = Application::new().with_assets(crate::simple_icons::Assets);
    app.on_reopen(show_main_window);
    app.run(|cx: &mut App| {
        gpui_component::init(cx);
        // The close button only hides the resident host, so quitting is its
        // own explicit action — GPUI provides no implicit Cmd+Q. `cx.quit()`
        // runs the on_app_quit hook, which saves the config like the tray's
        // Quit does.
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.bind_keys([KeyBinding::new("cmd-q", Quit, None)]);
        cx.set_menus(vec![Menu {
            name: "OpenMicro".into(),
            items: vec![MenuItem::action(tr("mb_quit"), Quit)],
        }]);
        let _ = cx.text_system().add_fonts(vec![Cow::Borrowed(
            &include_bytes!("../resources/lucide.ttf")[..],
        )]);

        let bounds = Bounds::centered(None, size(px(820.), px(500.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                is_resizable: false,
                app_id: Some("ai.conol.openmicro".into()),
                titlebar: Some(TitleBar::title_bar_options()),
                ..Default::default()
            },
            |window, cx| {
                window.on_window_should_close(cx, |_window, cx| {
                    #[cfg(target_os = "macos")]
                    cx.hide();
                    #[cfg(not(target_os = "macos"))]
                    _window.minimize_window();
                    false
                });
                let view = cx.new(|cx| OpenMicro::new(window, cx));
                cx.new(|cx| Root::new(view, window, cx))
            },
        )
        .expect("failed to open the OpenMicro window");
        cx.activate(true);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn theme_resolution_tracks_system_or_honors_a_pinned_choice() {
        assert_eq!(
            resolve_theme(ThemeSetting::System, WindowAppearance::Light),
            pixel::ColorScheme::Light
        );
        assert_eq!(
            resolve_theme(ThemeSetting::System, WindowAppearance::VibrantDark),
            pixel::ColorScheme::Dark
        );
        assert_eq!(
            resolve_theme(ThemeSetting::Light, WindowAppearance::Dark),
            pixel::ColorScheme::Light
        );
        assert_eq!(
            resolve_theme(ThemeSetting::Dark, WindowAppearance::VibrantLight),
            pixel::ColorScheme::Dark
        );
    }

    #[test]
    fn app_update_controls_keep_sparkle_and_manual_fallback_independent() {
        assert_eq!(
            app_update_controls(false, false, false, false, false),
            AppUpdateControls {
                detail: AppUpdateDetailState::Available,
                sparkle: None,
                manual: Some(AppUpdateButtonState::DownloadDmg),
                dismissible: true,
            }
        );
        assert_eq!(
            app_update_controls(true, false, false, false, false),
            AppUpdateControls {
                detail: AppUpdateDetailState::Available,
                sparkle: Some(AppUpdateButtonState::StartSparkle),
                manual: None,
                dismissible: true,
            }
        );
        assert_eq!(
            app_update_controls(true, true, false, false, false),
            AppUpdateControls {
                detail: AppUpdateDetailState::SparkleActive,
                sparkle: Some(AppUpdateButtonState::SparkleBusy),
                manual: None,
                dismissible: false,
            }
        );
        assert_eq!(
            app_update_controls(false, false, true, false, false),
            AppUpdateControls {
                detail: AppUpdateDetailState::ManualDownloading,
                sparkle: None,
                manual: Some(AppUpdateButtonState::DownloadingDmg),
                dismissible: false,
            }
        );
        assert_eq!(
            app_update_controls(true, false, false, true, true),
            AppUpdateControls {
                detail: AppUpdateDetailState::Error,
                sparkle: Some(AppUpdateButtonState::StartSparkle),
                manual: None,
                dismissible: true,
            }
        );
    }

    #[test]
    fn simple_picker_features_common_brands_once() {
        let icons = simple_picker_icons("");
        assert_eq!(icons.len(), crate::simple_icons::icons().len());
        assert_eq!(icons.first().map(|icon| icon.slug.as_str()), Some("apple"));
        assert!(FEATURED_SIMPLE_ICONS
            .iter()
            .all(|slug| crate::simple_icons::find(slug).is_some()));
        assert_eq!(
            icons
                .iter()
                .map(|icon| icon.slug.as_str())
                .collect::<HashSet<_>>()
                .len(),
            icons.len()
        );
    }

    #[test]
    fn configured_icons_keep_catalogs_distinct() {
        assert!(configured_icon_visual("apple", 16., pixel::text_color()).is_some());
        assert!(configured_icon_visual("simple:apple", 16., pixel::text_color()).is_some());
        assert!(
            configured_icon_visual("simple:not_a_real_brand", 16., pixel::text_color()).is_none()
        );
        assert_eq!(configured_icon_label("apple"), "apple");
        assert_eq!(configured_icon_label("simple:apple"), "Apple");
    }

    #[test]
    fn picker_opens_on_the_page_containing_the_saved_icon() {
        let simple = crate::simple_icons::icons().last().unwrap();
        let ordered = simple_picker_icons("");
        let simple_index = ordered
            .iter()
            .position(|icon| icon.slug == simple.slug)
            .unwrap();
        assert_eq!(
            icon_picker_page(&crate::simple_icons::storage_value(&simple.slug)),
            simple_index / ICON_PAGE_SIZE
        );

        let lucide = crate::lucide::ICONS.last().unwrap().0;
        assert_eq!(
            icon_picker_page(lucide),
            (crate::lucide::ICONS.len() - 1) / ICON_PAGE_SIZE
        );
    }
}
