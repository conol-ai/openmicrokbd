//! HID usage tables and lookups shared by the keycode picker UI, the
//! hotkey/interception layer, and keystroke synthesis.
//!
//! Three views of the same key exist in this app and they rarely align 1:1:
//! the HID usage the firmware puts on the wire (the `Slot.code` in a profile),
//! the macOS virtual keycode (kVK_*) needed to synthesise or intercept that
//! key with CGEvent, and the w3c `Code` that global-hotkey wants for OS-level
//! registration. This module is the single place that maps between them, so
//! gaps are visible in one table instead of scattered `match`es: e.g. macOS
//! has no virtual keycode at all for F21-F24, and PrintScreen/ScrollLock/Pause
//! only exist as legacy F13-F15 aliases there (we record them as `None` rather
//! than pretend).

use global_hotkey::hotkey::{Code, Modifiers};

use crate::config::{Slot, SlotKind};

/// One keyboard-page usage and its per-platform aliases.
pub struct KeyDef {
    /// HID keyboard-page usage (what the firmware emits).
    pub usage: u16,
    /// Canonical display name for pickers and labels.
    pub name: &'static str,
    /// macOS virtual keycode (kVK_*), where one exists.
    pub macos_vk: Option<u16>,
    /// global-hotkey / w3c code, where one exists (needed to register the
    /// key as a system hotkey on Windows/Linux).
    pub hotkey: Option<Code>,
}

/// Shorthand constructor to keep the table below one line per key.
const fn k(usage: u16, name: &'static str, macos_vk: Option<u16>, hotkey: Option<Code>) -> KeyDef {
    KeyDef {
        usage,
        name,
        macos_vk,
        hotkey,
    }
}

/// Keyboard-page usages the picker offers, in display order.
///
/// macOS virtual keycodes are the ANSI kVK_* constants from Carbon's
/// Events.h; they are layout positions, not characters, which is exactly
/// what synthesis/interception needs.
pub static KEYBOARD_USAGES: &[KeyDef] = &[
    // Letters (usage 0x04..=0x1D).
    k(0x04, "A", Some(0x00), Some(Code::KeyA)),
    k(0x05, "B", Some(0x0B), Some(Code::KeyB)),
    k(0x06, "C", Some(0x08), Some(Code::KeyC)),
    k(0x07, "D", Some(0x02), Some(Code::KeyD)),
    k(0x08, "E", Some(0x0E), Some(Code::KeyE)),
    k(0x09, "F", Some(0x03), Some(Code::KeyF)),
    k(0x0A, "G", Some(0x05), Some(Code::KeyG)),
    k(0x0B, "H", Some(0x04), Some(Code::KeyH)),
    k(0x0C, "I", Some(0x22), Some(Code::KeyI)),
    k(0x0D, "J", Some(0x26), Some(Code::KeyJ)),
    k(0x0E, "K", Some(0x28), Some(Code::KeyK)),
    k(0x0F, "L", Some(0x25), Some(Code::KeyL)),
    k(0x10, "M", Some(0x2E), Some(Code::KeyM)),
    k(0x11, "N", Some(0x2D), Some(Code::KeyN)),
    k(0x12, "O", Some(0x1F), Some(Code::KeyO)),
    k(0x13, "P", Some(0x23), Some(Code::KeyP)),
    k(0x14, "Q", Some(0x0C), Some(Code::KeyQ)),
    k(0x15, "R", Some(0x0F), Some(Code::KeyR)),
    k(0x16, "S", Some(0x01), Some(Code::KeyS)),
    k(0x17, "T", Some(0x11), Some(Code::KeyT)),
    k(0x18, "U", Some(0x20), Some(Code::KeyU)),
    k(0x19, "V", Some(0x09), Some(Code::KeyV)),
    k(0x1A, "W", Some(0x0D), Some(Code::KeyW)),
    k(0x1B, "X", Some(0x07), Some(Code::KeyX)),
    k(0x1C, "Y", Some(0x10), Some(Code::KeyY)),
    k(0x1D, "Z", Some(0x06), Some(Code::KeyZ)),
    // Digits (usage 0x1E..=0x27, HID order 1..9,0).
    k(0x1E, "1", Some(0x12), Some(Code::Digit1)),
    k(0x1F, "2", Some(0x13), Some(Code::Digit2)),
    k(0x20, "3", Some(0x14), Some(Code::Digit3)),
    k(0x21, "4", Some(0x15), Some(Code::Digit4)),
    k(0x22, "5", Some(0x17), Some(Code::Digit5)),
    k(0x23, "6", Some(0x16), Some(Code::Digit6)),
    k(0x24, "7", Some(0x1A), Some(Code::Digit7)),
    k(0x25, "8", Some(0x1C), Some(Code::Digit8)),
    k(0x26, "9", Some(0x19), Some(Code::Digit9)),
    k(0x27, "0", Some(0x1D), Some(Code::Digit0)),
    // Whitespace and control.
    k(0x28, "Enter", Some(0x24), Some(Code::Enter)),
    k(0x29, "Escape", Some(0x35), Some(Code::Escape)),
    k(0x2A, "Backspace", Some(0x33), Some(Code::Backspace)),
    k(0x2B, "Tab", Some(0x30), Some(Code::Tab)),
    k(0x2C, "Space", Some(0x31), Some(Code::Space)),
    // Caps Lock (0x39): assignable as a device output; never a hotkey.
    k(0x39, "Caps Lock", Some(0x39), None),
    // Punctuation (usage 0x2D..=0x38).
    k(0x2D, "-", Some(0x1B), Some(Code::Minus)),
    k(0x2E, "=", Some(0x18), Some(Code::Equal)),
    k(0x2F, "[", Some(0x21), Some(Code::BracketLeft)),
    k(0x30, "]", Some(0x1E), Some(Code::BracketRight)),
    k(0x31, "\\", Some(0x2A), Some(Code::Backslash)),
    // 0x32 is the ISO "Non-US # and ~" key; neither macOS nor w3c codes give
    // it a stable identity of its own, so it is picker-only.
    k(0x32, "Non-US #", None, None),
    k(0x33, ";", Some(0x29), Some(Code::Semicolon)),
    k(0x34, "'", Some(0x27), Some(Code::Quote)),
    k(0x35, "`", Some(0x32), Some(Code::Backquote)),
    k(0x36, ",", Some(0x2B), Some(Code::Comma)),
    k(0x37, ".", Some(0x2F), Some(Code::Period)),
    k(0x38, "/", Some(0x2C), Some(Code::Slash)),
    // Function keys F1-F12 (usage 0x3A..=0x45).
    k(0x3A, "F1", Some(0x7A), Some(Code::F1)),
    k(0x3B, "F2", Some(0x78), Some(Code::F2)),
    k(0x3C, "F3", Some(0x63), Some(Code::F3)),
    k(0x3D, "F4", Some(0x76), Some(Code::F4)),
    k(0x3E, "F5", Some(0x60), Some(Code::F5)),
    k(0x3F, "F6", Some(0x61), Some(Code::F6)),
    k(0x40, "F7", Some(0x62), Some(Code::F7)),
    k(0x41, "F8", Some(0x64), Some(Code::F8)),
    k(0x42, "F9", Some(0x65), Some(Code::F9)),
    k(0x43, "F10", Some(0x6D), Some(Code::F10)),
    k(0x44, "F11", Some(0x67), Some(Code::F11)),
    k(0x45, "F12", Some(0x6F), Some(Code::F12)),
    // Extended function keys F13-F24 (usage 0x68..=0x73). These are the
    // pad's preferred "invisible" usages: no text side effects, and macOS
    // still has virtual keycodes up to F20. F21-F24 have none — on macOS
    // those four can be neither synthesised nor intercepted.
    k(0x68, "F13", Some(105), Some(Code::F13)),
    k(0x69, "F14", Some(107), Some(Code::F14)),
    k(0x6A, "F15", Some(113), Some(Code::F15)),
    k(0x6B, "F16", Some(106), Some(Code::F16)),
    k(0x6C, "F17", Some(64), Some(Code::F17)),
    k(0x6D, "F18", Some(79), Some(Code::F18)),
    k(0x6E, "F19", Some(80), Some(Code::F19)),
    k(0x6F, "F20", Some(90), Some(Code::F20)),
    k(0x70, "F21", None, Some(Code::F21)),
    k(0x71, "F22", None, Some(Code::F22)),
    k(0x72, "F23", None, Some(Code::F23)),
    k(0x73, "F24", None, Some(Code::F24)),
    // System keys (usage 0x46..=0x48). On macOS these only ever existed as
    // aliases of F13-F15 on legacy PC keyboards — no dedicated keycode.
    k(0x46, "Print Screen", None, Some(Code::PrintScreen)),
    k(0x47, "Scroll Lock", None, Some(Code::ScrollLock)),
    k(0x48, "Pause", None, Some(Code::Pause)),
    // Navigation cluster (usage 0x49..=0x4E). A PC keyboard's Insert key
    // reaches macOS as kVK_Help (0x72) — same position, so we use it.
    k(0x49, "Insert", Some(0x72), Some(Code::Insert)),
    k(0x4A, "Home", Some(0x73), Some(Code::Home)),
    k(0x4B, "Page Up", Some(0x74), Some(Code::PageUp)),
    k(0x4C, "Delete", Some(0x75), Some(Code::Delete)),
    k(0x4D, "End", Some(0x77), Some(Code::End)),
    k(0x4E, "Page Down", Some(0x79), Some(Code::PageDown)),
    // Arrows (usage 0x4F..=0x52).
    k(0x4F, "Right", Some(0x7C), Some(Code::ArrowRight)),
    k(0x50, "Left", Some(0x7B), Some(Code::ArrowLeft)),
    k(0x51, "Down", Some(0x7D), Some(Code::ArrowDown)),
    k(0x52, "Up", Some(0x7E), Some(Code::ArrowUp)),
];

/// A bounded group for the keystroke picker.
///
/// Makepad's dropdown menu is not scrollable, so presenting all 90 keyboard
/// usages in one popup pushes most of them off-screen. These groups keep every
/// popup short while still exposing the complete keyboard-page catalog.
pub struct KeyboardGroup {
    pub label: &'static str,
    pub usages: &'static [u16],
}

pub static KEYBOARD_GROUPS: &[KeyboardGroup] = &[
    KeyboardGroup {
        label: "Letters A–M",
        usages: &[
            0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F, 0x10,
        ],
    },
    KeyboardGroup {
        label: "Letters N–Z",
        usages: &[
            0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1A, 0x1B, 0x1C, 0x1D,
        ],
    },
    KeyboardGroup {
        label: "Numbers",
        usages: &[0x1E, 0x1F, 0x20, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27],
    },
    KeyboardGroup {
        label: "Common keys",
        usages: &[0x28, 0x29, 0x2A, 0x2B, 0x2C, 0x39],
    },
    KeyboardGroup {
        label: "Symbols",
        usages: &[
            0x2D, 0x2E, 0x2F, 0x30, 0x31, 0x32, 0x33, 0x34, 0x35, 0x36, 0x37, 0x38,
        ],
    },
    KeyboardGroup {
        label: "Function F1–F12",
        usages: &[
            0x3A, 0x3B, 0x3C, 0x3D, 0x3E, 0x3F, 0x40, 0x41, 0x42, 0x43, 0x44, 0x45,
        ],
    },
    KeyboardGroup {
        label: "Function F13–F24",
        usages: &[
            0x68, 0x69, 0x6A, 0x6B, 0x6C, 0x6D, 0x6E, 0x6F, 0x70, 0x71, 0x72, 0x73,
        ],
    },
    KeyboardGroup {
        label: "Navigation",
        usages: &[0x49, 0x4A, 0x4B, 0x4C, 0x4D, 0x4E, 0x4F, 0x50, 0x51, 0x52],
    },
    KeyboardGroup {
        label: "System keys",
        usages: &[0x46, 0x47, 0x48],
    },
];

pub fn keyboard_group_index(usage: u16) -> Option<usize> {
    KEYBOARD_GROUPS
        .iter()
        .position(|group| group.usages.contains(&usage))
}

/// Consumer-page usages the picker offers: (usage, display name).
/// These need no per-platform aliases — the OS handles consumer usages from
/// any HID device natively, so the pad emits them directly.
pub static CONSUMER_USAGES: &[(u16, &'static str)] = &[
    (0xE9, "Volume Up"),
    (0xEA, "Volume Down"),
    (0xE2, "Mute"),
    (0xB0, "Play"),
    (0xCD, "Play/Pause"),
    (0xB5, "Next Track"),
    (0xB6, "Previous Track"),
    (0xB7, "Stop"),
    (0x6F, "Brightness Up"),
    (0x70, "Brightness Down"),
    (0x029D, "Globe / Fn"),
    (0x019E, "Lock Screen"),
];

/// Full definition for a keyboard-page usage (linear scan: the table is
/// small and this only runs on UI events).
pub fn keyboard_def(usage: u16) -> Option<&'static KeyDef> {
    KEYBOARD_USAGES.iter().find(|d| d.usage == usage)
}

/// Display name for a keyboard-page usage.
pub fn keyboard_name(usage: u16) -> Option<&'static str> {
    keyboard_def(usage).map(|d| d.name)
}

/// Display name for a consumer-page usage.
pub fn consumer_name(usage: u16) -> Option<&'static str> {
    CONSUMER_USAGES
        .iter()
        .find(|(u, _)| *u == usage)
        .map(|(_, n)| *n)
}

/// global-hotkey code for a keyboard-page usage, if it has one.
pub fn hotkey_code(usage: u16) -> Option<Code> {
    keyboard_def(usage).and_then(|d| d.hotkey)
}

/// Can this host OS intercept (and re-synthesise) this usage?
///
/// macOS interception runs on a CGEvent tap keyed by virtual keycode, so it
/// needs `macos_vk`; Windows/Linux go through global-hotkey registration,
/// which needs a w3c `Code`. The picker uses this to warn about bindings
/// that would emit into the void on the current machine.
pub fn interceptable_here(usage: u16) -> bool {
    let Some(def) = keyboard_def(usage) else {
        return false;
    };
    if cfg!(target_os = "macos") {
        def.macos_vk.is_some()
    } else {
        def.hotkey.is_some()
    }
}

/// Compact labels for consumer usages, sized for keycap-style UI chips.
fn consumer_short_name(usage: u16) -> Option<&'static str> {
    Some(match usage {
        0xE9 => "Vol +",
        0xEA => "Vol -",
        0xE2 => "Mute",
        0xB0 => "Play",
        0xCD => "Play/Pause",
        0xB5 => "Next",
        0xB6 => "Prev",
        0xB7 => "Stop",
        0x6F => "Bright +",
        0x70 => "Bright -",
        0x029D => "Globe",
        0x019E => "Lock",
        _ => return None,
    })
}

/// The four modifier pairs of the HID bitmask, in display order
/// (low nibble = left hand, high nibble = right; we show them the same).
const MOD_PAIRS: [(u8, &str, &str); 4] = [
    // (left-hand bit, macOS symbol, textual name)
    (0x01, "\u{2303}", "Ctrl"),  // ⌃ Control
    (0x04, "\u{2325}", "Alt"),   // ⌥ Option
    (0x02, "\u{21E7}", "Shift"), // ⇧ Shift
    (0x08, "\u{2318}", "Win"),   // ⌘ Command / Win / Super
];

/// Human-readable modifier prefix from the HID bitmask: "⌃⇧⌘" on macOS,
/// "Ctrl+Shift+Win+" elsewhere (trailing separator included so a key name
/// can be appended directly). Left and right variants render the same.
/// Display label for an emitted keyboard slot: named key, bare modifier
/// hold ({mods, code 0} — the firmware ORs mods with an empty keycode), or
/// nothing.
pub fn emitted_key_label(mods: u8, code: u16) -> String {
    if code != 0 {
        keyboard_name(code)
            .map(str::to_string)
            .unwrap_or_else(|| format!("0x{code:02X}"))
    } else if mods != 0 {
        format!("{} (hold)", mods_label(mods))
    } else {
        "—".to_string()
    }
}

pub fn mods_label(mods: u8) -> String {
    let mac = cfg!(target_os = "macos");
    let mut out = String::new();
    for (bit, sym, name) in MOD_PAIRS {
        // Fold the right-hand modifier (bits 4-7) onto its left twin.
        if mods & (bit | bit << 4) != 0 {
            if mac {
                out.push_str(sym);
            } else {
                out.push_str(name);
                out.push('+');
            }
        }
    }
    out
}

/// Human-readable label for what a slot emits: "⇧F13", "Vol +", "—" (None).
/// Unknown codes fall back to the raw usage in hex so nothing is invisible.
pub fn slot_label(slot: &Slot) -> String {
    match slot.kind {
        SlotKind::None => "\u{2014}".to_string(),
        SlotKind::Keyboard => {
            let name = keyboard_name(slot.code)
                .map(str::to_string)
                .unwrap_or_else(|| format!("0x{:02X}", slot.code));
            format!("{}{}", mods_label(slot.mods), name)
        }
        // Consumer usages carry no modifiers on the wire; ignore slot.mods.
        SlotKind::Consumer => consumer_short_name(slot.code)
            .map(str::to_string)
            .unwrap_or_else(|| format!("0x{:02X}", slot.code)),
    }
}

/// HID modifier bitmask -> global-hotkey `Modifiers`. Left and right hands
/// collapse to the same flag (global-hotkey has no sided modifiers; note it
/// rewrites META to SUPER internally, so SUPER is the canonical GUI flag).
pub fn hotkey_mods(mods: u8) -> Modifiers {
    // Fold right-hand bits (4-7) onto the left-hand ones (0-3).
    let folded = (mods & 0x0F) | (mods >> 4);
    let mut out = Modifiers::empty();
    if folded & 0x01 != 0 {
        out |= Modifiers::CONTROL;
    }
    if folded & 0x02 != 0 {
        out |= Modifiers::SHIFT;
    }
    if folded & 0x04 != 0 {
        out |= Modifiers::ALT;
    }
    if folded & 0x08 != 0 {
        out |= Modifiers::SUPER;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_has_no_duplicate_usages() {
        for (i, a) in KEYBOARD_USAGES.iter().enumerate() {
            for b in &KEYBOARD_USAGES[i + 1..] {
                assert_ne!(a.usage, b.usage, "duplicate usage 0x{:02X}", a.usage);
            }
        }
    }

    #[test]
    fn picker_groups_cover_every_key_once_and_stay_short() {
        let grouped: Vec<u16> = KEYBOARD_GROUPS
            .iter()
            .flat_map(|group| group.usages.iter().copied())
            .collect();
        assert!(KEYBOARD_GROUPS
            .iter()
            .all(|group| !group.usages.is_empty() && group.usages.len() <= 13));
        assert_eq!(grouped.len(), KEYBOARD_USAGES.len());
        for key in KEYBOARD_USAGES {
            assert_eq!(
                grouped.iter().filter(|usage| **usage == key.usage).count(),
                1,
                "usage 0x{:02X} must appear in exactly one picker group",
                key.usage
            );
        }
    }

    #[test]
    fn lookups_cover_the_spec_examples() {
        assert_eq!(keyboard_name(0x68), Some("F13"));
        assert_eq!(keyboard_name(0x4B), Some("Page Up"));
        assert_eq!(consumer_name(0xE9), Some("Volume Up"));
        assert_eq!(consumer_name(0xB0), Some("Play"));
        assert_eq!(consumer_name(0x029D), Some("Globe / Fn"));
        assert_eq!(consumer_name(0x019E), Some("Lock Screen"));
        assert_eq!(
            slot_label(&Slot {
                kind: SlotKind::Consumer,
                mods: 0,
                code: 0x029D
            }),
            "Globe"
        );
        assert_eq!(
            slot_label(&Slot {
                kind: SlotKind::Consumer,
                mods: 0,
                code: 0x019E
            }),
            "Lock"
        );
        assert_eq!(hotkey_code(0x68), Some(Code::F13));
        // F21-F24: registerable off-macOS, no virtual keycode on macOS.
        let f21 = keyboard_def(0x70).unwrap();
        assert_eq!(f21.macos_vk, None);
        assert_eq!(f21.hotkey, Some(Code::F21));
    }

    #[test]
    fn mods_fold_left_and_right() {
        // RShift (0x20) must render and register like LShift (0x02).
        assert_eq!(hotkey_mods(0x20), Modifiers::SHIFT);
        assert_eq!(hotkey_mods(0x02), Modifiers::SHIFT);
        assert_eq!(
            hotkey_mods(0xFF),
            Modifiers::CONTROL | Modifiers::SHIFT | Modifiers::ALT | Modifiers::SUPER
        );
        assert_eq!(mods_label(0x02), mods_label(0x20));
    }

    #[test]
    fn slot_labels() {
        let none = Slot::default();
        assert_eq!(slot_label(&none), "\u{2014}");
        let vol = Slot {
            kind: SlotKind::Consumer,
            mods: 0,
            code: 0xE9,
        };
        assert_eq!(slot_label(&vol), "Vol +");
        let shift_f13 = Slot {
            kind: SlotKind::Keyboard,
            mods: 0x02,
            code: 0x68,
        };
        if cfg!(target_os = "macos") {
            assert_eq!(slot_label(&shift_f13), "\u{21E7}F13");
        } else {
            assert_eq!(slot_label(&shift_f13), "Shift+F13");
        }
    }
}
