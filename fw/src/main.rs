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
//!   JOY_X/Y  = PB1/ADC_IN9  PA0/ADC_IN0
//!   JOY_SW   = PA15      (pull-up, active low)
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
//! joystick -> arrows/enter.
//!
//! A third, vendor-defined HID interface (usage page 0xFF60) carries the
//! app protocol: version query, DFU reboot, keymap read/write/save, analog
//! tuning, and unsolicited input-event reports (first byte 0x80) that give
//! the app live press feedback without any OS input-monitoring permission.

#![no_std]
#![no_main]

mod dfu;
mod keymap;
mod ws2812;

use embassy_executor::Spawner;
use embassy_futures::join::join3;
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
use embassy_time::{with_timeout, Duration, Ticker, Timer};
use embassy_usb::class::hid::{HidReaderWriter, HidWriter, State};
use embassy_usb::driver::{Driver as UsbDriver, EndpointError};
// Bring-up logging over the SWD probe (RTT) + panic messages on the same
// channel. `DEFMT_LOG=off` compiles every log statement out.
use defmt::{debug, info, warn};
use defmt_rtt as _;
use panic_probe as _;
use static_cell::StaticCell;
use usbd_hid::descriptor::{KeyboardReport, MediaKeyboardReport, SerializedDescriptor};

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
const CMD_VERSION: u8 = 0x01;
const CMD_ENTER_DFU: u8 = 0x02;
const CMD_GET_KEYMAP: u8 = 0x03;
const CMD_SET_KEYMAP: u8 = 0x04;
const CMD_SAVE: u8 = 0x05;
const CMD_FACTORY_RESET: u8 = 0x06;
const CMD_GET_ANALOG: u8 = 0x07;
const CMD_SET_ANALOG: u8 = 0x08;
const ENTER_DFU_KEY: &[u8; 4] = b"DFU!";
const SAVE_KEY: &[u8; 4] = b"SAVE";
const RESET_KEY: &[u8; 4] = b"RST!";
const EVENT_REPORT: u8 = 0x80;

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

/// Which slots are currently held (matrix keys by position, plus the button
/// and joystick-direction slots), each with the Slot SNAPSHOT taken at press
/// time. One shared set so the keyboard report is always rebuilt from the
/// WHOLE truth — a joystick move can no longer drop a held key from the
/// host's point of view. The snapshot matters: the app can rewrite the
/// keymap mid-hold (profile switch), and a release must retract exactly what
/// its press emitted, not whatever the slot means now.
static HELD: embassy_sync::blocking_mutex::Mutex<
    ThreadModeRawMutex,
    core::cell::RefCell<[Option<keymap::Slot>; keymap::SLOT_COUNT]>,
> = embassy_sync::blocking_mutex::Mutex::new(core::cell::RefCell::new([None; keymap::SLOT_COUNT]));

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

fn keyboard_report(held: &[Option<keymap::Slot>; keymap::SLOT_COUNT]) -> KeyboardReport {
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
    // Press dispatches on the slot's current meaning; release dispatches on
    // the snapshot stored at press time.
    let (changed, s, before, after) = HELD.lock(|h| {
        let mut h = h.borrow_mut();
        let before = keyboard_report(&h);
        if held {
            let s = keymap::slot(slot_idx);
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
    let mut flash = Flash::new_blocking(p.FLASH);
    if keymap::load_from_flash() {
        info!("keymap: loaded saved configuration from flash");
    } else {
        info!("keymap: no saved configuration — factory defaults");
    }

    // ---- USB HID: a boot keyboard + a consumer-control interface ----
    let driver = Driver::new(p.USB, Irqs, p.PA12, p.PA11);
    let mut usb_config = embassy_usb::Config::new(0x1209, 0x0001);
    usb_config.manufacturer = Some("conol");
    usb_config.product = Some("OpenMicro");
    usb_config.serial_number = Some("0001");
    usb_config.device_release = version_bcd(FW_VERSION);

    static CONFIG_DESC: StaticCell<[u8; 256]> = StaticCell::new();
    static BOS_DESC: StaticCell<[u8; 256]> = StaticCell::new();
    static CONTROL_BUF: StaticCell<[u8; 64]> = StaticCell::new();
    static KBD_STATE: StaticCell<State> = StaticCell::new();
    static CONSUMER_STATE: StaticCell<State> = StaticCell::new();
    static RAW_STATE: StaticCell<State> = StaticCell::new();

    let mut builder = embassy_usb::Builder::new(
        driver,
        usb_config,
        CONFIG_DESC.init([0; 256]),
        BOS_DESC.init([0; 256]),
        &mut [],
        CONTROL_BUF.init([0; 64]),
    );

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

    // ---- matrix pins ----
    #[cfg(not(feature = "proto"))]
    let rows = [
        Output::new(p.PA9, Level::Low, Speed::Low),
        Output::new(p.PA10, Level::Low, Speed::Low),
        Output::new(p.PB3, Level::Low, Speed::Low),
        Output::new(p.PB8, Level::Low, Speed::Low),
    ];
    #[cfg(feature = "proto")]
    let rows = [
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

            match embassy_futures::select::select(KBD_CH.receive(), CONSUMER_CH.receive()).await {
                embassy_futures::select::Either::First(transition) => {
                    match write_keyboard_transition(&mut kbd_writer, &transition).await {
                        Ok(()) => {}
                        Err(EndpointError::Disabled) => keyboard_needs_resync = true,
                        Err(EndpointError::BufferOverflow) => {
                            warn!("keyboard HID report overflow");
                        }
                    }
                }
                embassy_futures::select::Either::Second(report) => {
                    let _ = consumer_writer.write_serialize(&report).await;
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
                            let ok = keymap::save_to_flash(&mut flash).is_ok();
                            info!(
                                "app: keymap save -> {}",
                                if ok { "flash written" } else { "FLASH ERROR" }
                            );
                            reply[1] = ok as u8;
                        }
                        CMD_FACTORY_RESET if &buf[1..5] == RESET_KEY => {
                            let ok = keymap::factory_reset(&mut flash).is_ok();
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
    join3(usb_fut, pump, updater).await;
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
    let mut enc_sw_last = enc_sw.is_high();
    let mut joy_sw_last = joy_sw.is_high();

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
                        if pos >= 0 {
                            let pos = pos as usize;
                            info!(
                                "key p{=usize} {} (r{=usize} c{=usize})",
                                pos,
                                if raw { "DOWN" } else { "UP" },
                                ri,
                                ci
                            );
                            set_held(pos, raw);
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
        if e != enc_sw_last {
            info!("encoder switch {}", if e { "UP" } else { "DOWN" });
            set_held(keymap::SLOT_ENC_PRESS, !e);
            post_event(2, !e as u8, 0);
        }
        enc_sw_last = e;

        let j = joy_sw.is_high();
        if j != joy_sw_last {
            info!("joystick switch {}", if j { "UP" } else { "DOWN" });
            set_held(keymap::SLOT_JOY_PRESS, !j);
            post_event(3, 4, !j as u8);
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
            tap_slot(keymap::SLOT_ENC_CW);
            post_event(1, 1, 0);
        }
        while accum <= -ENC_COUNTS_PER_DETENT {
            accum += ENC_COUNTS_PER_DETENT;
            info!("encoder CCW");
            tap_slot(keymap::SLOT_ENC_CCW);
            post_event(1, 0, 0);
        }
    }
}

/// Joystick: 50 Hz ADC poll; deflection past the (configurable) threshold
/// holds that direction's slot until the stick returns to centre.
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
    let mut last: u8 = DIR_NONE;
    let mut n: u32 = 0;
    loop {
        ticker.next().await;
        let x = adc.read(&mut joy_x).await;
        let y = adc.read(&mut joy_y).await;
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
        if dir != last {
            info!("joystick dir {=u8} (x={=u16} y={=u16})", dir, x, y);
            const DIR_SLOTS: [usize; 4] = [
                keymap::SLOT_JOY_UP,
                keymap::SLOT_JOY_DOWN,
                keymap::SLOT_JOY_LEFT,
                keymap::SLOT_JOY_RIGHT,
            ];
            if last != DIR_NONE {
                set_held(DIR_SLOTS[last as usize], false);
                post_event(3, last, 0);
            }
            if dir != DIR_NONE {
                set_held(DIR_SLOTS[dir as usize], true);
                post_event(3, dir, 1);
            }
            last = dir;
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
            tap_slot(keymap::SLOT_TOUCH_TAP);
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
async fn led_task(mut led_key: Output<'static>, mut led_ug: Output<'static>) {
    info!("led_task: entered");
    let mut ticker = Ticker::every(Duration::from_millis(33));
    let mut phase: u8 = 0;
    loop {
        ticker.next().await;
        phase = phase.wrapping_add(1);
        let state = KEYSTATE.load(core::sync::atomic::Ordering::Relaxed);
        // ~8.5 s heartbeat — cheap proof the executor is still scheduling.
        if phase == 0 {
            debug!("led: alive, keystate=0x{=u16:04x}", state);
        }

        let mut keys = [ws2812::Grb::default(); 13];
        for (i, px) in keys.iter_mut().enumerate() {
            // key_leds[i] sits under key position i's switch — chain order is
            // the sw index order in the .cohdl, and every position is
            // independent now (the 2U pair each has its own LED and bit).
            *px = if state & (1 << i) != 0 {
                ws2812::Grb::rgb(255, 255, 255).scaled(24)
            } else {
                hue(phase.wrapping_add((i as u8) * 20)).scaled(6)
            };
        }
        // The hue step is 256/UG_LEN so the ring carries exactly one full
        // wheel around the board regardless of revision.
        let mut ring = [ws2812::Grb::default(); UG_LEN];
        for (i, px) in ring.iter_mut().enumerate() {
            *px = hue(phase.wrapping_add((i as u8) * (256 / UG_LEN) as u8)).scaled(8);
        }
        ws2812::write(&mut led_key, &keys);
        ws2812::write(&mut led_ug, &ring);
    }
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
