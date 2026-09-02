//! Codex Micro compatibility mode — opt-in, off by default (see
//! `keymap::MODE_CODEX` and the boot chord in `main.rs`).
//!
//! When the pad boots in this mode it enumerates with the USB identity of
//! OpenAI's Codex Micro macropad (built by Work Louder) and adds a
//! vendor-defined HID interface carrying the JSON-RPC-style protocol that
//! ChatGPT Desktop speaks to that device. The desktop app then drives the pad
//! natively: the 13 keys report as the six Agent Keys plus the seven Command
//! Keys, the encoder as the dial, the stick as the analog stick — and the
//! host pushes the six agent status lights and its ambient/key lighting
//! configuration back down for the LEDs.
//!
//! The protocol is undocumented. This module is an independent Rust
//! re-implementation of the behaviour observed, documented and validated
//! (over BLE) by two MIT-licensed reference projects:
//! `imliubo/codex-micro-4-core2` (Copyright (c) 2026 imliubo) and
//! `digitsisyph/codex-micro-stopwatch`. Their protocol notes are the spec;
//! none of their code is copied. The identifiers used here (VID/PID, device
//! strings, usage page, report ID, key IDs, method names) are not ours: they
//! are emitted solely so a compatible host recognises the device, and imply
//! no affiliation with, or endorsement by, OpenAI or Work Louder.
//!
//! ## Wire format
//!
//! One HID application collection (usage page `0xFF00`, usage 1) with a
//! 63-byte Input and a 63-byte Output report under Report ID 6, so every
//! report is 64 bytes on the wire:
//!
//! ```text
//! [0x06 report id][0x02 type][len 0..=61][UTF-8 JSON fragment][zero pad]
//! ```
//!
//! Device -> host messages are newline-terminated JSON split into 61-byte
//! fragments. Host -> device fragments are accumulated until they form one
//! complete top-level JSON object; a fragment that starts a fresh
//! `{"method"` resynchronises a stale partial buffer, exactly as the
//! references do. Hosts may deliver output reports either on the interrupt
//! OUT endpoint or as a control SET_REPORT (`ControlHandler`); the parser
//! tolerates both the 64-byte form with the leading report ID and the bare
//! 63-byte body.
//!
//! ## Messages
//!
//! Device -> host events (no `id`):
//! - `{"method":"v.oai.hid","params":{"k":"AG02","act":1,"ag":2}}` — key
//!   press (`act` 1) / release (0) / one encoder step (2). `k` is `AG00`…
//!   `AG05` for the agent keys (with `ag` = index), `ACT06`…`ACT12` for the
//!   command keys, `ENC_CW` / `ENC_CC` for dial steps, `ENC` for the dial
//!   push. Gestures (double-press, 500 ms hold) are interpreted by the host.
//! - `{"method":"v.oai.rad","params":{"a":0.75,"d":1}}` — analog stick:
//!   `a` in normalised turns (right 0, down 0.25, left 0.5, up 0.75), `d` 1
//!   on deflection, 0 on return to centre.
//!
//! Host -> device requests (`id` echoed in the reply):
//! - `sys.version` -> `{"version":…}`
//! - `device.status` -> `{"version":…,"profile_index":0,"layer_index":1,
//!   "battery":100,"is_charging":false}`
//! - `v.oai.thstatus` (params: array of `{"id":0..5,"c":0xRRGGBB,"b":0..1,
//!   "e":"off"|"breath","s":speed}`) -> `{"ok":true}` — the agent lights
//! - `v.oai.rgbcfg` (params: `{"ambient":{c,b,e,s},"keys":{c,b,e,s}}`) ->
//!   `{"ok":true}` — underglow and command-key lighting
//! - `lights.preview`, `host.focused_app` -> `{"ok":true}` (acknowledged)
//! - anything else -> `{"error":{"code":-32601,"message":"Method not found"}}`
//!
//! `wire.rs` holds the codec (pure, host-tested); this file is the USB
//! plumbing around it.

pub mod wire;

use core::cell::RefCell;
use defmt::{debug, info, warn};
use embassy_futures::select::{select3, Either3};
use embassy_sync::blocking_mutex::raw::ThreadModeRawMutex;
use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::channel::Channel;
use embassy_time::{with_timeout, Duration};
use embassy_usb::class::hid::{HidReader, HidWriter, ReadError, ReportId, RequestHandler};
use embassy_usb::control::OutResponse;
use embassy_usb::driver::Driver;
use static_cell::StaticCell;

pub use wire::{
    Event, Key, Light, Lights, ACT_PRESS, ACT_RELEASE, ACT_STEP, REPORT_DESC, REPORT_LEN,
};
use wire::{Handled, Push, Reassembler, MSG_TYPE, REPORT_ID, TX_CAP};

// ---- identity ---------------------------------------------------------------

/// USB identity the host matches on. Not ours — see the module docs.
pub const VID: u16 = 0x303A;
pub const PID: u16 = 0x8360;
pub const MANUFACTURER: &str = "Work Louder";
pub const PRODUCT: &str = "Codex Micro";
/// bcdDevice. Confirmed against the Codex desktop app's device kit
/// (2026-09-02, Codex 26.825): it treats the connection as USB when the
/// HID transport reports `usb`, or when the transport is unknown and
/// `release % 4 == 0`; the BLE emulators' `0x0101` is what marks a pad as
/// wireless. Keep the low two bits clear.
pub const DEVICE_RELEASE: u16 = 0x0100;
/// What `sys.version` / `device.status` report. Suffixed so a host log can
/// tell this pad from the real hardware and from the BLE emulators.
pub const VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), "-openmicro");

// ---- device -> host events --------------------------------------------------

/// Outbound events. On overflow the OLDEST is evicted: a dropped release
/// would strand a key on the host, a dropped old press only costs a click.
static EVENT_CH: Channel<ThreadModeRawMutex, Event, 16> = Channel::new();

pub fn post(ev: Event) {
    if EVENT_CH.try_send(ev).is_err() {
        let _ = EVENT_CH.try_receive();
        let _ = EVENT_CH.try_send(ev);
    }
}

pub fn key(position: u8, pressed: bool) {
    post(Event::Key {
        key: Key::Position(position),
        act: if pressed { ACT_PRESS } else { ACT_RELEASE },
    });
}

pub fn encoder_step(cw: bool) {
    post(Event::Key {
        key: if cw { Key::EncCw } else { Key::EncCcw },
        act: ACT_STEP,
    });
}

pub fn encoder_press(pressed: bool) {
    post(Event::Key {
        key: Key::EncPress,
        act: if pressed { ACT_PRESS } else { ACT_RELEASE },
    });
}

pub fn stick(dir: u8, pressed: bool) {
    post(Event::Stick { dir, pressed });
}

// ---- host -> device lighting state -----------------------------------------

static LIGHTS: Mutex<ThreadModeRawMutex, RefCell<Lights>> = Mutex::new(RefCell::new(Lights::OFF));

/// Snapshot for the LED renderer.
pub fn lights() -> Lights {
    LIGHTS.lock(|l| *l.borrow())
}

/// Forget everything the host said (USB reset / re-enumeration): the host
/// re-sends its lighting state when it reconnects.
pub fn clear_lights() {
    LIGHTS.lock(|l| *l.borrow_mut() = Lights::OFF);
}

// ---- control-request path ---------------------------------------------------

/// Output reports the host sends as control SET_REPORT (rather than on the
/// interrupt OUT endpoint) land here and are drained by `pump`.
static CTRL_RX: Channel<ThreadModeRawMutex, ([u8; REPORT_LEN], u8), 4> = Channel::new();

pub struct ControlHandler;

/// `Config::request_handler` wants a `&'static mut`; `main` inits this once.
pub static CONTROL: StaticCell<ControlHandler> = StaticCell::new();

impl RequestHandler for ControlHandler {
    fn get_report(&mut self, id: ReportId, buf: &mut [u8]) -> Option<usize> {
        // An empty frame: nothing pending. Keeps a host that GET_REPORTs at
        // open time happy without inventing data.
        if id == ReportId::In(REPORT_ID) && buf.len() >= REPORT_LEN {
            buf[..REPORT_LEN].fill(0);
            buf[0] = REPORT_ID;
            buf[1] = MSG_TYPE;
            Some(REPORT_LEN)
        } else {
            None
        }
    }

    fn set_report(&mut self, id: ReportId, data: &[u8]) -> OutResponse {
        if id != ReportId::Out(REPORT_ID) {
            return OutResponse::Rejected;
        }
        let n = data.len().min(REPORT_LEN);
        let mut pkt = [0u8; REPORT_LEN];
        pkt[..n].copy_from_slice(&data[..n]);
        // Full queue: the host is outrunning us mid-message; the
        // `{"method"` resync recovers the next request.
        let _ = CTRL_RX.try_send((pkt, n as u8));
        OutResponse::Accepted
    }
}

// ---- USB pump ---------------------------------------------------------------

/// Newline-terminate `json`, split it into 61-byte fragments and send each
/// as one 64-byte report. A stalled endpoint (host not draining) gives up
/// after a short timeout rather than wedging the pump; the host discards
/// the truncated line and the next message starts clean.
async fn send_json<'d, D: Driver<'d>>(writer: &mut HidWriter<'d, D, REPORT_LEN>, json: &[u8]) {
    let mut rep = [0u8; REPORT_LEN];
    let mut off = 0;
    loop {
        let chunk = wire::frame(json, off, &mut rep);
        if chunk == 0 {
            return;
        }
        match with_timeout(Duration::from_millis(250), writer.write(&rep)).await {
            Ok(Ok(())) => {}
            _ => return,
        }
        off += chunk;
    }
}

/// The reassembly and scratch buffers live in .bss, not in the pump's
/// future: every task future is carved out of the executor's fixed 4 KiB
/// arena, and 1.3 KB of locals there would be most of its headroom.
static RX: StaticCell<Reassembler> = StaticCell::new();
static TX: StaticCell<[u8; TX_CAP]> = StaticCell::new();

/// Serve the compat interface forever: host requests (interrupt OUT or
/// control SET_REPORT) in, replies and input events out.
pub async fn pump<'d, D: Driver<'d>>(
    reader: &mut HidReader<'d, D, REPORT_LEN>,
    writer: &mut HidWriter<'d, D, REPORT_LEN>,
) -> ! {
    info!("codex: compat interface up ({})", VERSION);
    let rx = RX.init_with(Reassembler::new);
    let tx = TX.init_with(|| [0u8; TX_CAP]);
    let mut pkt = [0u8; REPORT_LEN];
    loop {
        let fragment: Option<([u8; REPORT_LEN], usize)> =
            match select3(reader.read(&mut pkt), CTRL_RX.receive(), EVENT_CH.receive()).await {
                Either3::First(Ok(n)) => Some((pkt, n)),
                Either3::First(Err(ReadError::Disabled)) => {
                    // Not configured (yet, or any more): nothing the host
                    // said still applies. Wait for the endpoint to come
                    // back rather than spinning.
                    rx.clear();
                    clear_lights();
                    reader.ready().await;
                    None
                }
                Either3::First(Err(_)) => None,
                Either3::Second((data, n)) => Some((data, n as usize)),
                Either3::Third(ev) => {
                    let n = wire::event_json(ev, tx);
                    if n > 0 {
                        send_json(writer, &tx[..n]).await;
                    }
                    None
                }
            };
        let Some((data, n)) = fragment else {
            continue;
        };
        match rx.push(&data[..n]) {
            Push::Pending => {}
            Push::Dropped => warn!("codex: oversized request dropped"),
            Push::Complete(len) => {
                let (what, n) = LIGHTS
                    .lock(|l| wire::handle_request(rx.data(len), VERSION, &mut l.borrow_mut(), tx));
                rx.clear();
                match what {
                    Handled::DeviceStatus => info!("codex: device.status"),
                    Handled::Unknown => debug!("codex: unknown method"),
                    _ => {}
                }
                if n > 0 {
                    send_json(writer, &tx[..n]).await;
                }
            }
        }
    }
}
