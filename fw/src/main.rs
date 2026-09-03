//! OpenMicro macropad firmware — STM32F072CBT6 + embassy.
//!
//! Pin map is the CoHDL design's (src/openmicro_parts.cohdl,
//! the position-aware GPIO assignment) — if the .cohdl changes, this table is
//! the one to update:
//!
//!   ROW0..3  = PA9  PA10 PB3  PB8   (outputs, drive high per scan step)
//!   COL0..3  = PB4  PB5  PC14 PC13  (inputs, pull-down; diode cathode -> COL)
//!   ENC_A/B  = PB12 PB13 (quadrature, pull-up, common to GND)
//!   ENC_SW   = PB15      (pull-up, active low)
//!   JOY_X/Y  = PB1/ADC_IN9  PA0/ADC_IN0 (stick mounted rotated: the JOY_X
//!              net senses vertical travel — adc_task swaps the channels)
//!   JOY_SW   = PA15      (pull-up, active low; DEAD on the 2026-08 fab —
//!              that revision's footprint paired the switch poles by column
//!              instead of by row, shorting the line to GND through the
//!              switch body; fixed in openmicro_parts.cohdl for the next spin)
//!   TOUCH    = PB9       (RC charge-time sensing, no external R)
//!   LED_KEY  = PA8       (13x SK6812MINI-E per-key chain)
//!   LED_UG   = PB14      (8x SK6812MINI-E perimeter underglow ring)
//!
//! COL2/COL3 sit on PC14/PC13, which are behind the VBAT power switch and can
//! only source a few mA. That is fine here and deliberate: in COL2ROW the
//! columns are only ever READ. Do not move an output onto them.
//!   USB      = PA11/PA12 (FS device; HSI48 + CRS, crystal not required)
//!
//! The `proto` Cargo feature selects the PROTOTYPE pin map instead — boards
//! fabbed before the 2026-07-28 GPIO re-derivation: rows PA9 PB3 PB6 PB5,
//! cols PB8 PB7 PA15 PA10, encoder PC13/PC14/PC15, joystick push PA8,
//! JOY_Y PB0/ADC_IN8, per-key chain PB4, 16-LED underglow ring on PA0.
//! Touch, USB, and SWD are identical on both revisions.
//!
//! HID map: every input's emitted code is CONFIGURABLE and stored in flash
//! (keymap.rs) — all 13 keys are independent positions (including the pair
//! under the 2U keycap). Factory defaults: keys F13..F20 + Shift+F13..F17
//! (interceptable on every OS), encoder -> volume/mute, touch -> play/pause,
//! joystick -> arrows/enter. The joystick alternatively runs in MOUSE mode
//! (keymap.rs joy_mode): a dedicated HID mouse interface carries
//! proportional pointer motion and the stick's push switch becomes left
//! click. GRADE mode rides the same interface with the speed applied
//! squared (sub-pixel creep at 1, brisk at 10) and the left button
//! auto-held while deflected — hover a DaVinci Resolve colour wheel and the
//! stick drags it like a panel trackball.
//!
//! A vendor-defined HID interface (usage page 0xFF60) carries the app
//! protocol: version query, DFU reboot, keymap read/write/save, analog
//! tuning, joystick mode, and unsolicited input-event reports (first byte
//! 0x80) that give the app live press feedback without any OS
//! input-monitoring permission.
//!
//! Device mode (keymap.rs `device_mode`, persisted with the keymap): the pad
//! normally boots as OpenMicro (1209:0001, the composite above). In the
//! opt-in **Codex Micro compat mode** (codex.rs) it boots with the Codex
//! Micro's USB identity plus a fifth HID interface speaking ChatGPT
//! Desktop's device protocol, and every input is routed there instead of
//! through the keymap (the vendor interface stays, so the app still works).
//! Switch by holding a key while plugging in — the first key of the second
//! row (slot 2, "KEY 03" in the app) → OpenMicro, the second key of that row
//! (slot 3, "KEY 04") → Codex — or with `SET_MODE` from the app. The choice is saved and applies to every later
//! boot; the underglow shows the mode's colour for a moment at power-up.

#![no_std]
#![no_main]

mod codex;
mod dfu;
mod keymap;
mod ws2812;

use codex::{Binding, Joystick};
use core::cell::RefCell;
use embassy_executor::Spawner;
use embassy_futures::join::join4;
use embassy_stm32::flash::Blocking;
use embassy_futures::select::{select, Either};
use embassy_stm32::adc::Adc;
use embassy_stm32::exti::ExtiInput;
use embassy_stm32::flash::Flash;
use embassy_stm32::gpio::{Flex, Input, Level, Output, Pull, Speed};
use embassy_stm32::rcc::{Hsi48Config, Sysclk};
use embassy_stm32::usb::Driver;
use embassy_stm32::{bind_interrupts, peripherals, usb, Config};
use embassy_sync::blocking_mutex::raw::ThreadModeRawMutex;
use embassy_sync::channel::Channel;
use embassy_time::{with_timeout, Duration, Instant, Ticker, Timer};
use embassy_usb::class::hid::{HidReaderWriter, HidWriter, State};
use embassy_usb::driver::{Driver as UsbDriver, EndpointError};
// Bring-up logging over the SWD probe (RTT) + panic messages on the same
// channel. `DEFMT_LOG=off` compiles every log statement out.
use defmt::{debug, info, warn};
use defmt_rtt as _;
use panic_probe as _;
use static_cell::StaticCell;
use usbd_hid::descriptor::{
    KeyboardReport, MediaKeyboardReport, MouseReport, SerializedDescriptor,
};

bind_interrupts!(struct Irqs {
    USB => usb::InterruptHandler<peripherals::USB>;
    ADC1_COMP => embassy_stm32::adc::InterruptHandler<peripherals::ADC1>;
});

// ---- board revision (see the pin-map tables in the module docs) ----
// JOY_Y is the one pin that crosses an ADC channel between revisions, so it
// is the one place the peripheral TYPE differs; everything else is erased
// into Output/Input/ExtiInput at construction.
#[cfg(not(feature = "proto"))]
type JoyYPin = peripherals::PA0;
#[cfg(feature = "proto")]
type JoyYPin = peripherals::PB0;

/// Underglow ring length: 2 per side on the current board, 4 per side on the
/// prototype. The hue step derives from this — count x step must stay 256.
#[cfg(not(feature = "proto"))]
const UG_LEN: usize = 8;
#[cfg(feature = "proto")]
const UG_LEN: usize = 16;

/// matrix position -> key slot index (-1 = no switch there). All 13 keys are
/// independent — the two switches under the 2U keycap are positions 10 and 11.
const POSITIONS: [[i8; 4]; 4] = [
    [-1, 0, 1, -1],   // R0: -,  p0,  p1,  -
    [2, 3, 4, 5],     // R1: p2..p5
    [6, 7, 8, 9],     // R2: p6..p9
    [-1, 10, 11, 12], // R3: -, p10, p11, p12
];

const FW_VERSION: &str = env!("CARGO_PKG_VERSION");

/// "a.b.c" -> USB bcdDevice (a in the high byte, b/c a nibble each), so the
/// updater can read the running version straight from the device descriptor.
const fn version_bcd(s: &str) -> u16 {
    let b = s.as_bytes();
    let mut parts = [0u16; 3];
    let mut pi = 0;
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'.' {
            pi += 1;
        } else if b[i].is_ascii_digit() && pi < 3 {
            parts[pi] = parts[pi] * 10 + (b[i] - b'0') as u16;
        }
        i += 1;
    }
    ((parts[0] & 0xFF) << 8) | ((parts[1] & 0xF) << 4) | (parts[2] & 0xF)
}

/// Vendor "raw HID" interface (QMK-style): usage page 0xFF60, 32-byte IN and
/// OUT reports, no report IDs. This is the updater channel.
#[rustfmt::skip]
const RAW_HID_DESC: &[u8] = &[
    0x06, 0x60, 0xFF, // Usage Page (Vendor 0xFF60)
    0x09, 0x61,       // Usage (0x61)
    0xA1, 0x01,       // Collection (Application)
    0x09, 0x62,       //   Usage (0x62)
    0x15, 0x00,       //   Logical Minimum (0)
    0x26, 0xFF, 0x00, //   Logical Maximum (255)
    0x75, 0x08,       //   Report Size (8)
    0x95, 0x20,       //   Report Count (32)
    0x81, 0x02,       //   Input (Data, Var, Abs)
    0x09, 0x63,       //   Usage (0x63)
    0x15, 0x00,       //   Logical Minimum (0)
    0x26, 0xFF, 0x00, //   Logical Maximum (255)
    0x75, 0x08,       //   Report Size (8)
    0x95, 0x20,       //   Report Count (32)
    0x91, 0x02,       //   Output (Data, Var, Abs)
    0xC0,             // End Collection
];

// App protocol (v2), one command per 32-byte OUT report; replies echo the
// command byte, unsolicited event reports start 0x80 (see EVENT_CH):
//   [0x01, ...]                    -> [0x01, len, version ascii...]
//   [0x02, 'D','F','U','!']        -> [0x02, 0x01], reboot into ROM DFU
//   [0x03, page]                   -> [0x03, page, count, count*4 slot bytes]
//   [0x04, page, count, slots...]  -> [0x04, ok] (applies to RAM immediately)
//   [0x05, 'S','A','V','E']        -> [0x05, ok] (persist keymap to flash)
//   [0x06, 'R','S','T','!']        -> [0x06, ok] (factory defaults, flash wiped)
//   [0x07]                         -> [0x07, thr_lo, thr_hi] (joystick threshold)
//   [0x08, thr_lo, thr_hi]         -> [0x08, 0x01] (RAM only; SAVE persists)
//   [0x09]                         -> [0x09, mode, speed] (joystick mode 0 keys /
//                                     1 mouse / 2 grade, pointer speed 1..=10)
//   [0x0A, mode, speed]            -> [0x0A, 0x01] (RAM only; SAVE persists)
//   [0x0B]                         -> [0x0B, brightness] (LED brightness 0..=255)
//   [0x0C, brightness]             -> [0x0C, 0x01] (RAM only, applied within one
//                                     LED frame; SAVE persists)
//   [0x0D]                         -> [0x0D, kmode,kr,kg,kb, umode,ur,ug,ub]
//                                     (per-chain pattern: 0 rainbow, 1 solid RGB)
//   [0x0E, kmode,kr,kg,kb, umode,ur,ug,ub] -> [0x0E, 0x01] (RAM; SAVE persists)
//   [0x0F, index, enabled, r,g,b] -> [0x0F, 0x01] (RAM-only per-key override)
//   [0x10]                         -> [0x10, mode] (running device mode:
//                                     0 OpenMicro / 1 Codex Micro compat)
//   [0x11, mode, 'M','O','D','E']  -> [0x11, ok]; a CHANGED mode is persisted
//                                     and the pad resets to re-enumerate in it
const CMD_VERSION: u8 = 0x01;
const CMD_ENTER_DFU: u8 = 0x02;
const CMD_GET_KEYMAP: u8 = 0x03;
const CMD_SET_KEYMAP: u8 = 0x04;
const CMD_SAVE: u8 = 0x05;
const CMD_FACTORY_RESET: u8 = 0x06;
const CMD_GET_ANALOG: u8 = 0x07;
const CMD_SET_ANALOG: u8 = 0x08;
const CMD_GET_JOYMODE: u8 = 0x09;
const CMD_SET_JOYMODE: u8 = 0x0A;
const CMD_GET_LED: u8 = 0x0B;
const CMD_SET_LED: u8 = 0x0C;
const CMD_GET_LEDPATTERN: u8 = 0x0D;
const CMD_SET_LEDPATTERN: u8 = 0x0E;
const CMD_SET_KEY_LED_OVERRIDE: u8 = 0x0F;
const CMD_GET_MODE: u8 = 0x10;
const CMD_SET_MODE: u8 = 0x11;
const ENTER_DFU_KEY: &[u8; 4] = b"DFU!";
const SAVE_KEY: &[u8; 4] = b"SAVE";
const RESET_KEY: &[u8; 4] = b"RST!";
const MODE_KEY: &[u8; 4] = b"MODE";
const EVENT_REPORT: u8 = 0x80;

/// Boot identity (keymap::MODE_*), fixed for this power cycle: the USB
/// descriptors are built from it once, so a change only lands after a reset.
static DEVICE_MODE: core::sync::atomic::AtomicU8 =
    core::sync::atomic::AtomicU8::new(keymap::MODE_OPENMICRO);

fn codex_mode() -> bool {
    DEVICE_MODE.load(core::sync::atomic::Ordering::Relaxed) == keymap::MODE_CODEX
}

/// Boot splash on the underglow ring: the mode's colour, solid for a moment
/// after every boot and blinking when a boot chord has just changed it.
static SPLASH_UNTIL_MS: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
static SPLASH_RGB: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
static SPLASH_BLINK: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);
/// OpenMicro mode shows the app's accent amber; Codex mode plain white.
const MODE_RGB_OPENMICRO: u32 = 0xF5AE58;
const MODE_RGB_CODEX: u32 = 0xFFFFFF;

fn splash(rgb: u32, blink: bool) {
    use core::sync::atomic::Ordering::Relaxed;
    let now = Instant::now().as_millis() as u32;
    SPLASH_RGB.store(rgb, Relaxed);
    SPLASH_BLINK.store(blink, Relaxed);
    SPLASH_UNTIL_MS.store(now.wrapping_add(if blink { 1800 } else { 1000 }), Relaxed);
}

/// Boot chord: a key held while power arrives picks the device mode for
/// this and every later boot. Row 1 is driven like a scan step and the two
/// columns sampled 20 times over 100 ms, so a bounce or a glancing touch
/// during plug-in cannot flip the mode; both keys down is ambiguous and
/// ignored. Slot 2 (the app's KEY 03) = row 1 / col 0, slot 3 (KEY 04) =
/// row 1 / col 1 (POSITIONS).
async fn boot_chord(rows: &mut [Output<'static>; 4], cols: &[Input<'static>; 4]) -> Option<u8> {
    const SAMPLES: u32 = 20;
    let mut key2 = 0u32;
    let mut key3 = 0u32;
    rows[1].set_high();
    for _ in 0..SAMPLES {
        Timer::after_millis(5).await;
        key2 += cols[0].is_high() as u32;
        key3 += cols[1].is_high() as u32;
    }
    rows[1].set_low();
    match (key2 == SAMPLES, key3 == SAMPLES) {
        (true, false) => Some(keymap::MODE_OPENMICRO),
        (false, true) => Some(keymap::MODE_CODEX),
        _ => None,
    }
}

static KEY_LED_OVERRIDE_MASK: portable_atomic::AtomicU16 =
    portable_atomic::AtomicU16::new(0);
static KEY_LED_OVERRIDE_RGB: [portable_atomic::AtomicU32; 13] =
    [const { portable_atomic::AtomicU32::new(0) }; 13];

#[derive(Clone, Copy)]
struct KeyboardTransition {
    first: KeyboardReport,
    second: Option<KeyboardReport>,
}

impl KeyboardTransition {
    fn single(report: KeyboardReport) -> Self {
        Self {
            first: report,
            second: None,
        }
    }

    fn pair(first: KeyboardReport, second: KeyboardReport) -> Self {
        Self {
            first,
            second: Some(second),
        }
    }
}

// A modifier-qualified key transition may need two reports (modifiers first,
// then the key; the reverse on release). Queue the transition as one item so
// overload can never split that ordering guarantee.
static KBD_CH: Channel<ThreadModeRawMutex, KeyboardTransition, 16> = Channel::new();
static CONSUMER_CH: Channel<ThreadModeRawMutex, MediaKeyboardReport, 8> = Channel::new();

/// One HID mouse frame (mouse/grade-mode joystick). Every frame carries the FULL
/// button state, so newest-wins eviction on overload can never strand a
/// click: whatever report goes out last is the truth.
#[derive(Clone, Copy)]
struct MouseFrame {
    buttons: u8,
    dx: i8,
    dy: i8,
}
static MOUSE_CH: Channel<ThreadModeRawMutex, MouseFrame, 8> = Channel::new();
/// Current mouse button bitmask (bit 0 = left, from the joystick push
/// switch). Written by scan_task, folded into motion frames by adc_task.
static MOUSE_BUTTONS: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);
/// Grade-mode auto-drag: set by adc_task while the stick is deflected.
/// OR-ed into every frame's bit 0 alongside the push switch, so a push
/// release mid-deflection cannot lift the hold a colour-wheel drag depends
/// on (and vice versa).
static GRADE_DRAG: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);

/// The full button truth for an outgoing frame: push switch OR auto-drag.
fn mouse_buttons() -> u8 {
    MOUSE_BUTTONS.load(core::sync::atomic::Ordering::Relaxed)
        | GRADE_DRAG.load(core::sync::atomic::Ordering::Relaxed) as u8
}

fn force_send_mouse(frame: MouseFrame) {
    if MOUSE_CH.try_send(frame).is_err() {
        let _ = MOUSE_CH.try_receive();
        let _ = MOUSE_CH.try_send(frame);
    }
}
/// Bit i = key position i pressed — drives the per-key LED effect.
static KEYSTATE: core::sync::atomic::AtomicU16 = core::sync::atomic::AtomicU16::new(0);

/// Live input events for the app ([src, a, b] -> report [0x80, src, a, b]):
/// src 0 = key (a = position, b = pressed), 1 = encoder rotate (a = 1 CW),
/// 2 = encoder button, 3 = joystick (a = dir 0..4, b = active), 4 = touch tap.
/// Dropped when full — press feedback is best-effort by design.
static EVENT_CH: Channel<ThreadModeRawMutex, [u8; 3], 16> = Channel::new();

fn post_event(src: u8, a: u8, b: u8) {
    let _ = EVENT_CH.try_send([src, a, b]);
}

/// Extra held-key entries for macro playback in Codex mode (a macro can
/// hold a few keys at once, e.g. Cmd+Shift+key).
const MACRO_SLOTS: usize = 4;
const HELD_LEN: usize = keymap::SLOT_COUNT + MACRO_SLOTS;

/// Which slots are currently held (matrix keys by position, plus the button
/// and joystick-direction slots, plus the macro entries), each with the Slot
/// SNAPSHOT taken at press time. One shared set so the keyboard report is
/// always rebuilt from the WHOLE truth — a joystick move can no longer drop
/// a held key from the host's point of view. The snapshot matters: the app
/// can rewrite the keymap mid-hold (profile switch), and a release must
/// retract exactly what its press emitted, not whatever the slot means now.
static HELD: embassy_sync::blocking_mutex::Mutex<
    ThreadModeRawMutex,
    core::cell::RefCell<[Option<keymap::Slot>; HELD_LEN]>,
> = embassy_sync::blocking_mutex::Mutex::new(core::cell::RefCell::new([None; HELD_LEN]));

/// try_send that never drops the NEWEST complete transition: on a full
/// channel the oldest transition is evicted first. Every transition ends in
/// the full current keyboard state, and a consumer report replaces the
/// previous usage, so newest-wins avoids stranded releases. Single executor,
/// no await between evict and re-send: this cannot interleave.
fn force_send_kbd(transition: KeyboardTransition) {
    if KBD_CH.try_send(transition).is_err() {
        let _ = KBD_CH.try_receive();
        let _ = KBD_CH.try_send(transition);
    }
}

fn force_send_consumer(usage: u16) {
    let report = MediaKeyboardReport { usage_id: usage };
    if CONSUMER_CH.try_send(report).is_err() {
        let _ = CONSUMER_CH.try_receive();
        let _ = CONSUMER_CH.try_send(report);
    }
}

fn empty_keyboard_report() -> KeyboardReport {
    KeyboardReport {
        modifier: 0,
        reserved: 0,
        leds: 0,
        keycodes: [0; 6],
    }
}

fn keyboard_report(held: &[Option<keymap::Slot>; HELD_LEN]) -> KeyboardReport {
    let mut report = empty_keyboard_report();
    let mut n = 0;
    for slot in held.iter().flatten() {
        if slot.kind != keymap::KIND_KEYBOARD {
            continue;
        }
        report.modifier |= slot.mods;
        // Codes above u8 range never enter RAM (write_page rejects them);
        // the cast is exact.
        let code = slot.code as u8;
        if code != 0 && n < 6 && !report.keycodes[..n].contains(&code) {
            report.keycodes[n] = code;
            n += 1;
        }
    }
    report
}

fn same_keyboard_report(a: &KeyboardReport, b: &KeyboardReport) -> bool {
    a.modifier == b.modifier && a.keycodes == b.keycodes
}

fn has_added_key(before: &KeyboardReport, after: &KeyboardReport) -> bool {
    after
        .keycodes
        .iter()
        .copied()
        .filter(|code| *code != 0)
        .any(|code| !before.keycodes.contains(&code))
}

fn has_removed_key(before: &KeyboardReport, after: &KeyboardReport) -> bool {
    before
        .keycodes
        .iter()
        .copied()
        .filter(|code| *code != 0)
        .any(|code| !after.keycodes.contains(&code))
}

fn retained_keycodes(before: &KeyboardReport, after: &KeyboardReport) -> [u8; 6] {
    let mut retained = [0; 6];
    let mut n = 0;
    for code in before.keycodes.iter().copied().filter(|code| *code != 0) {
        if after.keycodes.contains(&code) {
            retained[n] = code;
            n += 1;
        }
    }
    retained
}

fn keyboard_transition(
    before: KeyboardReport,
    after: KeyboardReport,
    pressed: bool,
) -> Option<KeyboardTransition> {
    if same_keyboard_report(&before, &after) {
        return None;
    }

    let intermediate = if before.modifier != after.modifier {
        if pressed && has_added_key(&before, &after) {
            Some(KeyboardReport {
                modifier: after.modifier,
                reserved: 0,
                leds: 0,
                keycodes: before.keycodes,
            })
        } else if !pressed && has_removed_key(&before, &after) {
            Some(KeyboardReport {
                modifier: before.modifier,
                reserved: 0,
                leds: 0,
                // At the 6KRO boundary, releasing one key can expose another
                // that was previously truncated. Keep only keys visible in
                // both states here so that newly exposed key is never pressed
                // under the modifiers that are being retired.
                keycodes: retained_keycodes(&before, &after),
            })
        } else {
            None
        }
    } else {
        None
    };

    match intermediate {
        Some(first)
            if !same_keyboard_report(&before, &first) && !same_keyboard_report(&first, &after) =>
        {
            Some(KeyboardTransition::pair(first, after))
        }
        _ => Some(KeyboardTransition::single(after)),
    }
}

/// Queue modifier-qualified chords in the same order as a physical keyboard:
/// modifier flags settle one USB frame before key-down, and key-up settles one
/// frame before the modifier flags clear. macOS normally accepts an atomic
/// report, but Carbon/Electron global shortcuts can miss a chord when its
/// modifiers and key first appear in the very same input report.
fn send_keyboard_transition(before: KeyboardReport, after: KeyboardReport, pressed: bool) {
    if let Some(transition) = keyboard_transition(before, after, pressed) {
        force_send_kbd(transition);
    }
}

async fn write_keyboard_transition<'d, D: UsbDriver<'d>, const N: usize>(
    writer: &mut HidWriter<'d, D, N>,
    transition: &KeyboardTransition,
) -> Result<(), EndpointError> {
    writer.write_serialize(&transition.first).await?;
    if let Some(second) = transition.second.as_ref() {
        writer.write_serialize(second).await?;
    }
    Ok(())
}

/// Mark a slot held/released and emit the consequences: keyboard-kind slots
/// rebuild the composite 6KRO report (modifiers OR-ed across every held slot —
/// the documented impurity of modifier-qualified codes); consumer-kind slots
/// send their usage on press, and on release fall back to another still-held
/// consumer slot's usage (or 0) so overlapping holds don't strand each other.
fn set_held(slot_idx: usize, held: bool) {
    apply_slot(slot_idx, held.then(|| keymap::slot(slot_idx)));
}

/// Press (`Some(slot)`) or release (`None`) held entry `slot_idx` with an
/// explicit Slot — the keymap's own for `set_held`, a Work Louder binding
/// in Codex mode.
fn apply_slot(slot_idx: usize, press: Option<keymap::Slot>) {
    // Press dispatches on the given meaning; release dispatches on the
    // snapshot stored at press time.
    let held = press.is_some();
    let (changed, s, before, after) = HELD.lock(|h| {
        let mut h = h.borrow_mut();
        let before = keyboard_report(&h);
        if let Some(s) = press {
            let changed = h[slot_idx].is_none();
            h[slot_idx] = Some(s);
            let after = keyboard_report(&h);
            (changed, s, before, after)
        } else {
            match h[slot_idx].take() {
                Some(s) => {
                    let after = keyboard_report(&h);
                    (true, s, before, after)
                }
                None => (false, keymap::Slot::none(), before, before),
            }
        }
    });
    if !changed {
        return;
    }
    match s.kind {
        keymap::KIND_CONSUMER => {
            let usage = if held {
                s.code
            } else {
                // Re-assert any other consumer slot still held.
                HELD.lock(|h| {
                    h.borrow()
                        .iter()
                        .flatten()
                        .find(|o| o.kind == keymap::KIND_CONSUMER)
                        .map(|o| o.code)
                        .unwrap_or(0)
                })
            };
            force_send_consumer(usage);
        }
        _ => {
            // KIND_NONE falls through here too: rebuilding the keyboard
            // report from the held snapshots is cheap and always correct.
            send_keyboard_transition(before, after, held);
        }
    }
}

/// Momentary tap of a slot (encoder detents, touch tap): press then release.
fn tap_slot(slot_idx: usize) {
    set_held(slot_idx, true);
    set_held(slot_idx, false);
}

/// Codex mode: perform a Work Louder binding for a control going down or
/// up. `slot` is the held-set entry the control owns, so a held key,
/// modifier or consumer usage is retracted exactly when it releases.
fn act(binding: Binding, pressed: bool, slot: usize) {
    match binding {
        Binding::None | Binding::Unsupported | Binding::Function => {}
        Binding::Oai(control) => codex::oai(control, pressed),
        Binding::Key { mods, code } => apply_slot(
            slot,
            pressed.then_some(keymap::Slot {
                kind: keymap::KIND_KEYBOARD,
                mods,
                code: code as u16,
            }),
        ),
        Binding::Consumer(usage) => apply_slot(slot, pressed.then_some(keymap::Slot::consumer(usage))),
        Binding::LayerToggle(n) => {
            if pressed {
                codex::toggle_layer(n);
            }
        }
        Binding::LayerHold(n) => codex::hold_layer(n, pressed),
        Binding::Profile(n) => {
            if pressed {
                codex::set_profile(n);
            }
        }
        Binding::Macro(id) => {
            if pressed {
                codex::run_macro(id);
            }
        }
        Binding::Multi(id) => act(codex::multi_tap(id), pressed, slot),
        Binding::Smart(id) => {
            if pressed {
                codex::post(codex::Event::Smart(id));
            }
        }
        Binding::CheatSheet(mode) => {
            let st = codex::status();
            let mode = match (mode, pressed) {
                (3, true) => 1,
                (3, false) => 0,
                (m, true) => m,
                (_, false) => return,
            };
            codex::post(codex::Event::CheatSheet {
                mode,
                layer: st.layer_index,
                profile: st.profile_index,
            });
        }
        Binding::Backlight(delta) => {
            if pressed {
                keymap::KEYMAP.lock(|k| {
                    let mut k = k.borrow_mut();
                    k.led_brightness = if delta > 0 {
                        k.led_brightness.saturating_add(32)
                    } else {
                        k.led_brightness.saturating_sub(32).max(16)
                    };
                });
            }
        }
    }
}

/// Macro playback's view of the keyboard: keys and modifiers occupy the
/// macro held-set entries, everything else goes through `act`.
struct MacroKeys;

static MACRO_KEYS: MacroKeys = MacroKeys;

impl codex::MacroKeys for MacroKeys {
    fn press(&self, b: Binding) {
        match b {
            Binding::Key { mods, code } => {
                let slot = HELD.lock(|h| {
                    let h = h.borrow();
                    (keymap::SLOT_COUNT..HELD_LEN).find(|&i| h[i].is_none())
                });
                if let Some(slot) = slot {
                    act(b, true, slot);
                } else {
                    warn!("codex: macro holds too many keys (mods {=u8} code {=u8})", mods, code);
                }
            }
            other => act(other, true, keymap::SLOT_COUNT),
        }
    }

    fn release(&self, b: Binding) {
        match b {
            Binding::Key { mods, code } => {
                let slot = HELD.lock(|h| {
                    let h = h.borrow();
                    (keymap::SLOT_COUNT..HELD_LEN).find(|&i| {
                        matches!(h[i], Some(s) if s.kind == keymap::KIND_KEYBOARD && s.mods == mods && s.code == code as u16)
                    })
                });
                if let Some(slot) = slot {
                    act(b, false, slot);
                }
            }
            other => act(other, false, keymap::SLOT_COUNT),
        }
    }
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    // First thing, before any clock/peripheral init: divert into the ROM DFU
    // bootloader if the previous run armed it (dfu.rs).
    dfu::check_and_enter();

    let mut config = Config::default();
    // USB clocking: HSI48 trimmed by CRS from USB SOF. The 8 MHz HSE crystal
    // is fitted (belt-and-braces) but not required for USB.
    config.rcc.hsi48 = Some(Hsi48Config {
        sync_from_usb: true,
    });
    // 48 MHz core from the same HSI48 (the WS2812 bit-bang cycle counts and
    // the USB peripheral both assume it); USBSW defaults to HSI48 on F0.
    config.rcc.sys = Sysclk::HSI48;
    let p = embassy_stm32::init(config);
    info!(
        "OpenMicro fw v{}: clocks up (HSI48 -> 48 MHz core, CRS synced from USB SOF)",
        FW_VERSION
    );

    // The configurable keymap: saved copy from the last flash page if one
    // exists, factory defaults otherwise. Loaded before any task can emit.
    // The flash driver is shared by the keymap page and, in Codex mode, the
    // Work Louder file slots; every use is a short synchronous borrow.
    static FLASH: StaticCell<RefCell<Flash<'static, Blocking>>> = StaticCell::new();
    let flash: &'static RefCell<Flash<'static, Blocking>> =
        FLASH.init(RefCell::new(Flash::new_blocking(p.FLASH)));
    if keymap::load_from_flash() {
        info!("keymap: loaded saved configuration from flash");
    } else {
        info!("keymap: no saved configuration — factory defaults");
    }

    // ---- matrix pins ---- (built before USB: the boot chord scans them)
    #[cfg(not(feature = "proto"))]
    let mut rows = [
        Output::new(p.PA9, Level::Low, Speed::Low),
        Output::new(p.PA10, Level::Low, Speed::Low),
        Output::new(p.PB3, Level::Low, Speed::Low),
        Output::new(p.PB8, Level::Low, Speed::Low),
    ];
    #[cfg(feature = "proto")]
    let mut rows = [
        Output::new(p.PA9, Level::Low, Speed::Low),
        Output::new(p.PB3, Level::Low, Speed::Low),
        Output::new(p.PB6, Level::Low, Speed::Low),
        Output::new(p.PB5, Level::Low, Speed::Low),
    ];
    #[cfg(not(feature = "proto"))]
    let cols = [
        Input::new(p.PB4, Pull::Down),
        Input::new(p.PB5, Pull::Down),
        Input::new(p.PC14, Pull::Down),
        Input::new(p.PC13, Pull::Down),
    ];
    #[cfg(feature = "proto")]
    let cols = [
        Input::new(p.PB8, Pull::Down),
        Input::new(p.PB7, Pull::Down),
        Input::new(p.PA15, Pull::Down),
        Input::new(p.PA10, Pull::Down),
    ];

    // ---- device mode: the saved one, unless a boot chord changes it ----
    // Persisted only on an actual change (flash endurance), before the USB
    // identity is chosen from it below.
    let saved_mode = keymap::device_mode();
    let chord = boot_chord(&mut rows, &cols).await;
    let changed = matches!(chord, Some(m) if m != saved_mode);
    let mode = match chord {
        Some(m) if m != saved_mode => {
            keymap::set_device_mode(m);
            match keymap::save_to_flash(&mut flash.borrow_mut()) {
                Ok(()) => info!("mode: boot chord -> {=u8}, saved", m),
                Err(()) => warn!("mode: boot chord -> {=u8}, FLASH ERROR (this boot only)", m),
            }
            m
        }
        Some(m) => {
            info!("mode: boot chord confirms {=u8}", m);
            m
        }
        None => saved_mode,
    };
    DEVICE_MODE.store(mode, core::sync::atomic::Ordering::Relaxed);
    let codex_mode = mode == keymap::MODE_CODEX;
    info!(
        "device mode: {}",
        if codex_mode { "Codex Micro compat" } else { "OpenMicro" }
    );
    splash(
        if codex_mode { MODE_RGB_CODEX } else { MODE_RGB_OPENMICRO },
        changed,
    );
    if codex_mode {
        // What the keys do in this mode comes from the Work Louder keymap
        // file (or its built-in default); macros play on their own task.
        codex::reload();
        spawner.must_spawn(codex::macro_task(&MACRO_KEYS));
    }

    // ---- USB HID: a boot keyboard + a consumer-control interface ----
    // (+ mouse + the vendor/app interface; in Codex Micro compat mode the
    // whole device borrows that identity and gains the Codex interface.)
    let driver = Driver::new(p.USB, Irqs, p.PA12, p.PA11);
    let mut usb_config = if codex_mode {
        embassy_usb::Config::new(codex::VID, codex::PID)
    } else {
        embassy_usb::Config::new(0x1209, 0x0001)
    };
    usb_config.manufacturer = Some(if codex_mode { codex::MANUFACTURER } else { "conol" });
    usb_config.product = Some(if codex_mode { codex::PRODUCT } else { "OpenMicro" });
    // Every unit reports its own serial: the MCU's factory-programmed
    // 96-bit unique ID, hex-encoded. Distinguishes pads when several are
    // plugged in, and gives support/logs a stable per-unit identity.
    usb_config.serial_number = Some(embassy_stm32::uid::uid_hex());
    usb_config.device_release = if codex_mode {
        codex::DEVICE_RELEASE
    } else {
        version_bcd(FW_VERSION)
    };

    static CONFIG_DESC: StaticCell<[u8; 256]> = StaticCell::new();
    static BOS_DESC: StaticCell<[u8; 256]> = StaticCell::new();
    // Must hold a whole control-OUT data stage: a 64-byte SET_REPORT on the
    // Codex interface, with headroom.
    static CONTROL_BUF: StaticCell<[u8; 128]> = StaticCell::new();
    static CODEX_STATE: StaticCell<State> = StaticCell::new();
    static KBD_STATE: StaticCell<State> = StaticCell::new();
    static CONSUMER_STATE: StaticCell<State> = StaticCell::new();
    static MOUSE_STATE: StaticCell<State> = StaticCell::new();
    static RAW_STATE: StaticCell<State> = StaticCell::new();

    let mut builder = embassy_usb::Builder::new(
        driver,
        usb_config,
        CONFIG_DESC.init([0; 256]),
        BOS_DESC.init([0; 256]),
        &mut [],
        CONTROL_BUF.init([0; 128]),
    );

    // Compat mode: the Codex interface goes FIRST (interface 0), so a host
    // that takes "the first HID interface with this VID/PID" lands on it.
    // Control SET_REPORTs need the request handler; without one embassy
    // STALLs them.
    let codex_hid = if codex_mode {
        Some(
            HidReaderWriter::<_, { codex::REPORT_LEN }, { codex::REPORT_LEN }>::new(
                &mut builder,
                CODEX_STATE.init(State::new()),
                embassy_usb::class::hid::Config {
                    report_descriptor: codex::REPORT_DESC,
                    request_handler: Some(codex::CONTROL.init(codex::ControlHandler)),
                    poll_ms: 1,
                    max_packet_size: codex::REPORT_LEN as u16,
                },
            ),
        )
    } else {
        None
    };

    let kbd_hid = HidReaderWriter::<_, 1, 8>::new(
        &mut builder,
        KBD_STATE.init(State::new()),
        embassy_usb::class::hid::Config {
            report_descriptor: KeyboardReport::desc(),
            request_handler: None,
            poll_ms: 1,
            max_packet_size: 8,
        },
    );
    let consumer_hid = HidReaderWriter::<_, 1, 8>::new(
        &mut builder,
        CONSUMER_STATE.init(State::new()),
        embassy_usb::class::hid::Config {
            report_descriptor: MediaKeyboardReport::desc(),
            request_handler: None,
            poll_ms: 8,
            max_packet_size: 8,
        },
    );

    // IN-only: the pointer interface for the joystick's mouse mode.
    let mut mouse_writer = HidWriter::<_, 8>::new(
        &mut builder,
        MOUSE_STATE.init(State::new()),
        embassy_usb::class::hid::Config {
            report_descriptor: MouseReport::desc(),
            request_handler: None,
            poll_ms: 4,
            max_packet_size: 8,
        },
    );

    let raw_hid = HidReaderWriter::<_, 32, 32>::new(
        &mut builder,
        RAW_STATE.init(State::new()),
        embassy_usb::class::hid::Config {
            report_descriptor: RAW_HID_DESC,
            request_handler: None,
            poll_ms: 10,
            max_packet_size: 32,
        },
    );

    let mut usb_dev = builder.build();
    let (_kbd_reader, mut kbd_writer) = kbd_hid.split();
    let (_consumer_reader, mut consumer_writer) = consumer_hid.split();
    let (mut raw_reader, mut raw_writer) = raw_hid.split();
    let codex_parts = codex_hid.map(|h| h.split());

    // ---- encoder + buttons ----
    // A/B are EXTI-driven rather than polled: at speed a 1 kHz scan aliases
    // transitions away, and the LED bit-bang blanks interrupts for ~870 us
    // every 33 ms. EXTI latches its pending bit, so an edge landing in that
    // window is still serviced once interrupts return.
    #[cfg(not(feature = "proto"))]
    let (enc_a, enc_b, enc_sw, joy_sw) = (
        ExtiInput::new(p.PB12, p.EXTI12, Pull::Up),
        ExtiInput::new(p.PB13, p.EXTI13, Pull::Up),
        Input::new(p.PB15, Pull::Up),
        Input::new(p.PA15, Pull::Up),
    );
    #[cfg(feature = "proto")]
    let (enc_a, enc_b, enc_sw, joy_sw) = (
        ExtiInput::new(p.PC13, p.EXTI13, Pull::Up),
        ExtiInput::new(p.PC14, p.EXTI14, Pull::Up),
        Input::new(p.PC15, Pull::Up),
        Input::new(p.PA8, Pull::Up),
    );

    // ---- joystick ADC ----
    let adc = Adc::new(p.ADC1, Irqs);
    let joy_x = p.PB1;
    #[cfg(not(feature = "proto"))]
    let joy_y = p.PA0;
    #[cfg(feature = "proto")]
    let joy_y = p.PB0;

    // ---- touch (RC charge-time on PB9) ----
    let touch = Flex::new(p.PB9);

    // ---- LED chains ----
    #[cfg(not(feature = "proto"))]
    let (led_key, led_ug) = (
        Output::new(p.PA8, Level::Low, Speed::VeryHigh),
        Output::new(p.PB14, Level::Low, Speed::VeryHigh),
    );
    #[cfg(feature = "proto")]
    let (led_key, led_ug) = (
        Output::new(p.PB4, Level::Low, Speed::VeryHigh),
        Output::new(p.PA0, Level::Low, Speed::VeryHigh),
    );

    spawner.must_spawn(scan_task(rows, cols, enc_sw, joy_sw));
    spawner.must_spawn(encoder_task(enc_a, enc_b));
    spawner.must_spawn(adc_task(adc, joy_x, joy_y));
    spawner.must_spawn(touch_task(touch));
    spawner.must_spawn(led_task(led_key, led_ug));
    info!("tasks spawned (scan/encoder/adc/touch/led); starting USB device");

    // USB device + report pumps + updater channel run forever on this task.
    let usb_fut = usb_dev.run();
    let pump = async {
        let mut keyboard_needs_resync = false;
        loop {
            if keyboard_needs_resync {
                // A disabled endpoint means the host reset or unplugged us.
                // Once it returns, discard stale transitions and reassert the
                // current held state from a clean host-side baseline.
                kbd_writer.ready().await;
                while KBD_CH.try_receive().is_ok() {}
                let current = HELD.lock(|h| keyboard_report(&h.borrow()));
                let transition = keyboard_transition(empty_keyboard_report(), current, true)
                    .unwrap_or_else(|| KeyboardTransition::single(current));
                match write_keyboard_transition(&mut kbd_writer, &transition).await {
                    Ok(()) => keyboard_needs_resync = false,
                    Err(EndpointError::Disabled) => continue,
                    Err(EndpointError::BufferOverflow) => {
                        warn!("keyboard HID report overflow");
                        keyboard_needs_resync = false;
                    }
                }
                continue;
            }

            match embassy_futures::select::select3(
                KBD_CH.receive(),
                CONSUMER_CH.receive(),
                MOUSE_CH.receive(),
            )
            .await
            {
                embassy_futures::select::Either3::First(transition) => {
                    match write_keyboard_transition(&mut kbd_writer, &transition).await {
                        Ok(()) => {}
                        Err(EndpointError::Disabled) => keyboard_needs_resync = true,
                        Err(EndpointError::BufferOverflow) => {
                            warn!("keyboard HID report overflow");
                        }
                    }
                }
                embassy_futures::select::Either3::Second(report) => {
                    let _ = consumer_writer.write_serialize(&report).await;
                }
                embassy_futures::select::Either3::Third(frame) => {
                    let report = MouseReport {
                        buttons: frame.buttons,
                        x: frame.dx,
                        y: frame.dy,
                        wheel: 0,
                        pan: 0,
                    };
                    let _ = mouse_writer.write_serialize(&report).await;
                }
            }
        }
    };
    // The vendor channel serves two flows on one endpoint pair: command
    // replies (echo the command byte) and unsolicited input events (0x80).
    // Events are best-effort: when no host is draining the IN endpoint the
    // write would park forever, so it gets a short timeout and the event is
    // dropped — a stale event left armed in the endpoint is harmless.
    let updater = async {
        let mut buf = [0u8; 32];
        loop {
            match select(raw_reader.read(&mut buf), EVENT_CH.receive()).await {
                Either::Second(ev) => {
                    let mut rep = [0u8; 32];
                    rep[0] = EVENT_REPORT;
                    rep[1..4].copy_from_slice(&ev);
                    let _ = with_timeout(Duration::from_millis(50), raw_writer.write(&rep)).await;
                }
                Either::First(res) => {
                    // `read` fails *immediately* while the endpoint is disabled
                    // — i.e. before the host configures us. A bare `continue`
                    // would never await and starve every other task; back off.
                    let Ok(_) = res else {
                        Timer::after_millis(100).await;
                        continue;
                    };
                    let mut reply = [0u8; 32];
                    reply[0] = buf[0];
                    match buf[0] {
                        CMD_VERSION => {
                            info!("app: version query -> {}", FW_VERSION);
                            reply[1] = FW_VERSION.len() as u8;
                            reply[2..2 + FW_VERSION.len()].copy_from_slice(FW_VERSION.as_bytes());
                        }
                        CMD_ENTER_DFU if &buf[1..5] == ENTER_DFU_KEY => {
                            warn!("app: DFU reboot requested -> ROM bootloader");
                            reply[1] = 0x01;
                            let _ = raw_writer.write(&reply).await;
                            // Let the ack reach the host before dropping off
                            // the bus.
                            Timer::after_millis(50).await;
                            dfu::reboot_into_bootloader();
                        }
                        CMD_GET_KEYMAP => {
                            let page = buf[1] as usize;
                            reply[1] = buf[1];
                            match keymap::read_page(page, &mut reply[3..31]) {
                                Some(n) => reply[2] = n as u8,
                                None => reply[2] = 0,
                            }
                        }
                        CMD_SET_KEYMAP => {
                            let ok =
                                keymap::write_page(buf[1] as usize, buf[2] as usize, &buf[3..31]);
                            info!(
                                "app: keymap page {=u8} write -> {}",
                                buf[1],
                                if ok { "ok" } else { "rejected" }
                            );
                            reply[1] = ok as u8;
                        }
                        CMD_SAVE if &buf[1..5] == SAVE_KEY => {
                            let ok = keymap::save_to_flash(&mut flash.borrow_mut()).is_ok();
                            info!(
                                "app: keymap save -> {}",
                                if ok { "flash written" } else { "FLASH ERROR" }
                            );
                            reply[1] = ok as u8;
                        }
                        CMD_FACTORY_RESET if &buf[1..5] == RESET_KEY => {
                            let ok = keymap::factory_reset(&mut flash.borrow_mut()).is_ok();
                            warn!("app: factory reset -> defaults");
                            reply[1] = ok as u8;
                        }
                        CMD_GET_ANALOG => {
                            let thr = keymap::joy_threshold();
                            reply[1..3].copy_from_slice(&thr.to_le_bytes());
                        }
                        CMD_SET_ANALOG => {
                            let thr = u16::from_le_bytes([buf[1], buf[2]]);
                            // Clamp to sane bounds: a tiny threshold would
                            // stream arrows from ADC noise, a huge one can
                            // never trigger.
                            let thr = thr.clamp(200, 1900);
                            keymap::KEYMAP.lock(|k| k.borrow_mut().joy_threshold = thr);
                            info!("app: joystick threshold -> {=u16}", thr);
                            reply[1] = 0x01;
                        }
                        CMD_GET_JOYMODE => {
                            reply[1] = keymap::joy_mode();
                            reply[2] = keymap::joy_mouse_speed();
                        }
                        CMD_SET_JOYMODE => {
                            let mode = if buf[1] <= keymap::JOY_MODE_GRADE {
                                buf[1]
                            } else {
                                keymap::DEFAULT_JOY_MODE
                            };
                            let speed = buf[2].clamp(1, 10);
                            keymap::KEYMAP.lock(|k| {
                                let mut k = k.borrow_mut();
                                k.joy_mode = mode;
                                k.joy_mouse_speed = speed;
                            });
                            info!("app: joystick mode -> {=u8} speed {=u8}", mode, speed);
                            reply[1] = 0x01;
                        }
                        CMD_GET_LED => {
                            reply[1] = keymap::led_brightness();
                        }
                        CMD_SET_LED => {
                            keymap::KEYMAP.lock(|k| k.borrow_mut().led_brightness = buf[1]);
                            info!("app: led brightness -> {=u8}", buf[1]);
                            reply[1] = 0x01;
                        }
                        CMD_GET_LEDPATTERN => {
                            let (kp, up) = keymap::led_patterns();
                            reply[1..5].copy_from_slice(&[kp.mode, kp.r, kp.g, kp.b]);
                            reply[5..9].copy_from_slice(&[up.mode, up.r, up.g, up.b]);
                        }
                        CMD_SET_LEDPATTERN => {
                            let sanitize = |mode: u8| {
                                if mode <= keymap::LED_PATTERN_SOLID {
                                    mode
                                } else {
                                    keymap::LED_PATTERN_RAINBOW
                                }
                            };
                            keymap::KEYMAP.lock(|k| {
                                let mut k = k.borrow_mut();
                                k.key_pattern = keymap::LedPattern {
                                    mode: sanitize(buf[1]),
                                    r: buf[2],
                                    g: buf[3],
                                    b: buf[4],
                                };
                                k.ug_pattern = keymap::LedPattern {
                                    mode: sanitize(buf[5]),
                                    r: buf[6],
                                    g: buf[7],
                                    b: buf[8],
                                };
                            });
                            info!("app: led patterns -> key mode {=u8}, ug mode {=u8}", buf[1], buf[5]);
                            reply[1] = 0x01;
                        }
                        CMD_SET_KEY_LED_OVERRIDE => {
                            let index = buf[1] as usize;
                            if index < 13 {
                                let bit = 1u16 << index;
                                if buf[2] != 0 {
                                    let rgb = ((buf[3] as u32) << 16)
                                        | ((buf[4] as u32) << 8)
                                        | buf[5] as u32;
                                    KEY_LED_OVERRIDE_RGB[index].store(
                                        rgb,
                                        core::sync::atomic::Ordering::Relaxed,
                                    );
                                    KEY_LED_OVERRIDE_MASK.fetch_or(
                                        bit,
                                        core::sync::atomic::Ordering::Relaxed,
                                    );
                                } else {
                                    KEY_LED_OVERRIDE_MASK.fetch_and(
                                        !bit,
                                        core::sync::atomic::Ordering::Relaxed,
                                    );
                                }
                                reply[1] = 0x01;
                            }
                        }
                        CMD_GET_MODE => {
                            reply[1] = DEVICE_MODE.load(core::sync::atomic::Ordering::Relaxed);
                        }
                        CMD_SET_MODE if &buf[2..6] == MODE_KEY => {
                            let mode = buf[1];
                            if mode > keymap::MODE_CODEX {
                                reply[1] = 0;
                            } else if mode == DEVICE_MODE.load(core::sync::atomic::Ordering::Relaxed)
                            {
                                // Already running it: nothing to persist or
                                // restart (an unchanged RAM copy may still
                                // have been edited; SAVE covers that).
                                reply[1] = 0x01;
                            } else {
                                // Persists the whole RAM configuration, like
                                // SAVE. On a flash error put the RAM mode
                                // back, or a later ordinary SAVE would carry
                                // the switch the app was just told failed.
                                keymap::set_device_mode(mode);
                                let ok = keymap::save_to_flash(&mut flash.borrow_mut()).is_ok();
                                if !ok {
                                    keymap::set_device_mode(
                                        DEVICE_MODE.load(core::sync::atomic::Ordering::Relaxed),
                                    );
                                }
                                warn!(
                                    "app: device mode -> {=u8} ({})",
                                    mode,
                                    if ok { "saved, resetting" } else { "FLASH ERROR" }
                                );
                                reply[1] = ok as u8;
                                if ok {
                                    let _ = raw_writer.write(&reply).await;
                                    // Let the ack reach the host, then come
                                    // back up with the new identity.
                                    Timer::after_millis(50).await;
                                    dfu::reboot();
                                }
                            }
                        }
                        other => {
                            debug!("app: unknown cmd 0x{=u8:02x}", other);
                            continue;
                        }
                    }
                    let _ = raw_writer.write(&reply).await;
                }
            }
        }
    };
    // Codex Micro compat interface: only exists in that mode; otherwise this
    // future just parks.
    let codex_fut = async {
        match codex_parts {
            Some((mut reader, mut writer)) => {
                let mut store = codex::files::FlashStore::new(flash);
                codex::pump(&mut reader, &mut writer, &mut store).await
            }
            None => core::future::pending::<()>().await,
        }
    };
    join4(usb_fut, pump, updater, codex_fut).await;
}

/// 1 kHz matrix scan with 5 ms debounce + encoder quadrature + buttons.
/// Quadrature transition table. A state is `(A << 1) | B`; the lookup index is
/// `(prev << 2) | now`. ±1 mark a valid step either way; 0 covers "no change"
/// and the illegal two-bit jumps that contact bounce produces.
///
/// A bare "did A change?" test cannot decode direction: in Gray code exactly
/// one of A/B moves per step, so "A changed" always implies "B did not" — which
/// is why the previous version could only ever emit one direction.
#[rustfmt::skip]
const QUAD_LUT: [i8; 16] = [
     0,  1, -1,  0,
    -1,  0,  0,  1,
     1,  0,  0, -1,
     0, -1,  1,  0,
];

/// Quadrature counts per detent — one full 4-state cycle on this EC11.
const ENC_COUNTS_PER_DETENT: i8 = 4;

#[embassy_executor::task]
async fn scan_task(
    mut rows: [Output<'static>; 4],
    cols: [Input<'static>; 4],
    enc_sw: Input<'static>,
    joy_sw: Input<'static>,
) {
    info!("scan_task: entered");
    let mut ticker = Ticker::every(Duration::from_millis(1));
    let mut n: u32 = 0;
    let mut debounce = [[0u8; 4]; 4];
    let mut pressed = [[false; 4]; 4];
    // Keys already down when we start (the boot chord) must not emit a
    // press — nor a release when they finally come up. Seed the state from
    // one scan and swallow that first release edge.
    let mut suppress = [[false; 4]; 4];
    for (ri, row) in rows.iter_mut().enumerate() {
        row.set_high();
        cortex_m::asm::delay(48);
        for (ci, col) in cols.iter().enumerate() {
            if col.is_high() {
                pressed[ri][ci] = true;
                suppress[ri][ci] = true;
            }
        }
        row.set_low();
    }
    let mut enc_sw_last = enc_sw.is_high();
    let mut joy_sw_last = joy_sw.is_high();
    // Same rule for the two push switches: down at boot means their first
    // release is not an input either.
    let mut enc_sw_suppress = !enc_sw_last;
    let mut joy_sw_suppress = !joy_sw_last;
    // Which form of "joystick push down" is outstanding, so a mode switch
    // mid-hold retracts exactly what was asserted.
    let mut joy_key_held = false;
    let mut joy_click_held = false;

    loop {
        ticker.next().await;

        // -- matrix: debounced edges feed the shared held-slot set --
        for (ri, row) in rows.iter_mut().enumerate() {
            row.set_high();
            // Two cycles of settle: the row line + diode charge instantly at
            // these impedances, one nop-read is enough on the M0.
            cortex_m::asm::delay(48);
            for (ci, col) in cols.iter().enumerate() {
                let raw = col.is_high();
                if raw == pressed[ri][ci] {
                    debounce[ri][ci] = 0;
                } else {
                    debounce[ri][ci] += 1;
                    if debounce[ri][ci] >= 5 {
                        pressed[ri][ci] = raw;
                        debounce[ri][ci] = 0;
                        let pos = POSITIONS[ri][ci];
                        if suppress[ri][ci] {
                            // The boot-chord key letting go: not an input.
                            suppress[ri][ci] = false;
                        } else if pos >= 0 {
                            let pos = pos as usize;
                            info!(
                                "key p{=usize} {} (r{=usize} c{=usize})",
                                pos,
                                if raw { "DOWN" } else { "UP" },
                                ri,
                                ci
                            );
                            if codex_mode() {
                                act(codex::key_binding(pos as u8), raw, pos);
                            } else {
                                set_held(pos, raw);
                            }
                            post_event(0, pos as u8, raw as u8);
                            // LED feedback tracks the physical press whatever
                            // the slot emits (or even if it emits nothing).
                            let mut state = KEYSTATE.load(core::sync::atomic::Ordering::Relaxed);
                            if raw {
                                state |= 1 << pos;
                            } else {
                                state &= !(1 << pos);
                            }
                            KEYSTATE.store(state, core::sync::atomic::Ordering::Relaxed);
                        }
                    }
                }
            }
            row.set_low();
        }

        // -- encoder push + joystick push: held slots like any key --
        let e = enc_sw.is_high();
        if e != enc_sw_last && enc_sw_suppress {
            enc_sw_suppress = false;
        } else if e != enc_sw_last {
            info!("encoder switch {}", if e { "UP" } else { "DOWN" });
            if codex_mode() {
                act(
                    codex::encoder_binding(codex::layout::ENCODER_PRESS),
                    !e,
                    keymap::SLOT_ENC_PRESS,
                );
            } else {
                set_held(keymap::SLOT_ENC_PRESS, !e);
            }
            post_event(2, !e as u8, 0);
        }
        enc_sw_last = e;

        // Joystick push: a held key slot in keys mode, mouse button 1 in
        // the pointer modes (mouse and grade). The mode can change while the
        // switch is down (app sync), so retract a stale assertion before
        // honouring the new mode. Codex mode has no known stick-click
        // message, so there it only feeds the app's live view.
        let j = joy_sw.is_high();
        let pointer_mode = !codex_mode()
            && matches!(
                keymap::joy_mode(),
                keymap::JOY_MODE_MOUSE | keymap::JOY_MODE_GRADE
            );
        if pointer_mode && joy_key_held {
            set_held(keymap::SLOT_JOY_PRESS, false);
            joy_key_held = false;
        }
        if !pointer_mode && joy_click_held {
            joy_click_held = false;
            MOUSE_BUTTONS.store(0, core::sync::atomic::Ordering::Relaxed);
            force_send_mouse(MouseFrame {
                buttons: mouse_buttons(),
                dx: 0,
                dy: 0,
            });
        }
        if j != joy_sw_last && joy_sw_suppress {
            joy_sw_suppress = false;
        } else if j != joy_sw_last {
            info!("joystick switch {}", if j { "UP" } else { "DOWN" });
            let down = !j;
            if codex_mode() {
                // event only (below)
            } else if pointer_mode {
                joy_click_held = down;
                MOUSE_BUTTONS.store(down as u8, core::sync::atomic::Ordering::Relaxed);
                force_send_mouse(MouseFrame {
                    buttons: mouse_buttons(),
                    dx: 0,
                    dy: 0,
                });
            } else {
                set_held(keymap::SLOT_JOY_PRESS, down);
                joy_key_held = down;
            }
            post_event(3, 4, down as u8);
        }
        joy_sw_last = j;

        // ~1 s heartbeat of the raw levels. The four pins above only log on
        // change, so this is what makes a stuck (or floating) line visible
        // without anyone having to press anything.
        n = n.wrapping_add(1);
        if n % 1000 == 0 {
            debug!(
                "inputs: enc_sw={} joy_sw={} | cols={} {} {} {}",
                e,
                j,
                cols[0].is_high(),
                cols[1].is_high(),
                cols[2].is_high(),
                cols[3].is_high()
            );
        }
    }
}

/// Rotary encoder, edge-driven. Every A/B transition raises EXTI, so nothing is
/// lost to scan aliasing at speed or to the LED bit-bang's critical section.
/// Contact bounce needs no filtering here: a bounce between two adjacent states
/// yields +1 then -1, which the accumulator cancels on its own, and the illegal
/// two-bit jumps map to 0 in the table.
#[embassy_executor::task]
async fn encoder_task(mut enc_a: ExtiInput<'static>, mut enc_b: ExtiInput<'static>) {
    info!("encoder_task: entered");
    let mut state = ((enc_a.is_high() as u8) << 1) | (enc_b.is_high() as u8);
    let mut accum: i8 = 0;
    loop {
        embassy_futures::select::select(enc_a.wait_for_any_edge(), enc_b.wait_for_any_edge()).await;

        let s = ((enc_a.is_high() as u8) << 1) | (enc_b.is_high() as u8);
        if s == state {
            continue;
        }
        // Negated: the board's ENC_A/ENC_B pin ordering runs opposite the
        // table's sense, so without this a clockwise turn counts negative.
        let delta = -QUAD_LUT[((state << 2) | s) as usize];
        debug!(
            "enc state {=u8} -> {=u8} delta={=i8} accum={=i8}",
            state, s, delta, accum
        );
        state = s;
        accum += delta;

        while accum >= ENC_COUNTS_PER_DETENT {
            accum -= ENC_COUNTS_PER_DETENT;
            info!("encoder CW");
            if codex_mode() {
                let b = codex::encoder_binding(codex::layout::ENCODER_CW);
                act(b, true, keymap::SLOT_ENC_CW);
                act(b, false, keymap::SLOT_ENC_CW);
            } else {
                tap_slot(keymap::SLOT_ENC_CW);
            }
            post_event(1, 1, 0);
        }
        while accum <= -ENC_COUNTS_PER_DETENT {
            accum += ENC_COUNTS_PER_DETENT;
            info!("encoder CCW");
            if codex_mode() {
                let b = codex::encoder_binding(codex::layout::ENCODER_CCW);
                act(b, true, keymap::SLOT_ENC_CCW);
                act(b, false, keymap::SLOT_ENC_CCW);
            } else {
                tap_slot(keymap::SLOT_ENC_CCW);
            }
            post_event(1, 0, 0);
        }
    }
}

/// Joystick, 50 Hz ADC poll. Keys mode: deflection past the (configurable)
/// threshold holds that direction's slot until the stick returns to centre.
/// Mouse mode: deflection past a small fixed dead zone moves the HID mouse
/// pointer proportionally, scaled by the app-tunable speed. Grade mode: the
/// same motion with the speed applied squared (fine floor, real top pace)
/// and the left button auto-held while deflected — park the pointer over a
/// DaVinci Resolve colour wheel and the stick grabs and drags it like a
/// panel trackball, letting go at centre. Threshold crossings post app
/// events in every mode, so the app's live feedback keeps working whatever
/// the stick means.
#[embassy_executor::task]
async fn adc_task(
    mut adc: Adc<'static, peripherals::ADC1>,
    mut joy_x: peripherals::PB1,
    mut joy_y: JoyYPin,
) {
    info!("adc_task: entered");
    let mut ticker = Ticker::every(Duration::from_millis(20));
    // Direction indices on the wire (and their slots, at SLOT_JOY_UP + dir).
    const DIR_NONE: u8 = 0xFF;
    const DIR_SLOTS: [usize; 4] = [
        keymap::SLOT_JOY_UP,
        keymap::SLOT_JOY_DOWN,
        keymap::SLOT_JOY_LEFT,
        keymap::SLOT_JOY_RIGHT,
    ];
    /// Mouse-mode dead zone in ADC counts around centre — fixed and small;
    /// the key threshold is a *trigger point*, this only absorbs stick slop.
    const MOUSE_DEADZONE: i32 = 200;
    /// Fractional-motion denominator: px/frame = deflection × speed / DIV.
    /// Full deflection (~1848 counts) at speed 5 ≈ 18 px per 20 ms frame.
    const MOUSE_DIV: i32 = 512;
    /// Grade-mode denominator, used with the speed applied SQUARED (1..100
    /// across the slider): full deflection spans ≈0.2 px/frame at speed 1 to
    /// ≈23 px/frame at speed 10. The squared curve buys two decades of range
    /// — sub-pixel precision at the bottom, real pace at the top (a flat 10x
    /// tested too slow on Resolve's colour wheels).
    const GRADE_DIV: i32 = 8192;
    /// Grade-mode drag release threshold, in ADC counts around centre —
    /// deliberately inside MOUSE_DEADZONE so the auto-held button only lets
    /// go once the stick is clearly home, and jitter at the dead-zone edge
    /// cannot machine-gun clicks on whatever the pointer is over.
    const DRAG_RELEASE: i32 = 120;
    fn past_deadzone(centred: i32) -> i32 {
        if centred > MOUSE_DEADZONE {
            centred - MOUSE_DEADZONE
        } else if centred < -MOUSE_DEADZONE {
            centred + MOUSE_DEADZONE
        } else {
            0
        }
    }
    /// Direction → stick angle in thousandths of a turn (right 0, down 250,
    /// left 500, up 750), the convention of both `v.oai.rad` and the Input
    /// app's radial sectors.
    const DIR_ANGLE: [u16; 4] = [750, 250, 500, 0];
    let mut last: u8 = DIR_NONE;
    let mut sector_binding = Binding::None;
    let mut last_mode = keymap::JOY_MODE_KEYS;
    let mut dragging = false;
    let mut acc_x: i32 = 0;
    let mut acc_y: i32 = 0;
    let mut n: u32 = 0;
    loop {
        ticker.next().await;
        let a = adc.read(&mut joy_x).await;
        let b = adc.read(&mut joy_y).await;
        // The current board mounts the stick rotated, so the JOY_X net (PB1)
        // senses the stick's VERTICAL travel and PA0 the horizontal. Swap at
        // the source so (x, y) mean what they say for keys and mouse alike.
        #[cfg(not(feature = "proto"))]
        let (x, y) = (b, a);
        #[cfg(feature = "proto")]
        let (x, y) = (a, b);
        // Once a second, so the raw swing can be eyeballed while the stick is
        // moved — these are the numbers the threshold is judged against.
        n = n.wrapping_add(1);
        if n % 50 == 0 {
            debug!("joystick raw x={=u16} y={=u16}", x, y);
        }
        // 12-bit, centre ~2048; the dead zone is the app-tunable threshold.
        let thr = keymap::joy_threshold();
        let lo = 2048u16.saturating_sub(thr);
        let hi = 2048u16.saturating_add(thr);
        let dir = if x < lo {
            2 // left
        } else if x > hi {
            3 // right
        } else if y < lo {
            0 // up
        } else if y > hi {
            1 // down
        } else {
            DIR_NONE
        };

        // The app can flip the mode while the stick is deflected; hand the
        // active direction over so no mode strands a held key, and drop any
        // grade-mode drag so the button cannot stay latched. In Codex mode
        // the joystick mode is moot: deflections go to the host as analog
        // stick directions, nothing else.
        let codex = codex_mode();
        let mode = keymap::joy_mode();
        let keys = mode == keymap::JOY_MODE_KEYS;
        if !codex && mode != last_mode {
            last_mode = mode;
            acc_x = 0;
            acc_y = 0;
            if last != DIR_NONE {
                set_held(DIR_SLOTS[last as usize], keys);
            }
            if dragging {
                dragging = false;
                GRADE_DRAG.store(false, core::sync::atomic::Ordering::Relaxed);
                force_send_mouse(MouseFrame {
                    buttons: mouse_buttons(),
                    dx: 0,
                    dy: 0,
                });
            }
        }

        if dir != last {
            info!("joystick dir {=u8} (x={=u16} y={=u16})", dir, x, y);
            // Codex mode: the keymap decides whether the stick is the Codex
            // Micro's analog stick (VENDOR) or a radial menu of keycode
            // sectors — those also get a kb.radial notification so the
            // Input app can draw the menu.
            let joy = if codex { codex::joystick_mode() } else { Joystick::None };
            if last != DIR_NONE {
                match joy {
                    Joystick::Vendor => codex::stick(last, false),
                    Joystick::Sectors => {
                        act(sector_binding, false, DIR_SLOTS[last as usize]);
                        let st = codex::status();
                        codex::post(codex::Event::Radial {
                            angle_milli: DIR_ANGLE[last as usize],
                            open: false,
                            layer: st.layer_index,
                            profile: st.profile_index,
                        });
                    }
                    Joystick::None if keys && !codex => set_held(DIR_SLOTS[last as usize], false),
                    Joystick::None => {}
                }
                post_event(3, last, 0);
            }
            if dir != DIR_NONE {
                match joy {
                    Joystick::Vendor => codex::stick(dir, true),
                    Joystick::Sectors => {
                        let angle = DIR_ANGLE[dir as usize];
                        sector_binding = codex::sector(angle);
                        let st = codex::status();
                        codex::post(codex::Event::Radial {
                            angle_milli: angle,
                            open: true,
                            layer: st.layer_index,
                            profile: st.profile_index,
                        });
                        act(sector_binding, true, DIR_SLOTS[dir as usize]);
                    }
                    Joystick::None if keys && !codex => set_held(DIR_SLOTS[dir as usize], true),
                    Joystick::None => {}
                }
                post_event(3, dir, 1);
            }
            last = dir;
        }

        if !codex && !keys {
            let grade = mode == keymap::JOY_MODE_GRADE;
            let cx = past_deadzone(x as i32 - 2048);
            let cy = past_deadzone(y as i32 - 2048);

            // Grade mode: deflection grabs (button down, BEFORE any motion
            // frame so the host sees press-then-drag), returning to centre
            // lets go. Fresh grab, fresh fractions — residue from the last
            // drag must not jump the wheel on contact.
            if grade {
                if !dragging && (cx != 0 || cy != 0) {
                    dragging = true;
                    GRADE_DRAG.store(true, core::sync::atomic::Ordering::Relaxed);
                    acc_x = 0;
                    acc_y = 0;
                    force_send_mouse(MouseFrame {
                        buttons: mouse_buttons(),
                        dx: 0,
                        dy: 0,
                    });
                } else if dragging
                    && (x as i32 - 2048).abs() < DRAG_RELEASE
                    && (y as i32 - 2048).abs() < DRAG_RELEASE
                {
                    dragging = false;
                    GRADE_DRAG.store(false, core::sync::atomic::Ordering::Relaxed);
                    acc_x = 0;
                    acc_y = 0;
                    force_send_mouse(MouseFrame {
                        buttons: mouse_buttons(),
                        dx: 0,
                        dy: 0,
                    });
                }
            }

            let speed = keymap::joy_mouse_speed() as i32;
            let (gain, div) = if grade {
                (speed * speed, GRADE_DIV)
            } else {
                (speed, MOUSE_DIV)
            };
            acc_x += cx * gain;
            acc_y += cy * gain;
            let dx = (acc_x / div).clamp(-127, 127) as i8;
            let dy = (acc_y / div).clamp(-127, 127) as i8;
            if dx != 0 || dy != 0 {
                acc_x -= dx as i32 * div;
                acc_y -= dy as i32 * div;
                force_send_mouse(MouseFrame {
                    buttons: mouse_buttons(),
                    dx,
                    dy,
                });
            }
        }
    }
}

/// Cap-touch on PB9 by RC charge time: discharge the pad, release it to the
/// internal pull-up, and count polls until the input reads high. A finger adds
/// capacitance -> longer rise. Self-calibrates a floating baseline.
///
/// The rise is only ~20 CPU cycles at 48 MHz (~40 kOhm pull-up into ~15 pF), so
/// the sense loop has to start essentially immediately after the pad is
/// released. `Flex::set_as_input(Pull::Up)` rewrites PUPDR *and* MODER every
/// call, which costs longer than the rise itself — the pad was always already
/// high by the first sample, so `t` could only ever read 0 and the trigger
/// (`t > baseline + baseline/2 + 8`) reduced to `0 > 8`, i.e. never.
///
/// So PUPDR and ODR are configured once up front and the loop flips *only*
/// MODER, via a value computed before the critical section. Each tick sums
/// several charge cycles: one cycle separates touched from untouched by just a
/// handful of counts, and accumulating trades a little time for the SNR that
/// makes the threshold meaningful.
#[embassy_executor::task]
async fn touch_task(mut pad: Flex<'static>) {
    use embassy_stm32::pac;
    use embassy_stm32::pac::gpio::regs::Moder;
    use embassy_stm32::pac::gpio::vals::Idr;

    /// PB9 — bit position within GPIOB's registers.
    const PIN: usize = 9;
    /// Charge cycles summed per tick. Each cycle only resolves ~3 counts, so
    /// the sum is what gives the threshold something to bite on.
    const SAMPLES: u32 = 64;
    /// Cycles held low to fully drain the pad before each measurement.
    const DISCHARGE_CYCLES: u32 = 48 * 20; // ~20 us
    /// Escape hatch so a shorted or floating pad cannot wedge the loop.
    const MAX_COUNT: u32 = 2_000;

    info!("touch_task: entered");

    // One-time setup: pull-up selected in PUPDR, ODR low so that flipping
    // MODER to OUTPUT sinks the pad without touching any other register.
    pad.set_as_input(Pull::Up);
    let gpio = pac::GPIOB;
    gpio.bsrr().write(|w| w.set_br(PIN, true));

    let mut ticker = Ticker::every(Duration::from_millis(30));
    let mut baseline: u32 = 0;
    let mut armed = true;
    let mut n: u32 = 0;
    let mut last_t: u32 = 0;
    loop {
        ticker.next().await;

        // Re-read MODER each tick so a pin reconfigured elsewhere is picked
        // up; inside the critical section nothing else can change it, so the
        // two cached values stay valid for the whole burst.
        let base = gpio.moder().read().0 & !(0b11 << (PIN * 2));
        let moder_input = Moder(base);
        let moder_output = Moder(base | (0b01 << (PIN * 2)));

        let mut t: u32 = 0;
        critical_section::with(|_| {
            for _ in 0..SAMPLES {
                gpio.moder().write_value(moder_output); // drive low, drain pad
                cortex_m::asm::delay(DISCHARGE_CYCLES);
                gpio.moder().write_value(moder_input); // release; poll at once
                let mut c: u32 = 0;
                while gpio.idr().read().idr(PIN) == Idr::LOW && c < MAX_COUNT {
                    c += 1;
                }
                t += c;
            }
        });

        // exponential baseline of the untouched pad
        if baseline == 0 {
            baseline = t;
        }
        // ~1 s cadence: the gap between `t` and `baseline` is what decides a
        // touch, so both numbers are needed to tune the threshold below.
        // Log on change as well as once a second: the untouched reading is
        // dead stable, so any movement at all is the signal worth seeing.
        n = n.wrapping_add(1);
        if t != last_t || n % 33 == 0 {
            debug!("touch charge t={=u32} baseline={=u32}", t, baseline);
        }
        last_t = t;
        // 25% over baseline. Measured on hardware: untouched sits at exactly
        // 192 with zero jitter over long runs, a finger reads 242..1015, and
        // a hovering hand tops out around 218 — so this sits clear of both.
        let touched = t > baseline + baseline / 4;
        if !touched {
            baseline = (baseline * 15 + t) / 16;
        }
        if touched && armed {
            info!("touch TAP (t={=u32} baseline={=u32})", t, baseline);
            armed = false;
            // Codex mode: the touch pad is the keymap's first "button".
            if codex_mode() {
                let b = codex::touch_binding();
                act(b, true, keymap::SLOT_TOUCH_TAP);
                act(b, false, keymap::SLOT_TOUCH_TAP);
            } else {
                tap_slot(keymap::SLOT_TOUCH_TAP);
            }
            post_event(4, 1, 0);
        } else if !touched {
            armed = true;
        }
    }
}

/// 30 Hz LED renderer: pressed keys light white, idle keys a dim rainbow;
/// the perimeter underglow ring runs a slow hue rotation. Brightness is capped
/// to stay inside the 500 mA VBUS budget.
#[embassy_executor::task]
async fn led_task(mut led_key: ws2812::LedPin<'static>, mut led_ug: ws2812::LedPin<'static>) {
    info!("led_task: entered");
    let mut ticker = Ticker::every(Duration::from_millis(33));
    let mut phase: u8 = 0;
    // Codex mode: one animation clock per host-described light (six agent
    // keys, the command-key strip, the ring), each advancing by that light's
    // own speed every frame so a stopped effect really stands still.
    let mut anim = [0u32; 8];
    loop {
        ticker.next().await;
        phase = phase.wrapping_add(1);
        let state = KEYSTATE.load(core::sync::atomic::Ordering::Relaxed);
        // ~8.5 s heartbeat — cheap proof the executor is still scheduling.
        if phase == 0 {
            debug!("led: alive, keystate=0x{=u16:04x}", state);
            // Bodge diagnostic: release each open-drain data line and read
            // it back — true only when an external pull-up lifts this net,
            // i.e. the 5V bodge resistor is really on the DIN trace.
        }

        // The app-tunable brightness dims DOWN from the hard-coded scales,
        // which stay the ceiling: they are what keeps the whole board inside
        // the 500 mA VBUS budget. The setting is SQUARED because perceived
        // brightness is roughly quadratic in duty cycle — linear scaling
        // made the top half of the slider look like nothing happened.
        let bright = keymap::led_brightness() as u32;
        let dim = |base: u32| ((base * bright * bright) / (255 * 255)) as u8;
        let (key_pat, ug_pat) = keymap::led_patterns();
        let override_mask =
            KEY_LED_OVERRIDE_MASK.load(core::sync::atomic::Ordering::Relaxed);
        // Codex mode: whatever lighting the host has described so far
        // outranks the pad's own patterns and the app's overrides.
        let host = if codex_mode() { Some(codex::lights()) } else { None };
        if let Some(h) = host.as_ref() {
            for (clock, light) in anim.iter_mut().zip(
                h.agents
                    .iter()
                    .chain(core::iter::once(&h.keys))
                    .chain(core::iter::once(&h.ambient)),
            ) {
                *clock = clock.wrapping_add(light.speed());
            }
        }

        let mut keys = [ws2812::Grb::default(); 13];
        for (i, px) in keys.iter_mut().enumerate() {
            // key_leds[i] sits under key position i's switch — chain order is
            // the sw index order in the .cohdl, and every position is
            // independent now (the 2U pair each has its own LED and bit).
            // A pressed key always pops white, whatever the idle pattern.
            *px = if state & (1 << i) != 0 {
                ws2812::Grb::rgb(255, 255, 255).scaled(dim(24))
            } else if let Some(c) = host.as_ref().and_then(|h| codex_key_light(h, &anim, i)) {
                c.scaled(dim(20))
            } else if override_mask & (1 << i) != 0 {
                let rgb = KEY_LED_OVERRIDE_RGB[i]
                    .load(core::sync::atomic::Ordering::Relaxed);
                ws2812::Grb::rgb(
                    ((rgb >> 16) & 0xff) as u8,
                    ((rgb >> 8) & 0xff) as u8,
                    (rgb & 0xff) as u8,
                )
                .scaled(dim(18))
            } else if key_pat.mode == keymap::LED_PATTERN_SOLID {
                ws2812::Grb::rgb(key_pat.r, key_pat.g, key_pat.b).scaled(dim(6))
            } else {
                hue(phase.wrapping_add((i as u8) * 20)).scaled(dim(6))
            };
        }
        // The hue step is 256/UG_LEN so the ring carries exactly one full
        // wheel around the board regardless of revision. The boot splash
        // (mode colour) and the Codex host's ambient light take precedence.
        let splash_px = splash_pixel();
        let mut ring = [ws2812::Grb::default(); UG_LEN];
        for (i, px) in ring.iter_mut().enumerate() {
            *px = if let Some(s) = splash_px {
                s.scaled(dim(8))
            } else if let Some(a) = host
                .as_ref()
                .and_then(|h| codex_light_pixel(&h.ambient, anim[7], i, UG_LEN))
            {
                a.scaled(dim(8))
            } else if ug_pat.mode == keymap::LED_PATTERN_SOLID {
                ws2812::Grb::rgb(ug_pat.r, ug_pat.g, ug_pat.b).scaled(dim(8))
            } else {
                hue(phase.wrapping_add((i as u8) * (256 / UG_LEN) as u8)).scaled(dim(8))
            };
        }
        #[cfg(not(feature = "proto"))]
        {
            let _ = (&mut led_key, &mut led_ug);
            ws2812::write_raw(embassy_stm32::pac::GPIOA, 8, &keys);
            ws2812::write_raw(embassy_stm32::pac::GPIOB, 14, &ring);
        }
        #[cfg(feature = "proto")]
        {
            ws2812::write_raw(embassy_stm32::pac::GPIOB, 4, &keys);
            ws2812::write_raw(embassy_stm32::pac::GPIOA, 0, &ring);
        }
    }
}

/// The boot splash colour while it runs (solid, or 150 ms on/off when a
/// chord just changed the mode); None once it has expired.
fn splash_pixel() -> Option<ws2812::Grb> {
    use core::sync::atomic::Ordering::Relaxed;
    let now = Instant::now().as_millis() as u32;
    let until = SPLASH_UNTIL_MS.load(Relaxed);
    let remaining = until.wrapping_sub(now) as i32;
    if remaining <= 0 {
        return None;
    }
    if SPLASH_BLINK.load(Relaxed) && (remaining as u32 / 150) % 2 == 1 {
        return Some(ws2812::Grb::default());
    }
    let rgb = SPLASH_RGB.load(Relaxed);
    Some(ws2812::Grb::rgb(
        ((rgb >> 16) & 0xFF) as u8,
        ((rgb >> 8) & 0xFF) as u8,
        (rgb & 0xFF) as u8,
    ))
}

/// Codex mode: the host's light for key `i` — an agent status light for
/// the six Agent Keys (p0..p5, each its own one-LED strip with its own
/// clock), or pixel `i - 6` of the seven-key Command Key strip. None until
/// the host has described it, so the pad's own pattern shows meanwhile.
fn codex_key_light(h: &codex::Lights, anim: &[u32; 8], i: usize) -> Option<ws2812::Grb> {
    if i < 6 {
        codex_light_pixel(&h.agents[i], anim[i], 0, 1)
    } else {
        codex_light_pixel(&h.keys, anim[6], i - 6, 7)
    }
}

/// Pixel `i` of an `n`-LED strip driven by a host-described light, at that
/// light's animation clock `acc` (advanced by its speed every 30 Hz frame,
/// so speed 0.4 breathes about every 1.4 s and runs a snake round the ring
/// in about the same). Effects follow the host's device kit: off, solid,
/// snake (a three-LED segment with a fading tail; on a single LED it
/// breathes instead), rainbow (hue cycles, colour ignored), breath,
/// gradient (a hue spread along the strip), shallow breath (50–100 %).
fn codex_light_pixel(light: &codex::Light, acc: u32, i: usize, n: usize) -> Option<ws2812::Grb> {
    use codex::wire::*;
    if !light.set {
        return None;
    }
    let level = light.level as u32;
    let scaled = |c: ws2812::Grb, num: u32| ws2812::Grb {
        g: (c.g as u32 * num / 255) as u8,
        r: (c.r as u32 * num / 255) as u8,
        b: (c.b as u32 * num / 255) as u8,
    };
    let base = ws2812::Grb::rgb(
        ((light.rgb >> 16) & 0xFF) as u8,
        ((light.rgb >> 8) & 0xFF) as u8,
        (light.rgb & 0xFF) as u8,
    );
    // A full breath cycle is 256 phase units: up over the first half, down
    // over the second.
    let breath_phase = ((acc >> 6) & 0xFF) as u32;
    let tri = if breath_phase < 128 { breath_phase * 2 } else { (255 - breath_phase) * 2 }; // 0..=255
    let n = n.max(1);
    let px = match light.effect {
        EFFECT_OFF => return Some(ws2812::Grb::default()),
        EFFECT_SNAKE if n > 1 => {
            // Head LED plus a tail of two, dimming behind it.
            let head = ((acc >> 11) as usize) % n;
            let behind = (head + n - i) % n;
            match behind {
                0 => scaled(base, level),
                1 => scaled(base, level * 2 / 3),
                2 => scaled(base, level / 3),
                _ => ws2812::Grb::default(),
            }
        }
        EFFECT_SNAKE | EFFECT_BREATH => scaled(base, level * (51 + tri * 204 / 255) / 255),
        EFFECT_SHALLOW_BREATH => scaled(base, level * (128 + tri / 2) / 255),
        EFFECT_RAINBOW => scaled(
            hue(((acc >> 7) as u8).wrapping_add((i * 256 / n) as u8)),
            level,
        ),
        EFFECT_GRADIENT => scaled(
            hue(((acc >> 7) as u8).wrapping_add((i * 256 / n) as u8)),
            level,
        ),
        _ => scaled(base, level), // solid, and anything newer than we know
    };
    Some(px)
}

/// Cheap 0..255 hue -> saturated RGB.
fn hue(h: u8) -> ws2812::Grb {
    let seg = h / 43;
    let rem = (h % 43) * 6;
    match seg {
        0 => ws2812::Grb::rgb(255, rem, 0),
        1 => ws2812::Grb::rgb(255 - rem, 255, 0),
        2 => ws2812::Grb::rgb(0, 255, rem),
        3 => ws2812::Grb::rgb(0, 255 - rem, 255),
        4 => ws2812::Grb::rgb(rem, 0, 255),
        _ => ws2812::Grb::rgb(255, 0, 255 - rem),
    }
}
