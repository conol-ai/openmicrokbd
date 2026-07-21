//! OpenMicro macropad firmware — STM32F072CBT6 + embassy.
//!
//! Pin map is the CoHDL design's (examples/openmicro/src/openmicro_parts.cohdl,
//! the position-aware GPIO assignment) — if the .cohdl changes, this table is
//! the one to update:
//!
//!   ROW0..3  = PA9  PB3  PB6  PB5   (outputs, drive high per scan step)
//!   COL0..3  = PB8  PB7  PA15 PA10  (inputs, pull-down; diode cathode -> COL)
//!   ENC_A/B  = PC13 PC14 (quadrature, pull-up, common to GND)
//!   ENC_SW   = PC15      (pull-up, active low)
//!   JOY_X/Y  = PB1/ADC_IN9  PB0/ADC_IN8
//!   JOY_SW   = PA8       (pull-up, active low)
//!   TOUCH    = PB9       (RC charge-time sensing, no external R)
//!   LED_KEY  = PB4       (13x SK6812MINI-E per-key chain)
//!   LED_UG   = PA0       (16x SK6812MINI-E underglow ring)
//!   USB      = PA11/PA12 (FS device; HSI48 + CRS, crystal not required)
//!
//! HID map: 13 keys -> F13..F24 (the 2U cap's two switches, sw10+sw11, share
//! F23); encoder -> volume +/- ; encoder push -> mute; touch -> play/pause;
//! joystick -> arrow keys, push -> enter.
//!
//! A third, vendor-defined HID interface (usage page 0xFF60) carries the
//! updater protocol: the host app can query the firmware version and command
//! a reboot into the ROM DFU bootloader (see dfu.rs and README.md).

#![no_std]
#![no_main]

mod dfu;
mod ws2812;

use embassy_executor::Spawner;
use embassy_futures::join::join3;
use embassy_stm32::adc::Adc;
use embassy_stm32::gpio::{Flex, Input, Level, Output, Pull, Speed};
use embassy_stm32::rcc::{Hsi48Config, Sysclk};
use embassy_stm32::usb::Driver;
use embassy_stm32::{bind_interrupts, peripherals, usb, Config};
use embassy_sync::blocking_mutex::raw::ThreadModeRawMutex;
use embassy_sync::channel::Channel;
use embassy_time::{Duration, Ticker, Timer};
use embassy_usb::class::hid::{HidReaderWriter, State};
use panic_halt as _;
use static_cell::StaticCell;
use usbd_hid::descriptor::{KeyboardReport, MediaKeyboardReport, SerializedDescriptor};

bind_interrupts!(struct Irqs {
    USB => usb::InterruptHandler<peripherals::USB>;
    ADC1_COMP => embassy_stm32::adc::InterruptHandler<peripherals::ADC1>;
});

// F13..F24 HID keyboard usages.
const KC_F13: u8 = 0x68;

/// keymap[row][col] -> keycode (0 = no key at that matrix position).
/// sw10 and sw11 sit under one 2U keycap and share F23.
const KEYMAP: [[u8; 4]; 4] = [
    [0, KC_F13, KC_F13 + 1, 0],                       // R0: -,  k1,  k2,  -
    [KC_F13 + 2, KC_F13 + 3, KC_F13 + 4, KC_F13 + 5], // R1: k3..k6
    [KC_F13 + 6, KC_F13 + 7, KC_F13 + 8, KC_F13 + 9], // R2: k7..k10
    [0, KC_F13 + 10, KC_F13 + 10, KC_F13 + 11],       // R3: -, k11, k11, k13
];

const KC_RIGHT: u8 = 0x4F;
const KC_LEFT: u8 = 0x50;
const KC_DOWN: u8 = 0x51;
const KC_UP: u8 = 0x52;
const KC_ENTER: u8 = 0x28;

const USAGE_MUTE: u16 = 0xE2;
const USAGE_VOL_UP: u16 = 0xE9;
const USAGE_VOL_DOWN: u16 = 0xEA;
const USAGE_PLAY_PAUSE: u16 = 0xCD;

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

// Updater protocol, one command per 32-byte OUT report:
//   [0x01, ...]                -> reply [0x01, len, version ascii...]
//   [0x02, 'D', 'F', 'U', '!'] -> ack [0x02, 0x01], reboot into ROM DFU
const CMD_VERSION: u8 = 0x01;
const CMD_ENTER_DFU: u8 = 0x02;
const ENTER_DFU_KEY: &[u8; 4] = b"DFU!";

static KBD_CH: Channel<ThreadModeRawMutex, KeyboardReport, 4> = Channel::new();
static CONSUMER_CH: Channel<ThreadModeRawMutex, MediaKeyboardReport, 4> = Channel::new();
/// Bit i = logical key i pressed — drives the per-key LED effect.
static KEYSTATE: core::sync::atomic::AtomicU16 = core::sync::atomic::AtomicU16::new(0);

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
    let rows = [
        Output::new(p.PA9, Level::Low, Speed::Low),
        Output::new(p.PB3, Level::Low, Speed::Low),
        Output::new(p.PB6, Level::Low, Speed::Low),
        Output::new(p.PB5, Level::Low, Speed::Low),
    ];
    let cols = [
        Input::new(p.PB8, Pull::Down),
        Input::new(p.PB7, Pull::Down),
        Input::new(p.PA15, Pull::Down),
        Input::new(p.PA10, Pull::Down),
    ];

    // ---- encoder + buttons ----
    let enc_a = Input::new(p.PC13, Pull::Up);
    let enc_b = Input::new(p.PC14, Pull::Up);
    let enc_sw = Input::new(p.PC15, Pull::Up);
    let joy_sw = Input::new(p.PA8, Pull::Up);

    // ---- joystick ADC ----
    let adc = Adc::new(p.ADC1, Irqs);
    let joy_x = p.PB1;
    let joy_y = p.PB0;

    // ---- touch (RC charge-time on PB9) ----
    let touch = Flex::new(p.PB9);

    // ---- LED chains ----
    let led_key = Output::new(p.PB4, Level::Low, Speed::VeryHigh);
    let led_ug = Output::new(p.PA0, Level::Low, Speed::VeryHigh);

    spawner.must_spawn(scan_task(rows, cols, enc_a, enc_b, enc_sw, joy_sw));
    spawner.must_spawn(adc_task(adc, joy_x, joy_y));
    spawner.must_spawn(touch_task(touch));
    spawner.must_spawn(led_task(led_key, led_ug));

    // USB device + report pumps + updater channel run forever on this task.
    let usb_fut = usb_dev.run();
    let pump = async {
        loop {
            match embassy_futures::select::select(KBD_CH.receive(), CONSUMER_CH.receive()).await {
                embassy_futures::select::Either::First(report) => {
                    let _ = kbd_writer.write_serialize(&report).await;
                }
                embassy_futures::select::Either::Second(report) => {
                    let _ = consumer_writer.write_serialize(&report).await;
                }
            }
        }
    };
    let updater = async {
        let mut buf = [0u8; 32];
        loop {
            let Ok(_) = raw_reader.read(&mut buf).await else {
                continue;
            };
            match buf[0] {
                CMD_VERSION => {
                    let mut reply = [0u8; 32];
                    reply[0] = CMD_VERSION;
                    reply[1] = FW_VERSION.len() as u8;
                    reply[2..2 + FW_VERSION.len()].copy_from_slice(FW_VERSION.as_bytes());
                    let _ = raw_writer.write(&reply).await;
                }
                CMD_ENTER_DFU if &buf[1..5] == ENTER_DFU_KEY => {
                    let mut reply = [0u8; 32];
                    reply[0] = CMD_ENTER_DFU;
                    reply[1] = 0x01;
                    let _ = raw_writer.write(&reply).await;
                    // Let the ack reach the host before dropping off the bus.
                    Timer::after_millis(50).await;
                    dfu::reboot_into_bootloader();
                }
                _ => {}
            }
        }
    };
    join3(usb_fut, pump, updater).await;
}

/// 1 kHz matrix scan with 5 ms debounce + encoder quadrature + buttons.
#[embassy_executor::task]
async fn scan_task(
    mut rows: [Output<'static>; 4],
    cols: [Input<'static>; 4],
    enc_a: Input<'static>,
    enc_b: Input<'static>,
    enc_sw: Input<'static>,
    joy_sw: Input<'static>,
) {
    let mut ticker = Ticker::every(Duration::from_millis(1));
    let mut debounce = [[0u8; 4]; 4];
    let mut pressed = [[false; 4]; 4];
    let mut last_report = KeyboardReport {
        modifier: 0,
        reserved: 0,
        leds: 0,
        keycodes: [0; 6],
    };
    let mut enc_last = (enc_a.is_high(), enc_b.is_high());
    let mut enc_sw_last = enc_sw.is_high();
    let mut joy_sw_last = joy_sw.is_high();

    loop {
        ticker.next().await;

        // -- matrix --
        let mut changed = false;
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
                        changed = true;
                    }
                }
            }
            row.set_low();
        }

        if changed {
            let mut report = KeyboardReport {
                modifier: 0,
                reserved: 0,
                leds: 0,
                keycodes: [0; 6],
            };
            let mut n = 0;
            let mut state: u16 = 0;
            for ri in 0..4 {
                for ci in 0..4 {
                    if pressed[ri][ci] {
                        let kc = KEYMAP[ri][ci];
                        if kc != 0 {
                            state |= 1 << (kc - KC_F13);
                            if n < 6 && !report.keycodes[..n].contains(&kc) {
                                report.keycodes[n] = kc;
                                n += 1;
                            }
                        }
                    }
                }
            }
            KEYSTATE.store(state, core::sync::atomic::Ordering::Relaxed);
            if report.keycodes != last_report.keycodes {
                last_report = report;
                let _ = KBD_CH.try_send(last_report);
            }
        }

        // -- encoder (quadrature, one detent per full cycle) --
        let now = (enc_a.is_high(), enc_b.is_high());
        if now.0 != enc_last.0 && now.0 == now.1 {
            let usage = if now.1 != enc_last.1 {
                USAGE_VOL_UP
            } else {
                USAGE_VOL_DOWN
            };
            let _ = CONSUMER_CH.try_send(MediaKeyboardReport { usage_id: usage });
            let _ = CONSUMER_CH.try_send(MediaKeyboardReport { usage_id: 0 });
        }
        enc_last = now;

        // -- encoder push -> mute, joystick push -> enter --
        let e = enc_sw.is_high();
        if !e && enc_sw_last {
            let _ = CONSUMER_CH.try_send(MediaKeyboardReport {
                usage_id: USAGE_MUTE,
            });
            let _ = CONSUMER_CH.try_send(MediaKeyboardReport { usage_id: 0 });
        }
        enc_sw_last = e;

        let j = joy_sw.is_high();
        if j != joy_sw_last {
            let mut r = last_report;
            if !j {
                if let Some(slot) = r.keycodes.iter_mut().find(|k| **k == 0) {
                    *slot = KC_ENTER;
                }
            }
            let _ = KBD_CH.try_send(r);
        }
        joy_sw_last = j;
    }
}

/// Joystick: 50 Hz ADC poll, thresholds -> arrow keys.
#[embassy_executor::task]
async fn adc_task(
    mut adc: Adc<'static, peripherals::ADC1>,
    mut joy_x: peripherals::PB1,
    mut joy_y: peripherals::PB0,
) {
    let mut ticker = Ticker::every(Duration::from_millis(20));
    let mut last: u8 = 0;
    loop {
        ticker.next().await;
        let x = adc.read(&mut joy_x).await;
        let y = adc.read(&mut joy_y).await;
        // 12-bit, centre ~2048, generous dead zone.
        let mut dir = 0u8;
        if x < 1024 {
            dir = KC_LEFT;
        } else if x > 3072 {
            dir = KC_RIGHT;
        } else if y < 1024 {
            dir = KC_UP;
        } else if y > 3072 {
            dir = KC_DOWN;
        }
        if dir != last {
            let report = KeyboardReport {
                modifier: 0,
                reserved: 0,
                leds: 0,
                keycodes: [dir, 0, 0, 0, 0, 0],
            };
            let _ = KBD_CH.try_send(report);
            last = dir;
        }
    }
}

/// Cap-touch on PB9 by RC charge time: discharge the pad, release with the
/// internal pull-up, count how long the input takes to read high. A finger
/// adds capacitance -> longer rise. Self-calibrates a floating baseline.
#[embassy_executor::task]
async fn touch_task(mut pad: Flex<'static>) {
    let mut ticker = Ticker::every(Duration::from_millis(30));
    let mut baseline: u32 = 0;
    let mut armed = true;
    loop {
        ticker.next().await;
        // discharge
        pad.set_as_output(Speed::Low);
        pad.set_low();
        Timer::after_micros(50).await;
        // charge through the internal pull-up, count polls until high
        pad.set_as_input(Pull::Up);
        let mut t: u32 = 0;
        while pad.is_low() && t < 10_000 {
            t += 1;
        }
        // exponential baseline of the untouched pad
        if baseline == 0 {
            baseline = t;
        }
        let touched = t > baseline + baseline / 2 + 8;
        if !touched {
            baseline = (baseline * 15 + t) / 16;
        }
        if touched && armed {
            armed = false;
            let _ = CONSUMER_CH.try_send(MediaKeyboardReport {
                usage_id: USAGE_PLAY_PAUSE,
            });
            let _ = CONSUMER_CH.try_send(MediaKeyboardReport { usage_id: 0 });
        } else if !touched {
            armed = true;
        }
    }
}

/// 30 Hz LED renderer: pressed keys light white, idle keys a dim rainbow;
/// the underglow ring runs a slow hue rotation. Brightness is capped to stay
/// inside the 500 mA VBUS budget.
#[embassy_executor::task]
async fn led_task(mut led_key: Output<'static>, mut led_ug: Output<'static>) {
    let mut ticker = Ticker::every(Duration::from_millis(33));
    let mut phase: u8 = 0;
    loop {
        ticker.next().await;
        phase = phase.wrapping_add(1);
        let state = KEYSTATE.load(core::sync::atomic::Ordering::Relaxed);

        let mut keys = [ws2812::Grb::default(); 13];
        for (i, px) in keys.iter_mut().enumerate() {
            // key_leds[i] sits under logical key i's switch (chain order is
            // the sw index order in the .cohdl).
            let logical = KEYMAP_CHAIN[i];
            *px = if state & (1 << logical) != 0 {
                ws2812::Grb::rgb(255, 255, 255).scaled(24)
            } else {
                hue(phase.wrapping_add((i as u8) * 20)).scaled(6)
            };
        }
        let mut ring = [ws2812::Grb::default(); 16];
        for (i, px) in ring.iter_mut().enumerate() {
            *px = hue(phase.wrapping_add((i as u8) * 16)).scaled(8);
        }
        ws2812::write(&mut led_key, &keys);
        ws2812::write(&mut led_ug, &ring);
    }
}

/// key_leds chain index -> logical key bit (sw index folded to keycode bit).
const KEYMAP_CHAIN: [u8; 13] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 10, 11];

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
