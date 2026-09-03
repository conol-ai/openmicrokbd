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
//! The same interface serves Work Louder's own Input app, which configures
//! the device through a file-system RPC: it reads and writes `keymap.json`
//! (profiles → layers → keycodes per key) and `smart_actions.json`, and the
//! pad runs what those files say (`layout.rs`). Keys bound to `KV_OAI_*`
//! keycodes are the Codex Micro controls; `KC_*` keycodes type as USB HID;
//! layer/profile keys switch layouts; macros play key sequences; smart
//! actions are handed to the Input app as `kb.sa.*` notifications.
//!
//! The protocol is undocumented. This module is an independent Rust
//! re-implementation of the behaviour observed, documented and validated
//! (over BLE) by two MIT-licensed reference projects:
//! `imliubo/codex-micro-4-core2` (Copyright (c) 2026 imliubo) and
//! `digitsisyph/codex-micro-stopwatch`, plus what Work Louder's device kit
//! (bundled inside the Codex and Input apps) expects. None of their code is
//! copied. The identifiers used here (VID/PID, device strings, usage page,
//! report ID, key IDs, method names) are not ours: they are emitted solely
//! so a compatible host recognises the device, and imply no affiliation
//! with, or endorsement by, OpenAI or Work Louder.
//!
//! ## Wire format
//!
//! One HID application collection (usage page `0xFF00`, usage 1) with a
//! 63-byte Input and a 63-byte Output report under Report ID 6, so every
//! report is 64 bytes on the wire:
//!
//! ```text
//! [0x06 report id][0x02 channel][len 0..=61][UTF-8 JSON fragment][zero pad]
//! ```
//!
//! Device -> host messages are newline-terminated JSON split into 61-byte
//! fragments. Host -> device fragments are accumulated until they form one
//! complete top-level JSON object; a fragment that starts a fresh
//! `{"method"` resynchronises a stale partial buffer, exactly as the
//! references do. Hosts may deliver output reports either on the interrupt
//! OUT endpoint or as a control SET_REPORT (`ControlHandler`); the parser
//! tolerates both the 64-byte form with the leading report ID and the bare
//! 63-byte body. File chunks (`fs.writebin`) are streamed straight into
//! flash as they arrive instead of being buffered.
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
//! - `kb.sa.inserttext|exec|openapp|openurl` with the smart action's
//!   payload, `kb.cs.show|hide|toggle {l,p}`, `kb.radial {a,d,l,p,o}` — for
//!   the Input app.
//!
//! Host -> device requests (`id` echoed in the reply):
//! - `sys.version` -> `{"version":…}`
//! - `device.status` -> `{"version":…,"profile_index":n,"layer_index":n,
//!   "battery":100,"is_charging":false}`
//! - `v.oai.thstatus` (params: array of `{"id":0..5,"c":0xRRGGBB,"b":0..1,
//!   "e":effect,"s":0..1,"sk":0|1,"sa":0|1}`) -> `{"ok":true}` — the agent
//!   lights. `e` is the device kit's numeric effect: 0 off, 1 solid,
//!   2 snake, 3 rainbow, 4 breath, 5 gradient, 6 shallowBreath (the BLE
//!   emulators' probes send the names; both parse). `s` is the animation
//!   speed, 0 stopped. Fields left out keep their previous value.
//! - `v.oai.rgbcfg` (params: `{"ambient":{e,b,s,m,c},"keys":{e,b,s,m,c}}`)
//!   -> `{"ok":true}` — underglow and command-key lighting, same fields
//!   plus an unused `m` (magic)
//! - `lights.preview` (`{"backlight":{effect,brightness,speed,magic,color},
//!   "underglow":{…}}`, effect names) -> `{"ok":true}`
//! - `fs.list` -> `[{"name","size","checksum"}]` (SHA-1), `fs.readbin
//!   {file,offset,len}` -> `{"total_size","data"}` (base64), `fs.writebin
//!   {file,data,append,completed,offset}` -> `{"data_written"}`, `fs.read` /
//!   `fs.write {file,data}` (whole JSON), `fs.delete`, `fs.rmdir`,
//!   `fs.txbegin` -> `{"tx"}`, `fs.txcommit`
//! - `host.focused_app`, `sys.selftest`, `ui.*`, `appmgr.*`, `mp.*` ->
//!   acknowledged (empty lists for the app manager)
//! - anything else -> `{"error":{"code":-32601,"message":"Method not found"}}`
//!
//! `wire.rs` holds the codec, `layout.rs` the keymap engine, `files.rs` the
//! flash file store, `sha1.rs` the checksum (all host-tested); this file is
//! the USB plumbing around them.

pub mod files;
pub mod layout;
pub mod sha1;
pub mod wire;

use core::cell::RefCell;
use defmt::{debug, info, warn};
use embassy_futures::select::{select3, Either3};
use embassy_sync::blocking_mutex::raw::ThreadModeRawMutex;
use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::channel::Channel;
use embassy_time::{with_timeout, Duration, Timer};
use embassy_usb::class::hid::{HidReader, HidWriter, ReadError, ReportId, RequestHandler};
use embassy_usb::control::OutResponse;
use embassy_usb::driver::Driver;
use static_cell::StaticCell;

use files::FlashStore;
pub use layout::{Binding, Joystick, Layout};
use wire::{
    Ctx, FileStore, Handled, Push, Reassembler, Reply, Status, WriteSink, WriteState, MSG_TYPE,
    REPORT_ID, TX_CAP,
};
pub use wire::{
    Event, Key, Light, Lights, ACT_PRESS, ACT_RELEASE, ACT_STEP, REPORT_DESC, REPORT_LEN,
};

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

/// A Codex Micro control by `layout` id (key position, or `OAI_ENC_*`).
pub fn oai(control: u8, pressed: bool) {
    let act = if pressed { ACT_PRESS } else { ACT_RELEASE };
    match control {
        layout::OAI_ENC_CCW => {
            if pressed {
                post(Event::Key {
                    key: Key::EncCcw,
                    act: ACT_STEP,
                })
            }
        }
        layout::OAI_ENC_CW => {
            if pressed {
                post(Event::Key {
                    key: Key::EncCw,
                    act: ACT_STEP,
                })
            }
        }
        layout::OAI_ENC_PRESS => post(Event::Key {
            key: Key::EncPress,
            act,
        }),
        p if (p as usize) < layout::KEYS => post(Event::Key {
            key: Key::Position(p),
            act,
        }),
        _ => {}
    }
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

// ---- the keymap engine ------------------------------------------------------

struct Engine {
    layout: Layout,
    /// Profile picked with a `KI_PS<n>` key; None follows the file's
    /// `activeProfileId`.
    profile: Option<u8>,
    layer: u8,
    /// Layer to return to when a momentary layer key is released.
    held_from: Option<u8>,
}

static ENGINE: Mutex<ThreadModeRawMutex, RefCell<Engine>> = Mutex::new(RefCell::new(Engine {
    layout: Layout {
        keys: [Binding::None; layout::KEYS],
        encoder: [Binding::None; 3],
        touch: Binding::None,
        joystick: Joystick::None,
        sectors: [layout::Sector {
            binding: Binding::None,
            a1: 0,
            a2: 0,
        }; layout::MAX_SECTORS],
        sector_count: 0,
        profile_index: 0,
        layer_index: 0,
        profile_count: 0,
        layer_count: 0,
    },
    profile: None,
    layer: 0,
    held_from: None,
}));

fn keymap_doc() -> &'static [u8] {
    files::read(b"keymap.json").unwrap_or(layout::DEFAULT_KEYMAP.as_bytes())
}

/// Re-read `keymap.json` for the current profile/layer, and apply that
/// layer's lighting if it defines any.
pub fn reload() {
    let doc = keymap_doc();
    let (profile, layer) = ENGINE.lock(|e| {
        let e = e.borrow();
        (e.profile, e.layer)
    });
    let parsed = layout::parse(doc, profile, layer);
    ENGINE.lock(|e| {
        let mut e = e.borrow_mut();
        match parsed {
            Some(l) => {
                e.layer = l.layer_index;
                e.layout = l;
            }
            None => {
                e.layout = Layout::default();
                e.layer = 0;
            }
        }
    });
    if let Some(lights) = layout::layer_lights(doc, profile, layer) {
        LIGHTS.lock(|l| {
            let mut l = l.borrow_mut();
            if let Some(b) = wire::find_key(lights, b"backlight").filter(|v| wire::is_object(v)) {
                wire::update_light(&mut l.keys, b);
            }
            if let Some(u) = wire::find_key(lights, b"underglow").filter(|v| wire::is_object(v)) {
                wire::update_light(&mut l.ambient, u);
            }
        });
    }
    let st = status();
    info!(
        "codex: layout profile {=u8} layer {=u8}",
        st.profile_index, st.layer_index
    );
}

/// The active layout (a copy).
#[allow(dead_code)]
pub fn layout() -> Layout {
    ENGINE.lock(|e| e.borrow().layout)
}

pub fn status() -> Status {
    ENGINE.lock(|e| {
        let e = e.borrow();
        Status {
            profile_index: e.layout.profile_index,
            layer_index: e.layout.layer_index,
        }
    })
}

/// `KI_LS<n>`: switch to layer n, or back to the base layer if already
/// there. Layers the profile lacks fall back to the base layer.
pub fn toggle_layer(n: u8) {
    ENGINE.lock(|e| {
        let mut e = e.borrow_mut();
        e.layer = if e.layer == n { 0 } else { n };
        e.held_from = None;
    });
    reload();
}

/// `KI_LM<n>`: layer n while the key is down.
pub fn hold_layer(n: u8, held: bool) {
    ENGINE.lock(|e| {
        let mut e = e.borrow_mut();
        if held {
            if e.held_from.is_none() {
                e.held_from = Some(e.layer);
            }
            e.layer = n;
        } else if let Some(back) = e.held_from.take() {
            e.layer = back;
        }
    });
    reload();
}

/// `KI_PS<n>`: profile n (0-based); its base layer.
pub fn set_profile(n: u8) {
    ENGINE.lock(|e| {
        let mut e = e.borrow_mut();
        e.profile = Some(n);
        e.layer = 0;
        e.held_from = None;
    });
    reload();
}

pub fn key_binding(position: u8) -> Binding {
    ENGINE.lock(|e| {
        e.borrow()
            .layout
            .keys
            .get(position as usize)
            .copied()
            .unwrap_or(Binding::None)
    })
}

/// `layout::ENCODER_CCW` / `ENCODER_CW` / `ENCODER_PRESS`.
pub fn encoder_binding(which: usize) -> Binding {
    ENGINE.lock(|e| e.borrow().layout.encoder[which.min(2)])
}

pub fn touch_binding() -> Binding {
    ENGINE.lock(|e| e.borrow().layout.touch)
}

pub fn joystick_mode() -> Joystick {
    ENGINE.lock(|e| e.borrow().layout.joystick)
}

/// The tap keycode of a multi-action (`KA_M<n>`).
pub fn multi_tap(id: u16) -> Binding {
    layout::multi_tap(keymap_doc(), id).unwrap_or(Binding::None)
}

/// The sector a stick angle (thousandths of a turn) lands in.
pub fn sector(angle_milli: u16) -> Binding {
    ENGINE.lock(|e| {
        let e = e.borrow();
        e.layout.sectors[..e.layout.sector_count as usize]
            .iter()
            .find(|s| s.contains(angle_milli))
            .map(|s| s.binding)
            .unwrap_or(Binding::None)
    })
}

// ---- macros -----------------------------------------------------------------

/// What macro playback needs from the rest of the firmware: press and
/// release a binding on the USB interfaces / Codex channel.
pub trait MacroKeys: Sync {
    fn press(&self, b: Binding);
    fn release(&self, b: Binding);
}

static MACRO_CH: Channel<ThreadModeRawMutex, u16, 4> = Channel::new();

pub fn run_macro(id: u16) {
    let _ = MACRO_CH.try_send(id);
}

const MAX_STEPS: usize = 32;

/// Plays macros as they are queued, one at a time, with their delays.
#[embassy_executor::task]
pub async fn macro_task(keys: &'static dyn MacroKeys) {
    loop {
        let id = MACRO_CH.receive().await;
        let doc = keymap_doc();
        let mut steps = [layout::Step {
            binding: Binding::None,
            delay_ms: 0,
            act: layout::ACT_PRESS,
        }; MAX_STEPS];
        let mut n = 0usize;
        if !layout::macro_steps(doc, id, |s| {
            if n < MAX_STEPS {
                steps[n] = s;
                n += 1;
            }
        }) {
            warn!("codex: macro {=u16} not found", id);
            continue;
        }
        for s in &steps[..n] {
            match s.act {
                layout::ACT_PRESS => keys.press(s.binding),
                layout::ACT_RELEASE => keys.release(s.binding),
                _ => {
                    keys.press(s.binding);
                    Timer::after_millis(12).await;
                    keys.release(s.binding);
                }
            }
            if s.delay_ms > 0 {
                Timer::after_millis(s.delay_ms as u64).await;
            } else {
                Timer::after_millis(8).await;
            }
        }
    }
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

/// Newline-terminate the concatenation of `parts`, split it into 61-byte
/// fragments and send each as one 64-byte report. A stalled endpoint (host
/// not draining) gives up after a short timeout rather than wedging the
/// pump; the host discards the truncated line and the next message starts
/// clean.
async fn send_parts<'d, D: Driver<'d>>(writer: &mut HidWriter<'d, D, REPORT_LEN>, parts: &[&[u8]]) {
    let mut rep = [0u8; REPORT_LEN];
    let mut off = 0;
    loop {
        let chunk = wire::frame_parts(parts, off, &mut rep);
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
/// arena, and 1.7 KB of locals there would be most of its headroom.
static RX: StaticCell<Reassembler> = StaticCell::new();
static TX: StaticCell<[u8; TX_CAP]> = StaticCell::new();

/// Serve the compat interface forever: host requests (interrupt OUT or
/// control SET_REPORT) in, replies, input events and notifications out.
pub async fn pump<'d, D: Driver<'d>>(
    reader: &mut HidReader<'d, D, REPORT_LEN>,
    writer: &mut HidWriter<'d, D, REPORT_LEN>,
    store: &mut FlashStore,
) -> ! {
    info!("codex: compat interface up ({})", VERSION);
    let rx = RX.init_with(Reassembler::new);
    let tx = TX.init_with(|| [0u8; TX_CAP]);
    let mut write = WriteState::new();
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
                    store.abort_write();
                    write.active = false;
                    clear_lights();
                    reader.ready().await;
                    None
                }
                Either3::First(Err(_)) => None,
                Either3::Second((data, n)) => Some((data, n as usize)),
                Either3::Third(Event::Smart(id)) => {
                    match files::read(b"smart_actions.json")
                        .and_then(|doc| layout::smart_action(doc, id))
                    {
                        Some((kind, payload)) => {
                            let n = wire::notify_head(kind.method(), tx);
                            if n > 0 {
                                send_parts(writer, &[&tx[..n], payload, b"}"]).await;
                            }
                        }
                        None => debug!("codex: smart action {=u16} unknown", id),
                    }
                    None
                }
                Either3::Third(ev) => {
                    let n = wire::event_json(ev, tx);
                    if n > 0 {
                        send_parts(writer, &[&tx[..n]]).await;
                    }
                    None
                }
            };
        let Some((data, n)) = fragment else {
            continue;
        };
        let push = {
            let mut sink = WriteSink {
                state: &mut write,
                store,
            };
            rx.push(&data[..n], &mut sink)
        };
        match push {
            Push::Pending => {}
            Push::Dropped => warn!("codex: oversized request dropped"),
            Push::Complete(len) => {
                let status = status();
                let (what, reply) = LIGHTS.lock(|l| {
                    let mut ctx = Ctx {
                        version: VERSION,
                        lights: &mut l.borrow_mut(),
                        store,
                        write: &mut write,
                        status,
                    };
                    wire::handle_request(rx.data(len), &mut ctx, tx)
                });
                rx.clear();
                match what {
                    Handled::DeviceStatus => info!("codex: device.status"),
                    Handled::FileWritten => {
                        info!("codex: file written, reloading layout");
                        reload();
                    }
                    Handled::Unknown => debug!("codex: unknown method"),
                    _ => {}
                }
                match reply {
                    Reply::None => {}
                    Reply::Buf(n) => send_parts(writer, &[&tx[..n]]).await,
                    Reply::File {
                        head,
                        name,
                        name_len,
                        tail,
                    } => {
                        if let Some(body) = files::read(&name[..name_len as usize]) {
                            send_parts(writer, &[&tx[..head], body, tail]).await;
                        }
                    }
                }
            }
        }
    }
}
