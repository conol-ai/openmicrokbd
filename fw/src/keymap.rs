//! The configurable keymap: what every input on the pad emits.
//!
//! The PRD's core architecture decision — replacing fixed F13..F24 — is that
//! each input position stores its emitted HID code *in firmware flash*,
//! written by the companion app over the vendor HID interface. The pad then
//! emits the same codes on any machine, app running or not, and the factory
//! defaults are chosen to be interceptable on every OS (macOS exposes no
//! virtual keycodes for F21..F24, so those never appear as defaults; when the
//! interceptable range runs out the defaults continue as Shift-qualified
//! F-codes, which the PRD explicitly endorses).
//!
//! 24 slots, fixed order (the app shares this table):
//!   0..=12  the 13 keys, in switch order sw1..sw13 — sw10/sw11 (the pair
//!           under the 2U keycap) are INDEPENDENT positions now
//!   13..=15 encoder: CW, CCW, press
//!   16..=20 joystick: up, down, left, right, press
//!   21..=23 touch pad: tap, swipe left, swipe right (swipes have no hardware
//!           detection on this single-zone pad; the slots exist so the config
//!           format already covers a future pad revision)
//!
//! Persistence: the last 2 KiB flash page (offset 0x1F800 of 128 KiB), erased
//! and rewritten only on the SAVE command. Neither `probe-rs download` nor a
//! DfuSe update touches it — both erase only the pages the image covers — so
//! a saved keymap survives firmware updates, exactly as the PRD requires.

use core::cell::RefCell;
use embassy_stm32::flash::{Blocking, Flash};
use embassy_sync::blocking_mutex::raw::ThreadModeRawMutex;
use embassy_sync::blocking_mutex::Mutex;

pub const SLOT_COUNT: usize = 24;
/// Slots 0..KEY_SLOTS are the physical keys; part of the wire contract even
/// where the firmware itself computes with literal positions.
#[allow(dead_code)]
pub const KEY_SLOTS: usize = 13;
pub const SLOT_ENC_CW: usize = 13;
pub const SLOT_ENC_CCW: usize = 14;
pub const SLOT_ENC_PRESS: usize = 15;
pub const SLOT_JOY_UP: usize = 16;
pub const SLOT_JOY_DOWN: usize = 17;
pub const SLOT_JOY_LEFT: usize = 18;
pub const SLOT_JOY_RIGHT: usize = 19;
pub const SLOT_JOY_PRESS: usize = 20;
pub const SLOT_TOUCH_TAP: usize = 21;

/// Slot kinds on the wire (and in flash).
pub const KIND_NONE: u8 = 0;
pub const KIND_KEYBOARD: u8 = 1;
pub const KIND_CONSUMER: u8 = 2;

/// Joystick modes on the wire (and in flash). In KEYS mode deflection holds
/// the direction slots like any key; in MOUSE mode deflection moves the HID
/// mouse pointer proportionally and the stick's push switch is left click
/// (the direction and press slots are ignored, but keep their contents so
/// switching back restores them).
pub const JOY_MODE_KEYS: u8 = 0;
pub const JOY_MODE_MOUSE: u8 = 1;

/// One emitted code: 4 bytes on the wire (kind, mods, code LE).
/// `mods` is the HID modifier bitmask (bit0 LCtrl, bit1 LShift, bit2 LAlt,
/// bit3 LGui); `code` a keyboard usage (KIND_KEYBOARD) or consumer usage
/// (KIND_CONSUMER).
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Slot {
    pub kind: u8,
    pub mods: u8,
    pub code: u16,
}

impl Slot {
    pub const fn none() -> Self {
        Slot {
            kind: KIND_NONE,
            mods: 0,
            code: 0,
        }
    }
    pub const fn key(code: u16) -> Self {
        Slot {
            kind: KIND_KEYBOARD,
            mods: 0,
            code,
        }
    }
    pub const fn shifted(code: u16) -> Self {
        Slot {
            kind: KIND_KEYBOARD,
            mods: 0x02, // left shift
            code,
        }
    }
    pub const fn consumer(code: u16) -> Self {
        Slot {
            kind: KIND_CONSUMER,
            mods: 0,
            code,
        }
    }
}

const KC_F13: u16 = 0x68;
const KC_ENTER: u16 = 0x28;
const KC_RIGHT: u16 = 0x4F;
const KC_LEFT: u16 = 0x50;
const KC_DOWN: u16 = 0x51;
const KC_UP: u16 = 0x52;
const USAGE_MUTE: u16 = 0xE2;
const USAGE_VOL_UP: u16 = 0xE9;
const USAGE_VOL_DOWN: u16 = 0xEA;
const USAGE_PLAY_PAUSE: u16 = 0xCD;

/// Factory defaults: keys p0..p7 = F13..F20 plain, p8..p12 = Shift+F13..F17
/// (all interceptable everywhere); analog inputs keep their media/arrow roles.
pub const DEFAULT_SLOTS: [Slot; SLOT_COUNT] = [
    Slot::key(KC_F13),     // p0
    Slot::key(KC_F13 + 1), // p1
    Slot::key(KC_F13 + 2), // p2
    Slot::key(KC_F13 + 3), // p3
    Slot::key(KC_F13 + 4), // p4
    Slot::key(KC_F13 + 5), // p5
    Slot::key(KC_F13 + 6), // p6
    Slot::key(KC_F13 + 7), // p7
    Slot::shifted(KC_F13), // p8
    Slot::shifted(KC_F13 + 1), // p9
    Slot::shifted(KC_F13 + 2), // p10 (2U pair, left switch)
    Slot::shifted(KC_F13 + 3), // p11 (2U pair, right switch)
    Slot::shifted(KC_F13 + 4), // p12
    Slot::consumer(USAGE_VOL_UP),   // encoder CW
    Slot::consumer(USAGE_VOL_DOWN), // encoder CCW
    Slot::consumer(USAGE_MUTE),     // encoder press
    Slot::key(KC_UP),               // joystick up
    Slot::key(KC_DOWN),             // joystick down
    Slot::key(KC_LEFT),             // joystick left
    Slot::key(KC_RIGHT),            // joystick right
    Slot::key(KC_ENTER),            // joystick press
    Slot::consumer(USAGE_PLAY_PAUSE), // touch tap
    Slot::none(),                     // touch swipe left  (no hw detection)
    Slot::none(),                     // touch swipe right (no hw detection)
];

pub const DEFAULT_JOY_THRESHOLD: u16 = 1024;
pub const DEFAULT_JOY_MODE: u8 = JOY_MODE_KEYS;
/// 1..=10; the adc task scales pointer motion by this.
pub const DEFAULT_JOY_MOUSE_SPEED: u8 = 5;
/// 0..=255 dims both LED chains; 255 = the hard power-budget cap.
pub const DEFAULT_LED_BRIGHTNESS: u8 = 255;

pub struct KeymapState {
    pub slots: [Slot; SLOT_COUNT],
    pub joy_threshold: u16,
    pub joy_mode: u8,
    pub joy_mouse_speed: u8,
    pub led_brightness: u8,
}

/// The live keymap. All tasks run in thread mode (no ISR touches this), so a
/// thread-mode blocking mutex is enough.
pub static KEYMAP: Mutex<ThreadModeRawMutex, RefCell<KeymapState>> =
    Mutex::new(RefCell::new(KeymapState {
        slots: DEFAULT_SLOTS,
        joy_threshold: DEFAULT_JOY_THRESHOLD,
        joy_mode: DEFAULT_JOY_MODE,
        joy_mouse_speed: DEFAULT_JOY_MOUSE_SPEED,
        led_brightness: DEFAULT_LED_BRIGHTNESS,
    }));

pub fn slot(i: usize) -> Slot {
    KEYMAP.lock(|k| k.borrow().slots[i])
}

pub fn joy_threshold() -> u16 {
    KEYMAP.lock(|k| k.borrow().joy_threshold)
}

pub fn joy_mode() -> u8 {
    KEYMAP.lock(|k| k.borrow().joy_mode)
}

pub fn joy_mouse_speed() -> u8 {
    KEYMAP.lock(|k| k.borrow().joy_mouse_speed)
}

pub fn led_brightness() -> u8 {
    KEYMAP.lock(|k| k.borrow().led_brightness)
}

// ---- flash persistence -----------------------------------------------------

/// Last 2 KiB page of the 128 KiB flash, as an offset from the flash base.
const CONFIG_OFFSET: u32 = 0x1F800;
const CONFIG_ADDR: u32 = 0x0800_0000 + CONFIG_OFFSET;
const MAGIC: u32 = 0x4F4D_4B31; // "OMK1"
/// v1: … slots, checksum(2), pad(2). v2 spent those pad bytes on the two
/// joystick-mode fields. v3 appends led_brightness + a reserved byte and
/// moves the checksum after them, growing the blob 108 -> 112 bytes — still
/// a multiple of the F0's 4-byte programming unit for both flash writes
/// (body, then magic). Older blobs still load, with the fields they predate
/// at their defaults.
const LAYOUT_VERSION: u16 = 3;
/// Where v1 put the checksum: right after the slots.
const V1_CK_OFFSET: usize = 8 + SLOT_COUNT * 4;
/// v2 field homes (the former v1 checksum + pad area).
const JOY_MODE_OFFSET: usize = V1_CK_OFFSET;
const JOY_SPEED_OFFSET: usize = V1_CK_OFFSET + 1;
const V2_CK_OFFSET: usize = V1_CK_OFFSET + 2;
/// v3 field homes (the former v2 checksum area) + one reserved byte.
const LED_BRIGHTNESS_OFFSET: usize = V2_CK_OFFSET;
const CK_OFFSET: usize = V2_CK_OFFSET + 2;
const BLOB_LEN: usize = CK_OFFSET + 4;

/// Wrapping byte sum over everything before the (version-dependent) checksum
/// field. Combined with magic-last write ordering this makes a torn SAVE
/// fail validation instead of loading garbage slots.
fn checksum(b: &[u8], ck_offset: usize) -> u16 {
    b[..ck_offset]
        .iter()
        .fold(0u16, |a, &x| a.wrapping_add(x as u16))
}

fn encode(state: &KeymapState) -> [u8; BLOB_LEN] {
    let mut b = [0u8; BLOB_LEN];
    b[0..4].copy_from_slice(&MAGIC.to_le_bytes());
    b[4..6].copy_from_slice(&LAYOUT_VERSION.to_le_bytes());
    b[6..8].copy_from_slice(&state.joy_threshold.to_le_bytes());
    for (i, s) in state.slots.iter().enumerate() {
        let o = 8 + i * 4;
        b[o] = s.kind;
        b[o + 1] = s.mods;
        b[o + 2..o + 4].copy_from_slice(&s.code.to_le_bytes());
    }
    b[JOY_MODE_OFFSET] = state.joy_mode;
    b[JOY_SPEED_OFFSET] = state.joy_mouse_speed;
    b[LED_BRIGHTNESS_OFFSET] = state.led_brightness;
    let ck = checksum(&b, CK_OFFSET);
    b[CK_OFFSET..CK_OFFSET + 2].copy_from_slice(&ck.to_le_bytes());
    b
}

/// Load the saved keymap into RAM, if a valid one exists. Reads the config
/// page memory-mapped — no flash driver needed for reads.
pub fn load_from_flash() -> bool {
    let mut b = [0u8; BLOB_LEN];
    for (i, byte) in b.iter_mut().enumerate() {
        // Safety: reads of valid, always-mapped flash addresses.
        *byte = unsafe { core::ptr::read_volatile((CONFIG_ADDR + i as u32) as *const u8) };
    }
    if u32::from_le_bytes([b[0], b[1], b[2], b[3]]) != MAGIC {
        return false;
    }
    let version = u16::from_le_bytes([b[4], b[5]]);
    let ck_offset = match version {
        1 => V1_CK_OFFSET,
        2 => V2_CK_OFFSET,
        3 => CK_OFFSET,
        _ => return false,
    };
    if u16::from_le_bytes([b[ck_offset], b[ck_offset + 1]]) != checksum(&b, ck_offset) {
        return false;
    }
    KEYMAP.lock(|k| {
        let mut k = k.borrow_mut();
        k.joy_threshold = u16::from_le_bytes([b[6], b[7]]).clamp(200, 1900);
        if version >= 2 {
            let mode = b[JOY_MODE_OFFSET];
            k.joy_mode = if mode <= JOY_MODE_MOUSE {
                mode
            } else {
                DEFAULT_JOY_MODE
            };
            k.joy_mouse_speed = b[JOY_SPEED_OFFSET].clamp(1, 10);
        } else {
            k.joy_mode = DEFAULT_JOY_MODE;
            k.joy_mouse_speed = DEFAULT_JOY_MOUSE_SPEED;
        }
        k.led_brightness = if version >= 3 {
            b[LED_BRIGHTNESS_OFFSET]
        } else {
            DEFAULT_LED_BRIGHTNESS
        };
        for i in 0..SLOT_COUNT {
            let o = 8 + i * 4;
            // A corrupt slot (bad kind, or a keyboard code outside u8 range,
            // which the report builder cannot express) degrades to disabled
            // rather than emitting garbage.
            let kind = if b[o] <= KIND_CONSUMER { b[o] } else { KIND_NONE };
            let code = u16::from_le_bytes([b[o + 2], b[o + 3]]);
            let kind = if kind == KIND_KEYBOARD && code > 0xFF {
                KIND_NONE
            } else {
                kind
            };
            k.slots[i] = Slot {
                kind,
                mods: b[o + 1],
                code,
            };
        }
    });
    true
}

/// Persist the RAM keymap: erase the config page and rewrite it. The CPU
/// stalls on flash operations (~25 ms erase) — USB rides it out on hardware
/// NAKs, the same trade the WS2812 bit-bang already makes, and this only runs
/// on an explicit SAVE command.
///
/// The 4-byte magic is written LAST, as its own operation: a save torn by
/// power loss leaves the header blank (erased flash reads 0xFF), so the next
/// boot fails the magic check and falls back to defaults instead of loading
/// half a keymap. The checksum covers the subtler torn case where power dies
/// between the body write and the magic write of a *re*-save.
pub fn save_to_flash(flash: &mut Flash<'_, Blocking>) -> Result<(), ()> {
    let blob = KEYMAP.lock(|k| encode(&k.borrow()));
    flash
        .blocking_erase(CONFIG_OFFSET, CONFIG_OFFSET + 2048)
        .map_err(|_| ())?;
    flash
        .blocking_write(CONFIG_OFFSET + 4, &blob[4..])
        .map_err(|_| ())?;
    flash
        .blocking_write(CONFIG_OFFSET, &blob[..4])
        .map_err(|_| ())
}

/// Factory reset: defaults in RAM, config page erased (so the next boot also
/// sees defaults).
pub fn factory_reset(flash: &mut Flash<'_, Blocking>) -> Result<(), ()> {
    KEYMAP.lock(|k| {
        let mut k = k.borrow_mut();
        k.slots = DEFAULT_SLOTS;
        k.joy_threshold = DEFAULT_JOY_THRESHOLD;
        k.joy_mode = DEFAULT_JOY_MODE;
        k.joy_mouse_speed = DEFAULT_JOY_MOUSE_SPEED;
        k.led_brightness = DEFAULT_LED_BRIGHTNESS;
    });
    flash
        .blocking_erase(CONFIG_OFFSET, CONFIG_OFFSET + 2048)
        .map_err(|_| ())
}

// ---- vendor-HID wire helpers ----------------------------------------------

/// Slots per GET/SET_KEYMAP page (7 × 4 bytes + 3-byte header ≤ 32).
pub const PAGE_SLOTS: usize = 7;
pub const PAGE_COUNT: usize = SLOT_COUNT.div_ceil(PAGE_SLOTS);

/// Fill `out` (a 32-byte reply body after [cmd, page]) with one page.
/// Returns the slot count in the page, or None for a bad page index.
pub fn read_page(page: usize, out: &mut [u8]) -> Option<usize> {
    if page >= PAGE_COUNT {
        return None;
    }
    let start = page * PAGE_SLOTS;
    let n = PAGE_SLOTS.min(SLOT_COUNT - start);
    KEYMAP.lock(|k| {
        let k = k.borrow();
        for i in 0..n {
            let s = k.slots[start + i];
            let o = i * 4;
            out[o] = s.kind;
            out[o + 1] = s.mods;
            out[o + 2..o + 4].copy_from_slice(&s.code.to_le_bytes());
        }
    });
    Some(n)
}

/// Apply one SET_KEYMAP page to RAM. Returns false for a malformed request —
/// validated in full BEFORE mutating, so a rejected page leaves RAM intact.
pub fn write_page(page: usize, count: usize, data: &[u8]) -> bool {
    let start = page * PAGE_SLOTS;
    if page >= PAGE_COUNT || count > PAGE_SLOTS || start + count > SLOT_COUNT || data.len() < count * 4
    {
        return false;
    }
    for i in 0..count {
        let o = i * 4;
        let code = u16::from_le_bytes([data[o + 2], data[o + 3]]);
        // Keyboard usages live in a u8 report field; codes past 0xFF would
        // silently alias to their low byte — reject instead.
        if data[o] == KIND_KEYBOARD && code > 0xFF {
            return false;
        }
    }
    KEYMAP.lock(|k| {
        let mut k = k.borrow_mut();
        for i in 0..count {
            let o = i * 4;
            let kind = if data[o] <= KIND_CONSUMER { data[o] } else { KIND_NONE };
            k.slots[start + i] = Slot {
                kind,
                mods: data[o + 1],
                code: u16::from_le_bytes([data[o + 2], data[o + 3]]),
            };
        }
    });
    true
}
