//! The device worker thread: owns the process's one HidApi instance, keeps
//! the pad's vendor-HID interface open for as long as it is plugged in,
//! streams input events to the UI, and runs keymap sync + firmware updates
//! end-to-end.
//!
//! Talks the firmware's vendor-HID protocol v2 (../fw/src/main.rs): 32-byte
//! reports where a reply echoes the command byte, and anything with the top
//! bit set ([0x80, src, a, b]) is an unsolicited input event that can arrive
//! at any moment — including between a command and its reply, so the reply
//! reader decodes and forwards those instead of dropping them.
//!
//! Lifecycle: search (~800 ms) → open → post Connected + Keymap → serve
//! commands and pump events → on a read error or a silently vanished device,
//! post Disconnected and go back to searching. Firmware updates (dfuse.rs)
//! open the device themselves, so the held handle is closed first and a full
//! reconnect cycle afterwards re-posts Connected/Keymap.
//!
//! Everything is reported to the UI through the framework-neutral event bus.

use hidapi::{HidApi, HidDevice};
use std::ffi::CString;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crate::config::{
    JoyMode, LedPattern, Slot, SlotKind, DEFAULT_JOY_MOUSE_SPEED, DEFAULT_LED_BRIGHTNESS,
    SLOT_COUNT,
};
use crate::dfuse;
use crate::events;

pub const VID: u16 = 0x1209;
pub const PID: u16 = 0x0001;
const RAW_USAGE_PAGE: u16 = 0xFF60;

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

/// First byte of an unsolicited device->host event report.
const EVENT_MARK: u8 = 0x80;

/// GET/SET_KEYMAP move 7 slots per 32-byte report (3 header + 7*4 bytes).
const PAGE_SLOTS: usize = 7;
const KEYMAP_PAGES: u8 = 4;

/// How often we probe for the pad while disconnected.
const SEARCH_PERIOD: Duration = Duration::from_millis(800);
/// How often we re-check enumeration while connected — hidapi on some
/// platforms keeps returning Ok(0) forever after an unplug instead of erroring.
const PRESENCE_PERIOD: Duration = Duration::from_secs(2);
/// Blocking event-read timeout; doubles as the connected loop's pacing.
const EVENT_READ_MS: i32 = 30;
/// Deadline for an ordinary command reply.
const REPLY_TIMEOUT: Duration = Duration::from_millis(500);
/// SAVE and FACTORY_RESET erase+write flash, which stalls the MCU — allow more.
const SAVE_TIMEOUT: Duration = Duration::from_millis(1500);

/// Posted to the UI: presence, live input events, and keymap traffic.
#[derive(Debug, Clone)]
pub enum DeviceMsg {
    Connected {
        version: String,
        serial: String,
    },
    Disconnected,
    Event(PadEvent),
    Keymap {
        slots: [Slot; SLOT_COUNT],
        joy_threshold: u16,
        joy_mode: JoyMode,
        joy_mouse_speed: u8,
        led_brightness: u8,
        led_key_pattern: LedPattern,
        led_ambient_pattern: LedPattern,
    },
    SyncDone {
        ok: bool,
        detail: String,
    },
}

/// One decoded input event from the pad ([0x80, src, a, b] on the wire).
#[derive(Clone, Copy, Debug)]
pub enum PadEvent {
    Key {
        index: u8,
        pressed: bool,
    },
    Encoder {
        cw: bool,
    },
    EncoderButton {
        pressed: bool,
    },
    /// dir: 0 up, 1 down, 2 left, 3 right, 4 press.
    Joystick {
        dir: u8,
        active: bool,
    },
    Touch,
}

/// Posted to the UI during a firmware update.
#[derive(Debug, Clone)]
pub enum UpdateMsg {
    Phase(String),
    Log(String),
    /// 0.0 ..= 1.0 across the whole erase+program run.
    Progress(f64),
    Done {
        version: String,
    },
    Failed(String),
}

/// UI -> worker commands.
pub enum DeviceCmd {
    StartUpdate {
        image: PathBuf,
        /// Release downloads name the version they are expected to boot.
        /// Manual recovery images leave this unset.
        expected_version: Option<String>,
    },
    EnterDfuOnly,
    SyncKeymap {
        slots: [Slot; SLOT_COUNT],
        joy_threshold: u16,
        joy_mode: JoyMode,
        joy_mouse_speed: u8,
        led_brightness: u8,
        led_key_pattern: LedPattern,
        led_ambient_pattern: LedPattern,
    },
    /// Live slider preview: RAM only, best-effort, no reply posted. The
    /// debounced SyncKeymap that follows is what persists it.
    SetLedBrightness {
        brightness: u8,
    },
    /// Runtime-only LED override.  This deliberately does not call SAVE, so
    /// activity feedback never replaces the user's configured idle pattern.
    SetTransientLedPattern {
        key_pattern: LedPattern,
        ambient_pattern: LedPattern,
    },
    ReadKeymap,
    FactoryReset,
}

pub fn spawn_worker() -> mpsc::Sender<DeviceCmd> {
    let (tx, rx) = mpsc::channel::<DeviceCmd>();
    std::thread::spawn(move || {
        let api = match HidApi::new() {
            Ok(api) => api,
            Err(e) => {
                events::post(UpdateMsg::Failed(format!("HID init failed: {e}")));
                return;
            }
        };
        worker(api, rx);
    });
    tx
}

/// Why a connected session ended; tells the outer loop what to do after it
/// has closed the device handle.
enum SessionEnd {
    /// Read error or the device silently vanished — Disconnected, re-search.
    Lost,
    /// UI asked for a firmware update; the update path re-opens the device.
    RunUpdate {
        image: PathBuf,
        expected_version: Option<String>,
    },
    /// UI asked for DFU only; sent on a fresh handle after ours is closed.
    EnterDfu,
    /// The command channel closed — the app is shutting down.
    Quit,
}

/// Top-level connect/serve/reconnect cycle. One iteration = one session.
fn worker(mut api: HidApi, rx: mpsc::Receiver<DeviceCmd>) {
    loop {
        let Some((dev, serial)) = wait_for_device(&mut api, &rx) else {
            return; // channel closed while searching
        };
        hello(&dev, serial);
        let end = session(&mut api, &rx, &dev);
        // Close our handle before anything re-opens the device (DFU/update),
        // and so a fresh session always re-posts Connected + Keymap.
        drop(dev);
        events::post(DeviceMsg::Disconnected);
        match end {
            SessionEnd::Lost => {}
            SessionEnd::EnterDfu => enter_dfu_standalone(&mut api),
            SessionEnd::RunUpdate {
                image,
                expected_version,
            } => run_update(&mut api, &image, expected_version.as_deref()),
            SessionEnd::Quit => return,
        }
    }
}

/// Search for the pad every ~800 ms, still serving UI commands (they mostly
/// fail politely while unplugged). Returns None when the channel closes.
fn wait_for_device(
    api: &mut HidApi,
    rx: &mpsc::Receiver<DeviceCmd>,
) -> Option<(HidDevice, String)> {
    loop {
        // Probe first so a plugged-in pad connects without the initial wait.
        let _ = api.refresh_devices();
        if let Some((path, serial)) = find_raw(api) {
            if let Ok(dev) = api.open_path(&path) {
                return Some((dev, serial));
            }
        }
        match rx.recv_timeout(SEARCH_PERIOD) {
            Ok(cmd) => handle_cmd_offline(api, cmd),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return None,
        }
    }
}

/// Commands that arrive while no device is open. StartUpdate still runs —
/// it can resume a pad already sitting in the DFU bootloader.
fn handle_cmd_offline(api: &mut HidApi, cmd: DeviceCmd) {
    match cmd {
        DeviceCmd::StartUpdate {
            image,
            expected_version,
        } => run_update(api, &image, expected_version.as_deref()),
        DeviceCmd::EnterDfuOnly => enter_dfu_standalone(api),
        DeviceCmd::SyncKeymap { .. } | DeviceCmd::FactoryReset => {
            events::post(DeviceMsg::SyncDone {
                ok: false,
                detail: "device not connected".into(),
            });
        }
        // Nothing to read; the next Connected re-posts the keymap anyway.
        // A brightness preview with no pad is simply moot.
        DeviceCmd::ReadKeymap
        | DeviceCmd::SetLedBrightness { .. }
        | DeviceCmd::SetTransientLedPattern { .. } => {}
    }
}

/// Just connected: identify the pad, then pull its whole keymap + analog
/// tuning so the UI starts from what is actually on the device.
fn hello(dev: &HidDevice, serial: String) {
    let version = query_version(dev).unwrap_or_else(|| "?".into());
    events::post(DeviceMsg::Connected { version, serial });
    match read_keymap(dev) {
        Ok(keymap) => events::post(keymap.into_msg()),
        Err(e) => events::post(DeviceMsg::SyncDone {
            ok: false,
            detail: format!("keymap read failed: {e}"),
        }),
    }
}

/// The connected loop: drain UI commands, pump input events, and watch for
/// the device going away. Returns when the session must end.
fn session(api: &mut HidApi, rx: &mpsc::Receiver<DeviceCmd>, dev: &HidDevice) -> SessionEnd {
    let mut next_presence = Instant::now() + PRESENCE_PERIOD;
    loop {
        // (a) Commands first so a sync isn't starved by a chatty event stream.
        loop {
            match rx.try_recv() {
                Ok(DeviceCmd::StartUpdate {
                    image,
                    expected_version,
                }) => {
                    return SessionEnd::RunUpdate {
                        image,
                        expected_version,
                    }
                }
                Ok(DeviceCmd::EnterDfuOnly) => return SessionEnd::EnterDfu,
                Ok(DeviceCmd::SyncKeymap {
                    slots,
                    joy_threshold,
                    joy_mode,
                    joy_mouse_speed,
                    led_brightness,
                    led_key_pattern,
                    led_ambient_pattern,
                }) => {
                    let (ok, detail) = match sync_keymap(
                        dev,
                        &slots,
                        joy_threshold,
                        joy_mode,
                        joy_mouse_speed,
                        led_brightness,
                        led_key_pattern,
                        led_ambient_pattern,
                    ) {
                        Ok(detail) => (true, detail),
                        Err(e) => (false, e),
                    };
                    events::post(DeviceMsg::SyncDone { ok, detail });
                }
                Ok(DeviceCmd::SetLedBrightness { brightness }) => {
                    // Live preview while the slider drags: best-effort, no
                    // SyncDone spam — the debounced sync that follows both
                    // persists and reports.
                    let mut reply = [0u8; 32];
                    let _ = command(dev, &[CMD_SET_LED, brightness], &mut reply, REPLY_TIMEOUT);
                }
                Ok(DeviceCmd::SetTransientLedPattern {
                    key_pattern,
                    ambient_pattern,
                }) => {
                    // This is the same RAM-only pattern command used during
                    // keymap sync, but intentionally omits CMD_SAVE.
                    let key = key_pattern.to_wire();
                    let ambient = ambient_pattern.to_wire();
                    let mut reply = [0u8; 32];
                    let _ = command(
                        dev,
                        &[
                            CMD_SET_LEDPATTERN,
                            key[0],
                            key[1],
                            key[2],
                            key[3],
                            ambient[0],
                            ambient[1],
                            ambient[2],
                            ambient[3],
                        ],
                        &mut reply,
                        REPLY_TIMEOUT,
                    );
                }
                Ok(DeviceCmd::ReadKeymap) => match read_keymap(dev) {
                    Ok(keymap) => events::post(keymap.into_msg()),
                    Err(e) => events::post(DeviceMsg::SyncDone {
                        ok: false,
                        detail: format!("keymap read failed: {e}"),
                    }),
                },
                Ok(DeviceCmd::FactoryReset) => {
                    match factory_reset(dev).and_then(|()| read_keymap(dev)) {
                        Ok(keymap) => {
                            events::post(keymap.into_msg());
                            events::post(DeviceMsg::SyncDone {
                                ok: true,
                                detail: "factory defaults restored".into(),
                            });
                        }
                        Err(e) => events::post(DeviceMsg::SyncDone {
                            ok: false,
                            detail: format!("factory reset: {e}"),
                        }),
                    }
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => return SessionEnd::Quit,
            }
        }

        // (b) Pump one event report; the short timeout paces the loop.
        let mut buf = [0u8; 32];
        match dev.read_timeout(&mut buf, EVENT_READ_MS) {
            Ok(0) => {} // timeout — nothing pending
            Ok(n) => post_if_event(&buf[..n]),
            Err(_) => return SessionEnd::Lost, // unplugged mid-read
        }

        // (c) Silent-unplug check: some hidapi backends never error after an
        // unplug, they just return 0 bytes forever.
        if Instant::now() >= next_presence {
            next_presence = Instant::now() + PRESENCE_PERIOD;
            let _ = api.refresh_devices();
            if find_raw(api).is_none() {
                return SessionEnd::Lost;
            }
        }
    }
}

// ---------------------------------------------------------------- protocol --

/// One command round-trip on the raw interface. hidapi wants a leading
/// report-ID byte (0x00 — the interface defines no report IDs). Replies echo
/// the command byte; reports with the top bit set are input events that raced
/// the reply — those are decoded and posted, never dropped, and the read
/// continues until the real reply or the deadline.
fn command(
    dev: &HidDevice,
    cmd: &[u8],
    reply: &mut [u8; 32],
    timeout: Duration,
) -> Result<usize, String> {
    debug_assert!(!cmd.is_empty() && cmd.len() <= 32);
    let mut out = [0u8; 33];
    out[1..1 + cmd.len()].copy_from_slice(cmd);
    dev.write(&out).map_err(|e| e.to_string())?;
    let deadline = Instant::now() + timeout;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(format!("no reply to command 0x{:02x}", cmd[0]));
        }
        let n = dev
            .read_timeout(reply, left.as_millis() as i32)
            .map_err(|e| e.to_string())?;
        if n == 0 {
            continue; // hidapi timeout — the deadline check above will fire
        }
        if reply[0] >= EVENT_MARK {
            post_if_event(&reply[..n]);
            continue;
        }
        if reply[0] == cmd[0] {
            return Ok(n);
        }
        // A stale reply from an earlier timed-out command — skip it.
    }
}

/// Most commands acknowledge with [cmd, 0x01].
fn expect_ack(n: usize, reply: &[u8; 32], what: &str) -> Result<(), String> {
    if n >= 2 && reply[1] == 0x01 {
        Ok(())
    } else {
        Err(format!("{what}: device did not acknowledge"))
    }
}

/// Decode an unsolicited event report and post it to the UI.
fn post_if_event(buf: &[u8]) {
    if let Some(ev) = decode_event(buf) {
        events::post(DeviceMsg::Event(ev));
    }
}

/// [0x80, src, a, b] per the firmware's event spec. Unknown sources (future
/// firmware) are ignored rather than treated as an error.
fn decode_event(buf: &[u8]) -> Option<PadEvent> {
    if buf.len() < 4 || buf[0] != EVENT_MARK {
        return None;
    }
    let (a, b) = (buf[2], buf[3]);
    match buf[1] {
        0 => Some(PadEvent::Key {
            index: a,
            pressed: b != 0,
        }),
        1 => Some(PadEvent::Encoder { cw: a != 0 }),
        2 => Some(PadEvent::EncoderButton { pressed: a != 0 }),
        3 => Some(PadEvent::Joystick {
            dir: a,
            active: b != 0,
        }),
        4 => Some(PadEvent::Touch),
        _ => None,
    }
}

/// Pack a slot into its 4-byte wire form: kind (0/1/2), mods, code u16 LE.
pub fn slot_to_wire(slot: Slot) -> [u8; 4] {
    let kind = match slot.kind {
        SlotKind::None => 0,
        SlotKind::Keyboard => 1,
        SlotKind::Consumer => 2,
    };
    let code = slot.code.to_le_bytes();
    [kind, slot.mods, code[0], code[1]]
}

/// Unpack a 4-byte wire slot. Unknown kind bytes decode as None so a newer
/// firmware can't wedge the app.
pub fn slot_from_wire(bytes: &[u8]) -> Slot {
    if bytes.len() < 4 {
        return Slot::default();
    }
    let kind = match bytes[0] {
        1 => SlotKind::Keyboard,
        2 => SlotKind::Consumer,
        _ => SlotKind::None,
    };
    Slot {
        kind,
        mods: bytes[1],
        code: u16::from_le_bytes([bytes[2], bytes[3]]),
    }
}

/// Everything read_keymap pulls off the pad in one go.
struct DeviceKeymap {
    slots: [Slot; SLOT_COUNT],
    joy_threshold: u16,
    joy_mode: JoyMode,
    joy_mouse_speed: u8,
    led_brightness: u8,
    led_key_pattern: LedPattern,
    led_ambient_pattern: LedPattern,
}

impl DeviceKeymap {
    fn into_msg(self) -> DeviceMsg {
        DeviceMsg::Keymap {
            slots: self.slots,
            joy_threshold: self.joy_threshold,
            joy_mode: self.joy_mode,
            joy_mouse_speed: self.joy_mouse_speed,
            led_brightness: self.led_brightness,
            led_key_pattern: self.led_key_pattern,
            led_ambient_pattern: self.led_ambient_pattern,
        }
    }
}

/// Pull the whole keymap (4 pages) plus the joystick threshold and mode.
fn read_keymap(dev: &HidDevice) -> Result<DeviceKeymap, String> {
    let mut slots = [Slot::default(); SLOT_COUNT];
    for page in 0..KEYMAP_PAGES {
        let mut reply = [0u8; 32];
        let n = command(dev, &[CMD_GET_KEYMAP, page], &mut reply, REPLY_TIMEOUT)?;
        if n < 3 || reply[1] != page {
            return Err(format!("GET_KEYMAP page {page}: malformed reply"));
        }
        let count = reply[2] as usize;
        let base = page as usize * PAGE_SLOTS;
        if base + count > SLOT_COUNT || 3 + count * 4 > n {
            return Err(format!(
                "GET_KEYMAP page {page}: implausible slot count {count}"
            ));
        }
        for (i, chunk) in reply[3..3 + count * 4].chunks_exact(4).enumerate() {
            slots[base + i] = slot_from_wire(chunk);
        }
    }
    let mut reply = [0u8; 32];
    let n = command(dev, &[CMD_GET_ANALOG], &mut reply, REPLY_TIMEOUT)?;
    if n < 3 {
        return Err("GET_ANALOG: short reply".into());
    }
    let joy_threshold = u16::from_le_bytes([reply[1], reply[2]]);
    // Pre-0.3 firmware silently drops GET_JOYMODE/GET_LED, so a timeout here
    // is not an error: it means the defaults, the only thing that firmware
    // can do.
    let (joy_mode, joy_mouse_speed) =
        match command(dev, &[CMD_GET_JOYMODE], &mut reply, REPLY_TIMEOUT) {
            Ok(n) if n >= 3 => (JoyMode::from_wire(reply[1]), reply[2].clamp(1, 10)),
            _ => (JoyMode::Keys, DEFAULT_JOY_MOUSE_SPEED),
        };
    let led_brightness = match command(dev, &[CMD_GET_LED], &mut reply, REPLY_TIMEOUT) {
        Ok(n) if n >= 2 => reply[1],
        _ => DEFAULT_LED_BRIGHTNESS,
    };
    let (led_key_pattern, led_ambient_pattern) =
        match command(dev, &[CMD_GET_LEDPATTERN], &mut reply, REPLY_TIMEOUT) {
            Ok(n) if n >= 9 => (
                LedPattern::from_wire([reply[1], reply[2], reply[3], reply[4]]),
                LedPattern::from_wire([reply[5], reply[6], reply[7], reply[8]]),
            ),
            _ => (LedPattern::Rainbow, LedPattern::Rainbow),
        };
    Ok(DeviceKeymap {
        slots,
        joy_threshold,
        joy_mode,
        joy_mouse_speed,
        led_brightness,
        led_key_pattern,
        led_ambient_pattern,
    })
}

/// Push the whole keymap + analog tuning + joystick mode to RAM, then SAVE
/// to flash. Ok carries the human detail line for the UI — the mode write is
/// tolerated failing on pre-0.3 firmware, and the detail says so.
fn sync_keymap(
    dev: &HidDevice,
    slots: &[Slot; SLOT_COUNT],
    joy_threshold: u16,
    joy_mode: JoyMode,
    joy_mouse_speed: u8,
    led_brightness: u8,
    led_key_pattern: LedPattern,
    led_ambient_pattern: LedPattern,
) -> Result<String, String> {
    for page in 0..KEYMAP_PAGES {
        let base = page as usize * PAGE_SLOTS;
        let count = PAGE_SLOTS.min(SLOT_COUNT - base);
        let mut out = Vec::with_capacity(3 + count * 4);
        out.extend_from_slice(&[CMD_SET_KEYMAP, page, count as u8]);
        for slot in &slots[base..base + count] {
            out.extend_from_slice(&slot_to_wire(*slot));
        }
        let mut reply = [0u8; 32];
        let n = command(dev, &out, &mut reply, REPLY_TIMEOUT)?;
        expect_ack(n, &reply, &format!("SET_KEYMAP page {page}"))?;
    }
    let [lo, hi] = joy_threshold.to_le_bytes();
    let mut reply = [0u8; 32];
    let n = command(dev, &[CMD_SET_ANALOG, lo, hi], &mut reply, REPLY_TIMEOUT)?;
    expect_ack(n, &reply, "SET_ANALOG")?;
    // Pre-0.3 firmware drops these commands; joystick mode and brightness
    // then stay at their defaults on the pad. Everything else synced fine,
    // so report success with a nudge instead of failing the sync.
    let mut mode_supported = command(
        dev,
        &[CMD_SET_JOYMODE, joy_mode.to_wire(), joy_mouse_speed],
        &mut reply,
        REPLY_TIMEOUT,
    )
    .map(|n| expect_ack(n, &reply, "SET_JOYMODE").is_ok())
    .unwrap_or(false);
    // 0.3/0.4 firmware acks SET_JOYMODE but silently degrades the grade mode
    // (wire 2) to keys — a readback is the only way to tell, so grade gets
    // one and joins the "needs a firmware update" nudge on mismatch.
    if mode_supported && joy_mode == JoyMode::Grade {
        mode_supported = matches!(
            command(dev, &[CMD_GET_JOYMODE], &mut reply, REPLY_TIMEOUT),
            Ok(n) if n >= 2 && reply[1] == JoyMode::Grade.to_wire()
        );
    }
    let led_supported = command(
        dev,
        &[CMD_SET_LED, led_brightness],
        &mut reply,
        REPLY_TIMEOUT,
    )
    .map(|n| expect_ack(n, &reply, "SET_LED").is_ok())
    .unwrap_or(false);
    let kp = led_key_pattern.to_wire();
    let up = led_ambient_pattern.to_wire();
    let pattern_supported = command(
        dev,
        &[CMD_SET_LEDPATTERN, kp[0], kp[1], kp[2], kp[3], up[0], up[1], up[2], up[3]],
        &mut reply,
        REPLY_TIMEOUT,
    )
    .map(|n| expect_ack(n, &reply, "SET_LEDPATTERN").is_ok())
    .unwrap_or(false);
    let n = command(
        dev,
        &[CMD_SAVE, b'S', b'A', b'V', b'E'],
        &mut reply,
        SAVE_TIMEOUT,
    )?;
    expect_ack(n, &reply, "SAVE")?;
    Ok(if mode_supported && led_supported && pattern_supported {
        "keymap written · saved to flash".to_string()
    } else {
        "keymap saved · joystick/LED extras need a firmware update".to_string()
    })
}

/// RAM + flash back to firmware defaults; caller re-reads afterwards.
fn factory_reset(dev: &HidDevice) -> Result<(), String> {
    let mut reply = [0u8; 32];
    let n = command(
        dev,
        &[CMD_FACTORY_RESET, b'R', b'S', b'T', b'!'],
        &mut reply,
        SAVE_TIMEOUT, // it rewrites flash, same budget as SAVE
    )?;
    expect_ack(n, &reply, "FACTORY_RESET")
}

// --------------------------------------------------------------- discovery --

/// Locate the raw-HID (usage page 0xFF60) interface of the pad.
fn find_raw(api: &HidApi) -> Option<(CString, String)> {
    for info in api.device_list() {
        if info.vendor_id() == VID
            && info.product_id() == PID
            && info.usage_page() == RAW_USAGE_PAGE
        {
            let serial = info.serial_number().unwrap_or("?").to_string();
            return Some((info.path().to_owned(), serial));
        }
    }
    None
}

fn open_raw(api: &mut HidApi) -> Option<HidDevice> {
    let _ = api.refresh_devices();
    let (path, _) = find_raw(api)?;
    api.open_path(&path).ok()
}

fn query_version(dev: &HidDevice) -> Option<String> {
    let mut reply = [0u8; 32];
    let n = command(dev, &[CMD_VERSION], &mut reply, REPLY_TIMEOUT).ok()?;
    if n < 2 {
        return None;
    }
    let len = (reply[1] as usize).min(30);
    core::str::from_utf8(&reply[2..2 + len])
        .ok()
        .map(|s| s.to_string())
}

fn enter_dfu(dev: &HidDevice) -> Result<(), String> {
    let mut reply = [0u8; 32];
    let n = command(
        dev,
        &[CMD_ENTER_DFU, b'D', b'F', b'U', b'!'],
        &mut reply,
        REPLY_TIMEOUT,
    )?;
    if n >= 2 && reply[1] == 0x01 {
        Ok(())
    } else {
        Err("device did not acknowledge the DFU command".into())
    }
}

/// EnterDfuOnly: open a fresh handle (ours, if any, is already closed), send
/// the DFU magic, and report through the update log like the old flow did.
fn enter_dfu_standalone(api: &mut HidApi) {
    match open_raw(api) {
        Some(dev) => match enter_dfu(&dev) {
            Ok(()) => events::post(UpdateMsg::Log(
                "device rebooted into DFU mode (0483:df11)".into(),
            )),
            Err(e) => events::post(UpdateMsg::Log(format!("enter DFU failed: {e}"))),
        },
        None => events::post(UpdateMsg::Log("device not found".into())),
    }
}

// ------------------------------------------------------------------ update --

/// The whole update: sanity-check the image, drop the device into the ROM
/// bootloader, flash over DFU, wait for the app to come back.
fn run_update(api: &mut HidApi, image_path: &PathBuf, expected_version: Option<&str>) {
    let phase = |s: &str| events::post(UpdateMsg::Phase(s.to_string()));
    let log = |s: String| events::post(UpdateMsg::Log(s));
    let fail = |s: String| events::post(UpdateMsg::Failed(s));

    // -- image sanity: a Cortex-M0 vector table for this exact chip --
    let image = match std::fs::read(image_path) {
        Ok(b) => b,
        Err(e) => return fail(format!("cannot read image: {e}")),
    };
    const MAX_FIRMWARE_LEN: usize = 126 * 1024;
    if image.len() < 192 || image.len() > MAX_FIRMWARE_LEN {
        return fail(format!(
            "image is {} bytes — firmware must fit below the reserved config page (max {})",
            image.len(),
            MAX_FIRMWARE_LEN
        ));
    }
    let sp = u32::from_le_bytes(image[0..4].try_into().unwrap());
    let rv_raw = u32::from_le_bytes(image[4..8].try_into().unwrap());
    // Cortex-M reset handlers must carry the Thumb bit. The F072CB has 16 KiB
    // SRAM and application flash stops before the config page at 0x0801_F800.
    let rv = rv_raw & !1;
    if sp & 0x3 != 0
        || !(0x2000_0000..=0x2000_4000).contains(&sp)
        || rv_raw & 1 == 0
        || !(0x0800_0000..0x0801_F800).contains(&rv)
    {
        return fail(format!(
            "not an OpenMicro firmware image (SP={sp:08x} RV={rv_raw:08x}) — expected a Thumb vector table for 0x08000000"
        ));
    }
    log(format!(
        "image: {} ({} bytes)",
        image_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("?"),
        image.len()
    ));

    // -- get this pad into the bootloader (or resume one already there) --
    //
    // A generic STM32 ROM bootloader has no OpenMicro product identity. If a
    // normal pad and any 0483:df11 device are both present, never guess: that
    // DFU device could be unrelated hardware. A lone pre-existing DFU device
    // is accepted only as the user's explicit recovery target.
    let raw = open_raw(api);
    let bootloader = match dfuse::find_bootloader() {
        Ok(device) => device,
        Err(e) => return fail(e),
    };
    match (raw, bootloader.is_some()) {
        (Some(_), true) => {
            return fail(
                "an OpenMicro pad and a separate STM32 DFU device are both connected; disconnect the unrelated DFU device"
                    .into(),
            )
        }
        (Some(dev), false) => {
            phase("Rebooting the pad into DFU mode…");
            if let Err(e) = enter_dfu(&dev) {
                return fail(format!("enter DFU: {e}"));
            }
        }
        (None, true) => log("DFU bootloader already present — resuming recovery".into()),
        (None, false) => {
            return fail(
                "device not found (and no DFU bootloader present) — plug the pad in".into(),
            )
        }
    }

    phase("Waiting for the DFU bootloader…");
    let deadline = Instant::now() + Duration::from_secs(8);
    let dfu = loop {
        match dfuse::find_bootloader() {
            Ok(Some(device)) => break device,
            Ok(None) => {}
            Err(e) => return fail(e),
        }
        if Instant::now() > deadline {
            return fail("DFU bootloader (0483:df11) never enumerated".into());
        }
        std::thread::sleep(Duration::from_millis(200));
    };

    // -- flash --
    if let Err(e) = dfuse::flash(dfu, &image, |p, frac| {
        events::post(UpdateMsg::Phase(p.to_string()));
        events::post(UpdateMsg::Progress(frac));
    }) {
        return fail(format!("DFU flashing failed: {e} — recovery: SWD on J2"));
    }

    // -- wait for the new firmware --
    phase("Waiting for the pad to come back…");
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        std::thread::sleep(Duration::from_millis(300));
        if let Some(dev) = open_raw(api) {
            let version = query_version(&dev).unwrap_or_else(|| "?".into());
            if let Some(expected) = expected_version {
                if version != expected {
                    return fail(format!(
                        "pad returned after flashing, but reports firmware {version} (expected {expected})"
                    ));
                }
            }
            events::post(UpdateMsg::Done { version });
            // The handle is dropped here; the worker's reconnect cycle will
            // re-open the pad and re-post Connected + Keymap.
            return;
        }
        if Instant::now() > deadline {
            return fail("flashed OK, but the device did not re-enumerate".into());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_screen_consumer_usage_roundtrips_little_endian() {
        let lock = Slot {
            kind: SlotKind::Consumer,
            mods: 0,
            code: 0x019E,
        };
        let wire = slot_to_wire(lock);
        assert_eq!(wire, [2, 0, 0x9E, 0x01]);
        assert_eq!(slot_from_wire(&wire), lock);
    }
}
