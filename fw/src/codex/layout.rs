//! The Work Louder keymap: what the Input app writes to `keymap.json` and
//! how the pad turns it into key behaviour.
//!
//! Device-side format (as `@worklouder/input` serialises it):
//!
//! ```json
//! {"version":1,"activeProfileId":0,
//!  "profiles":[{"id":0,"name":"Codex","layers":[
//!     {"id":0,"name":"ChatGPT","color":16711680,"os":0,
//!      "layout":{"keymap":[["KV_OAI_AG00","KV_OAI_AG01"],["KV_OAI_AG02",…]],
//!                "encoders":[["KV_OAI_ENC_CC","KV_OAI_ENC_CW","KV_OAI_ENC_CLK"]],
//!                "buttons":[["KC_MPLY"]],
//!                "joystick":{"type":"VENDOR","sectors":[]}},
//!      "lights":{"backlight":{…},"underglow":{…}}}]}],
//!  "macros":[{"id":0,"name":"…","actions":[{"kc":"KC_LGUI","delay":0,"act":1},…]}],
//!  "multiActions":[{"id":0,"kcOnTap":"KC_A","kcOnHold":"KC_B",…,"tt":250}], …}
//! ```
//!
//! `keymap` rows are the physical rows top to bottom (2 / 4 / 4 / 3 keys);
//! flattened they are key positions 0..=12. Keycodes: `KC_*` (QMK names →
//! HID usages), `KV_OAI_*` (the Codex Micro events), `KI_LS<n>` / `KI_LM<n>`
//! / `KI_PS<n>` (layer toggle / momentary layer / profile), `KA_A<n>` /
//! `KA_M<n>` (macro / multi-action, by id), `SA_<n>` (smart action, run by
//! the Input app), `KI_CS_*` (cheat sheet), `KI_BLUP` / `KI_BLDW`.
//!
//! Everything is read straight out of the JSON slice (which lives in flash)
//! — nothing here allocates or keeps the document in RAM.

use super::wire::{as_str, find_key, for_each_elem, parse_milli, parse_u32};

pub const KEYS: usize = 13;
pub const ENCODER_CCW: usize = 0;
pub const ENCODER_CW: usize = 1;
pub const ENCODER_PRESS: usize = 2;
pub const MAX_SECTORS: usize = 8;
pub const MAX_LAYERS: u8 = 6;

/// Codex Micro control ids for `Binding::Oai`: key positions 0..=12, then
/// the dial.
pub const OAI_ENC_CCW: u8 = 13;
pub const OAI_ENC_CW: u8 = 14;
pub const OAI_ENC_PRESS: u8 = 15;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Binding {
    #[default]
    None,
    /// A Codex Micro control (see `OAI_*` and key positions).
    Oai(u8),
    /// Keyboard usage with a modifier mask (bit0 LCtrl … bit7 RGui); a bare
    /// modifier key has `code` 0.
    Key {
        mods: u8,
        code: u8,
    },
    Consumer(u16),
    /// `KI_LS<n>`: toggle layer n (0-based here).
    LayerToggle(u8),
    /// `KI_LM<n>`: layer n while held.
    LayerHold(u8),
    /// `KI_PS<n>`: profile n (0-based here).
    Profile(u8),
    /// `KI_FP`: the profile's function layer while held (unsupported: no-op).
    Function,
    /// `KA_A<n>`: macro by id.
    Macro(u16),
    /// `KA_M<n>`: multi-action by id (its tap keycode is used).
    Multi(u16),
    /// `SA_<n>`: smart action by id, executed by the Input app.
    Smart(u16),
    /// `KI_CS_HIDE` 0 / `KI_CS_SHOW` 1 / `KI_CS_TOGGLE` 2 / `KI_CS_SHOW_TMP` 3.
    CheatSheet(u8),
    /// `KI_BLUP` +1 / `KI_BLDW` -1.
    Backlight(i8),
    /// A keycode we do not implement (Bluetooth slots, radial close, …).
    Unsupported,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Joystick {
    #[default]
    None,
    /// `VENDOR`: the Codex Micro analog stick (`v.oai.rad`).
    Vendor,
    /// `RADIAL` / `JOYSTICK`: direction sectors bound to keycodes.
    Sectors,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Sector {
    pub binding: Binding,
    /// Start / end angle in thousandths of a turn (right 0, down 250, left
    /// 500, up 750), wrapping when `a1 > a2`.
    pub a1: u16,
    pub a2: u16,
}

impl Sector {
    pub fn contains(&self, angle_milli: u16) -> bool {
        if self.a1 <= self.a2 {
            (self.a1..self.a2).contains(&angle_milli)
        } else {
            angle_milli >= self.a1 || angle_milli < self.a2
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Layout {
    pub keys: [Binding; KEYS],
    pub encoder: [Binding; 3],
    /// `buttons[0][0]` — the touch pad on this hardware.
    pub touch: Binding,
    pub joystick: Joystick,
    pub sectors: [Sector; MAX_SECTORS],
    pub sector_count: u8,
    pub profile_index: u8,
    pub layer_index: u8,
    pub profile_count: u8,
    pub layer_count: u8,
}

// ---- keycodes ---------------------------------------------------------------

const KIND_KEY: u8 = 0;
const KIND_MOD: u8 = 1;
const KIND_CONSUMER: u8 = 2;

/// QMK keycode names Input offers → HID usage. Keyboard-page codes and
/// modifiers (`KIND_MOD`: the value is the modifier bit) and consumer-page
/// usages (`KIND_CONSUMER`).
#[rustfmt::skip]
const KEYCODES: &[(&str, u8, u16)] = &[
    ("A", KIND_KEY, 0x04), ("B", KIND_KEY, 0x05), ("C", KIND_KEY, 0x06), ("D", KIND_KEY, 0x07),
    ("E", KIND_KEY, 0x08), ("F", KIND_KEY, 0x09), ("G", KIND_KEY, 0x0A), ("H", KIND_KEY, 0x0B),
    ("I", KIND_KEY, 0x0C), ("J", KIND_KEY, 0x0D), ("K", KIND_KEY, 0x0E), ("L", KIND_KEY, 0x0F),
    ("M", KIND_KEY, 0x10), ("N", KIND_KEY, 0x11), ("O", KIND_KEY, 0x12), ("P", KIND_KEY, 0x13),
    ("Q", KIND_KEY, 0x14), ("R", KIND_KEY, 0x15), ("S", KIND_KEY, 0x16), ("T", KIND_KEY, 0x17),
    ("U", KIND_KEY, 0x18), ("V", KIND_KEY, 0x19), ("W", KIND_KEY, 0x1A), ("X", KIND_KEY, 0x1B),
    ("Y", KIND_KEY, 0x1C), ("Z", KIND_KEY, 0x1D),
    ("1", KIND_KEY, 0x1E), ("2", KIND_KEY, 0x1F), ("3", KIND_KEY, 0x20), ("4", KIND_KEY, 0x21),
    ("5", KIND_KEY, 0x22), ("6", KIND_KEY, 0x23), ("7", KIND_KEY, 0x24), ("8", KIND_KEY, 0x25),
    ("9", KIND_KEY, 0x26), ("0", KIND_KEY, 0x27),
    ("ENT", KIND_KEY, 0x28), ("ESC", KIND_KEY, 0x29), ("BSPC", KIND_KEY, 0x2A), ("TAB", KIND_KEY, 0x2B),
    ("SPC", KIND_KEY, 0x2C), ("MINS", KIND_KEY, 0x2D), ("EQL", KIND_KEY, 0x2E), ("LBRC", KIND_KEY, 0x2F),
    ("RBRC", KIND_KEY, 0x30), ("BSLS", KIND_KEY, 0x31), ("NUHS", KIND_KEY, 0x32), ("SCLN", KIND_KEY, 0x33),
    ("QUOT", KIND_KEY, 0x34), ("GRV", KIND_KEY, 0x35), ("COMM", KIND_KEY, 0x36), ("DOT", KIND_KEY, 0x37),
    ("SLSH", KIND_KEY, 0x38), ("CAPS", KIND_KEY, 0x39),
    ("F1", KIND_KEY, 0x3A), ("F2", KIND_KEY, 0x3B), ("F3", KIND_KEY, 0x3C), ("F4", KIND_KEY, 0x3D),
    ("F5", KIND_KEY, 0x3E), ("F6", KIND_KEY, 0x3F), ("F7", KIND_KEY, 0x40), ("F8", KIND_KEY, 0x41),
    ("F9", KIND_KEY, 0x42), ("F10", KIND_KEY, 0x43), ("F11", KIND_KEY, 0x44), ("F12", KIND_KEY, 0x45),
    ("PSCR", KIND_KEY, 0x46), ("SLCK", KIND_KEY, 0x47), ("PAUS", KIND_KEY, 0x48), ("INS", KIND_KEY, 0x49),
    ("HOME", KIND_KEY, 0x4A), ("PGUP", KIND_KEY, 0x4B), ("DEL", KIND_KEY, 0x4C), ("END", KIND_KEY, 0x4D),
    ("PGDN", KIND_KEY, 0x4E), ("RGHT", KIND_KEY, 0x4F), ("LEFT", KIND_KEY, 0x50), ("DOWN", KIND_KEY, 0x51),
    ("UP", KIND_KEY, 0x52), ("NLCK", KIND_KEY, 0x53), ("PSLS", KIND_KEY, 0x54), ("PAST", KIND_KEY, 0x55),
    ("PMNS", KIND_KEY, 0x56), ("PPLS", KIND_KEY, 0x57), ("PENT", KIND_KEY, 0x58),
    ("P1", KIND_KEY, 0x59), ("P2", KIND_KEY, 0x5A), ("P3", KIND_KEY, 0x5B), ("P4", KIND_KEY, 0x5C),
    ("P5", KIND_KEY, 0x5D), ("P6", KIND_KEY, 0x5E), ("P7", KIND_KEY, 0x5F), ("P8", KIND_KEY, 0x60),
    ("P9", KIND_KEY, 0x61), ("P0", KIND_KEY, 0x62), ("PDOT", KIND_KEY, 0x63), ("NUBS", KIND_KEY, 0x64),
    ("APP", KIND_KEY, 0x65), ("KB_POWER", KIND_KEY, 0x66), ("PEQL", KIND_KEY, 0x67),
    ("F13", KIND_KEY, 0x68), ("F14", KIND_KEY, 0x69), ("F15", KIND_KEY, 0x6A), ("F16", KIND_KEY, 0x6B),
    ("F17", KIND_KEY, 0x6C), ("F18", KIND_KEY, 0x6D), ("F19", KIND_KEY, 0x6E), ("F20", KIND_KEY, 0x6F),
    ("F21", KIND_KEY, 0x70), ("F22", KIND_KEY, 0x71), ("F23", KIND_KEY, 0x72), ("F24", KIND_KEY, 0x73),
    ("EXEC", KIND_KEY, 0x74), ("HELP", KIND_KEY, 0x75), ("MENU", KIND_KEY, 0x76), ("SELECT", KIND_KEY, 0x77),
    ("STOP", KIND_KEY, 0x78), ("AGAIN", KIND_KEY, 0x79), ("UNDO", KIND_KEY, 0x7A), ("CUT", KIND_KEY, 0x7B),
    ("COPY", KIND_KEY, 0x7C), ("PASTE", KIND_KEY, 0x7D), ("FIND", KIND_KEY, 0x7E),
    ("LCAP", KIND_KEY, 0x82), ("LSCR", KIND_KEY, 0x84), ("PCMM", KIND_KEY, 0x85), ("KP_EQUAL_AS400", KIND_KEY, 0x86),
    ("RO", KIND_KEY, 0x87), ("KANA", KIND_KEY, 0x88), ("JYEN", KIND_KEY, 0x89), ("HENK", KIND_KEY, 0x8A),
    ("MHEN", KIND_KEY, 0x8B), ("INT6", KIND_KEY, 0x8C), ("INT7", KIND_KEY, 0x8D), ("INT8", KIND_KEY, 0x8E),
    ("INT9", KIND_KEY, 0x8F), ("HAEN", KIND_KEY, 0x90), ("HANJ", KIND_KEY, 0x91), ("LANG3", KIND_KEY, 0x92),
    ("LANG4", KIND_KEY, 0x93), ("LANG5", KIND_KEY, 0x94), ("LANG6", KIND_KEY, 0x95), ("LANG7", KIND_KEY, 0x96),
    ("LANG8", KIND_KEY, 0x97), ("LANG9", KIND_KEY, 0x98), ("ERAS", KIND_KEY, 0x99), ("SYSREQ", KIND_KEY, 0x9A),
    ("CANCEL", KIND_KEY, 0x9B), ("CLEAR", KIND_KEY, 0x9C), ("PRIOR", KIND_KEY, 0x9D), ("RETURN", KIND_KEY, 0x9E),
    ("SEPAR", KIND_KEY, 0x9F), ("OUT", KIND_KEY, 0xA0), ("OPER", KIND_KEY, 0xA1), ("CLEAR_AGAIN", KIND_KEY, 0xA2),
    ("CRSEL", KIND_KEY, 0xA3), ("EXSEL", KIND_KEY, 0xA4),
    ("LCTL", KIND_MOD, 0x01), ("LSFT", KIND_MOD, 0x02), ("LALT", KIND_MOD, 0x04), ("LGUI", KIND_MOD, 0x08),
    ("RCTL", KIND_MOD, 0x10), ("RSFT", KIND_MOD, 0x20), ("RALT", KIND_MOD, 0x40), ("RGUI", KIND_MOD, 0x80),
    ("MUTE", KIND_CONSUMER, 0xE2), ("VOLU", KIND_CONSUMER, 0xE9), ("VOLD", KIND_CONSUMER, 0xEA),
    ("MNXT", KIND_CONSUMER, 0xB5), ("MPRV", KIND_CONSUMER, 0xB6), ("MSTP", KIND_CONSUMER, 0xB7),
    ("MPLY", KIND_CONSUMER, 0xCD), ("MSEL", KIND_CONSUMER, 0x183), ("EJCT", KIND_CONSUMER, 0xB8),
    ("MFFD", KIND_CONSUMER, 0xB3), ("MRWD", KIND_CONSUMER, 0xB4), ("BRIU", KIND_CONSUMER, 0x6F),
    ("BRID", KIND_CONSUMER, 0x70), ("PWR", KIND_CONSUMER, 0x30), ("POWER", KIND_CONSUMER, 0x30),
    ("SLEP", KIND_CONSUMER, 0x32), ("WAKE", KIND_CONSUMER, 0x33),
];

fn parse_index(digits: &[u8]) -> Option<u16> {
    if digits.is_empty() || digits.len() > 4 || !digits.iter().all(u8::is_ascii_digit) {
        return None;
    }
    Some(digits.iter().fold(0u16, |n, &c| n * 10 + (c - b'0') as u16))
}

/// The behaviour a keycode string stands for.
pub fn binding(kc: &[u8]) -> Binding {
    if kc.is_empty() || kc == b"KC_NONE" || kc == b"KC_NO" {
        return Binding::None;
    }
    if let Some(rest) = kc.strip_prefix(b"KV_OAI_") {
        return match rest {
            b"ENC_CC" => Binding::Oai(OAI_ENC_CCW),
            b"ENC_CW" => Binding::Oai(OAI_ENC_CW),
            b"ENC_CLK" => Binding::Oai(OAI_ENC_PRESS),
            _ => {
                let digits = rest
                    .strip_prefix(b"AG")
                    .or_else(|| rest.strip_prefix(b"ACT"));
                match digits.and_then(parse_index) {
                    Some(n) if (n as usize) < KEYS => Binding::Oai(n as u8),
                    _ => Binding::Unsupported,
                }
            }
        };
    }
    if let Some(name) = kc.strip_prefix(b"KC_") {
        if name == b"FUNC" {
            return Binding::Function;
        }
        for (n, kind, code) in KEYCODES {
            if n.as_bytes() == name {
                return match *kind {
                    KIND_KEY => Binding::Key {
                        mods: 0,
                        code: *code as u8,
                    },
                    KIND_MOD => Binding::Key {
                        mods: *code as u8,
                        code: 0,
                    },
                    _ => Binding::Consumer(*code),
                };
            }
        }
        return Binding::Unsupported;
    }
    if let Some(rest) = kc.strip_prefix(b"KI_") {
        return match rest {
            b"FP" => Binding::Function,
            b"CS_HIDE" => Binding::CheatSheet(0),
            b"CS_SHOW" => Binding::CheatSheet(1),
            b"CS_TOGGLE" => Binding::CheatSheet(2),
            b"CS_SHOW_TMP" => Binding::CheatSheet(3),
            b"BLUP" => Binding::Backlight(1),
            b"BLDW" => Binding::Backlight(-1),
            _ => {
                if let Some(n) = rest.strip_prefix(b"LS").and_then(parse_index) {
                    if n >= 1 && n as u8 <= MAX_LAYERS {
                        return Binding::LayerToggle(n as u8);
                    }
                }
                if let Some(n) = rest.strip_prefix(b"LM").and_then(parse_index) {
                    if n >= 1 && n as u8 <= MAX_LAYERS {
                        return Binding::LayerHold(n as u8);
                    }
                }
                if let Some(n) = rest.strip_prefix(b"PS").and_then(parse_index) {
                    if n >= 1 {
                        return Binding::Profile(n as u8 - 1);
                    }
                }
                Binding::Unsupported
            }
        };
    }
    // Device-side spelling of the app's KA_<n> / KM_<n>.
    if let Some(n) = kc.strip_prefix(b"KA_A").and_then(parse_index) {
        return Binding::Macro(n);
    }
    if let Some(n) = kc.strip_prefix(b"KA_M").and_then(parse_index) {
        return Binding::Multi(n);
    }
    if let Some(n) = kc.strip_prefix(b"KM_").and_then(parse_index) {
        return Binding::Multi(n);
    }
    if let Some(n) = kc.strip_prefix(b"KA_").and_then(parse_index) {
        return Binding::Macro(n);
    }
    if let Some(n) = kc.strip_prefix(b"SA_").and_then(parse_index) {
        return Binding::Smart(n);
    }
    Binding::Unsupported
}

// ---- document navigation ----------------------------------------------------

/// The `profiles` element to use: the one whose `id` is `activeProfileId`
/// (or the first) unless `want` picks an index explicitly. Returns the
/// object slice, its index, and the profile count.
fn select_profile(doc: &[u8], want: Option<u8>) -> Option<(&[u8], u8, u8)> {
    let profiles = find_key(doc, b"profiles")?;
    let active = find_key(doc, b"activeProfileId").and_then(parse_u32);
    let mut count = 0u8;
    let mut chosen: Option<(&[u8], u8)> = None;
    let mut first: Option<&[u8]> = None;
    for_each_elem(profiles, |p| {
        let idx = count;
        count = count.saturating_add(1);
        if first.is_none() {
            first = Some(p);
        }
        let hit = match want {
            Some(w) => idx == w,
            None => active.is_some() && find_key(p, b"id").and_then(parse_u32) == active,
        };
        if hit && chosen.is_none() {
            chosen = Some((p, idx));
        }
    });
    if count == 0 {
        return None;
    }
    let (p, idx) = chosen.or(first.map(|f| (f, 0)))?;
    Some((p, idx, count))
}

/// Layer `index` of `profile` (falling back to the first), with the count.
fn select_layer(profile: &[u8], index: u8) -> Option<(&[u8], u8, u8)> {
    let layers = find_key(profile, b"layers")?;
    let mut count = 0u8;
    let mut chosen: Option<&[u8]> = None;
    let mut first: Option<&[u8]> = None;
    for_each_elem(layers, |l| {
        if count == index {
            chosen = Some(l);
        }
        if first.is_none() {
            first = Some(l);
        }
        count = count.saturating_add(1);
    });
    if count == 0 {
        return None;
    }
    match chosen {
        Some(l) => Some((l, index, count)),
        None => Some((first?, 0, count)),
    }
}

fn keycode_at(row: &[u8], want: usize) -> Option<Binding> {
    let mut i = 0usize;
    let mut found = None;
    for_each_elem(row, |kc| {
        if i == want {
            // Rows hold keycode strings; tolerate the app-side `{keycode}`
            // objects too.
            let s = as_str(kc).or_else(|| find_key(kc, b"keycode").and_then(as_str));
            found = Some(s.map(binding).unwrap_or(Binding::Unsupported));
        }
        i += 1;
    });
    found
}

/// Build the layout for a profile/layer of the document. `profile` None
/// selects `activeProfileId`.
pub fn parse(doc: &[u8], profile: Option<u8>, layer: u8) -> Option<Layout> {
    let (prof, pidx, pcount) = select_profile(doc, profile)?;
    let (lay, lidx, lcount) = select_layer(prof, layer)?;
    let layout = find_key(lay, b"layout")?;
    let mut out = Layout {
        profile_index: pidx,
        layer_index: lidx,
        profile_count: pcount,
        layer_count: lcount,
        ..Layout::default()
    };
    // keys: rows flattened in reading order
    if let Some(rows) = find_key(layout, b"keymap") {
        let mut pos = 0usize;
        for_each_elem(rows, |row| {
            let mut i = 0usize;
            loop {
                if pos >= KEYS {
                    break;
                }
                match keycode_at(row, i) {
                    Some(b) => {
                        out.keys[pos] = b;
                        pos += 1;
                        i += 1;
                    }
                    None => break,
                }
            }
        });
    }
    if let Some(encs) = find_key(layout, b"encoders") {
        let mut first = true;
        for_each_elem(encs, |enc| {
            if first {
                first = false;
                for (i, slot) in out.encoder.iter_mut().enumerate() {
                    if let Some(b) = keycode_at(enc, i) {
                        *slot = b;
                    }
                }
            }
        });
    }
    if let Some(buttons) = find_key(layout, b"buttons") {
        let mut first = true;
        for_each_elem(buttons, |row| {
            if first {
                first = false;
                if let Some(b) = keycode_at(row, 0) {
                    out.touch = b;
                }
            }
        });
    }
    if let Some(joy) = find_key(layout, b"joystick") {
        match find_key(joy, b"type").and_then(as_str) {
            Some(b"VENDOR") => out.joystick = Joystick::Vendor,
            Some(_) => {
                out.joystick = Joystick::Sectors;
                if let Some(sectors) = find_key(joy, b"sectors") {
                    for_each_elem(sectors, |s| {
                        let n = out.sector_count as usize;
                        if n >= MAX_SECTORS {
                            return;
                        }
                        let b = find_key(s, b"k")
                            .and_then(as_str)
                            .map(binding)
                            .unwrap_or(Binding::None);
                        let a1 = find_key(s, b"a1").and_then(parse_milli).unwrap_or(0);
                        let a2 = find_key(s, b"a2").and_then(parse_milli).unwrap_or(0);
                        out.sectors[n] = Sector {
                            binding: b,
                            a1: a1.clamp(0, 1000) as u16,
                            a2: a2.clamp(0, 1000) as u16,
                        };
                        out.sector_count += 1;
                    });
                }
            }
            None => {}
        }
    }
    Some(out)
}

/// The active layer's `lights` object (`{"backlight":…,"underglow":…}`), if
/// the layer defines one.
pub fn layer_lights(doc: &[u8], profile: Option<u8>, layer: u8) -> Option<&[u8]> {
    let (prof, _, _) = select_profile(doc, profile)?;
    let (lay, _, _) = select_layer(prof, layer)?;
    find_key(lay, b"lights")
}

/// One macro step: keycode, delay after it in ms, and 0 release / 1 press /
/// 2 click.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Step {
    pub binding: Binding,
    pub delay_ms: u16,
    pub act: u8,
}

pub const ACT_RELEASE: u8 = 0;
pub const ACT_PRESS: u8 = 1;
#[allow(dead_code)]
pub const ACT_CLICK: u8 = 2;

/// Visit the steps of macro `id`. Returns false if there is no such macro.
pub fn macro_steps(doc: &[u8], id: u16, mut f: impl FnMut(Step)) -> bool {
    let Some(macros) = find_key(doc, b"macros") else {
        return false;
    };
    let mut found = false;
    for_each_elem(macros, |m| {
        if found || find_key(m, b"id").and_then(parse_u32) != Some(id as u32) {
            return;
        }
        found = true;
        if let Some(actions) = find_key(m, b"actions") {
            for_each_elem(actions, |a| {
                let kc = find_key(a, b"kc")
                    .and_then(as_str)
                    .map(binding)
                    .unwrap_or(Binding::None);
                let delay = find_key(a, b"delay")
                    .and_then(parse_u32)
                    .unwrap_or(0)
                    .min(10_000) as u16;
                let act = find_key(a, b"act")
                    .and_then(parse_u32)
                    .unwrap_or(ACT_PRESS as u32)
                    .min(2) as u8;
                f(Step {
                    binding: kc,
                    delay_ms: delay,
                    act,
                });
            });
        }
    });
    found
}

/// The tap keycode of multi-action `id` (hold / double-tap variants are not
/// implemented).
pub fn multi_tap(doc: &[u8], id: u16) -> Option<Binding> {
    let multis = find_key(doc, b"multiActions")?;
    let mut found = None;
    for_each_elem(multis, |m| {
        if found.is_none() && find_key(m, b"id").and_then(parse_u32) == Some(id as u32) {
            found = Some(
                find_key(m, b"kcOnTap")
                    .and_then(as_str)
                    .map(binding)
                    .unwrap_or(Binding::None),
            );
        }
    });
    found
}

/// Smart action kinds, as `smart_actions.json` spells them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SmartKind {
    Text,
    Command,
    App,
    Url,
}

impl SmartKind {
    /// The notification the Input app listens for.
    pub fn method(self) -> &'static [u8] {
        match self {
            SmartKind::Text => b"kb.sa.inserttext",
            SmartKind::Command => b"kb.sa.exec",
            SmartKind::App => b"kb.sa.openapp",
            SmartKind::Url => b"kb.sa.openurl",
        }
    }
}

/// Smart action `id` from a `smart_actions.json` document: its kind and
/// the raw JSON of its `payload` object (`{"text":…}`, `{"cmd":…}`,
/// `{"name":…,"path":…}`, `{"url":…}`), which is what the notification
/// carries as `params`.
pub fn smart_action(doc: &[u8], id: u16) -> Option<(SmartKind, &[u8])> {
    let actions = find_key(doc, b"smartActions")?;
    let mut key = [0u8; 8];
    key[..3].copy_from_slice(b"SA_");
    let mut n = id;
    let mut digits = [0u8; 5];
    let mut nd = 0;
    loop {
        digits[nd] = b'0' + (n % 10) as u8;
        nd += 1;
        n /= 10;
        if n == 0 {
            break;
        }
    }
    for i in 0..nd {
        key[3 + i] = digits[nd - 1 - i];
    }
    let entry = find_key(actions, &key[..3 + nd])?;
    let kind = match find_key(entry, b"type").and_then(as_str)? {
        b"TEXT_STEP" => SmartKind::Text,
        b"CMD_STEP" => SmartKind::Command,
        b"APP_STEP" => SmartKind::App,
        b"URL_STEP" => SmartKind::Url,
        _ => return None,
    };
    let payload = find_key(entry, b"payload")?;
    Some((kind, payload))
}

/// The keymap a pad ships with: one profile, the ChatGPT layer, exactly as
/// the Input app's own template lays it out.
pub const DEFAULT_KEYMAP: &str = concat!(
    r#"{"version":1,"activeProfileId":0,"profiles":[{"id":0,"name":"Codex","layers":[{"id":0,"name":"ChatGPT","color":16711680,"os":0,"layout":{"keymap":[["KV_OAI_AG00","KV_OAI_AG01"],["KV_OAI_AG02","KV_OAI_AG03","KV_OAI_AG04","KV_OAI_AG05"],["KV_OAI_ACT06","KV_OAI_ACT07","KV_OAI_ACT08","KV_OAI_ACT09"],["KV_OAI_ACT10","KV_OAI_ACT11","KV_OAI_ACT12"]],"encoders":[["KV_OAI_ENC_CC","KV_OAI_ENC_CW","KV_OAI_ENC_CLK"]],"buttons":[["KC_MPLY"]],"joystick":{"type":"VENDOR","sectors":[]}}}],"macrosUsed":[],"multiActionsUsed":[]}],"multiActions":[],"macros":[],"macrosGroups":[],"multiActionsGroups":[],"linkedApps":[]}"#
);
