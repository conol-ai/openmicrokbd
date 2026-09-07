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
//! post Disconnected and go back to searching. Firmware updates (boot.rs for
//! the resident bootloader, dfuse.rs for the one-time ROM DFU migration)
//! open the device themselves, so the held handle is closed first and a full
//! reconnect cycle afterwards re-posts Connected/Keymap.
//!
//! While searching, a pad parked in its bootloader (1209:0002) is reported
//! as `BootloaderPresent` rather than treated as a pad: it speaks a
//! different protocol and has no keymap.
//!
//! Everything is reported to the UI through the framework-neutral event bus.

use hidapi::{HidApi, HidDevice};
use std::ffi::{CStr, CString};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use openmicro_layout::{
    op, status, u32_at, validate_app, AppHeader, BootInfo, BootInfoReply, Validity, BOOT_PROTOCOL,
    DATA_BASE, ENTER_BOOT_KEY, FLASH_BASE, FLASH_SIZE, HEADER_OFFSET, PAGE_SIZE, RESET_OFFSET,
};

use crate::boot;
use crate::config::{
    JoyMode, LedPattern, Slot, SlotKind, DEFAULT_JOY_MOUSE_SPEED, DEFAULT_LED_BRIGHTNESS,
    SLOT_COUNT,
};
use crate::dfuse;
use crate::events;

pub const VID: u16 = 0x1209;
pub const PID: u16 = 0x0001;
/// Every USB identity the pad can boot with: its own, and — in the opt-in
/// Codex Micro compat mode (firmware 0.8.0+) — the Codex Micro's, under
/// which it still exposes the same vendor interface so this app keeps
/// working. The raw-HID usage page is what actually singles it out.
pub const IDENTITIES: [(u16, u16); 2] = [(VID, PID), (0x303A, 0x8360)];
const RAW_USAGE_PAGE: u16 = 0xFF60;

/// Which USB identity the pad boots with (firmware 0.8.0+, `GET_MODE` /
/// `SET_MODE`). Changing it restarts the pad under the other identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceMode {
    OpenMicro,
    Codex,
}

impl DeviceMode {
    fn from_wire(byte: u8) -> Option<Self> {
        match byte {
            0 => Some(Self::OpenMicro),
            1 => Some(Self::Codex),
            _ => None,
        }
    }

    fn to_wire(self) -> u8 {
        match self {
            Self::OpenMicro => 0,
            Self::Codex => 1,
        }
    }
}

impl std::fmt::Display for DeviceMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::OpenMicro => "OpenMicro",
            Self::Codex => "Codex Micro compat",
        })
    }
}

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
/// Firmware 0.10.0+: reboot into the resident bootloader's update mode.
const CMD_ENTER_BOOT: u8 = op::ENTER_BOOT;
/// Firmware 0.10.0+: the bootloader info block, relayed by the application.
const CMD_BOOT_INFO: u8 = op::BOOT_INFO;

/// How long the bootloader gets to enumerate after ENTER_BOOT (the pad
/// resets, the OS re-enumerates; a few hundred ms in practice). Windows
/// gets three times as long: the first time 1209:0002 is plugged in, the
/// HID class driver install ("Setting up a device", a Windows Update driver
/// lookup) can easily take longer than ten seconds.
const BOOT_APPEAR_TIMEOUT: Duration =
    Duration::from_secs(if cfg!(target_os = "windows") { 30 } else { 10 });
/// How often the bus is probed while waiting for the bootloader. Short
/// enough to see the gap in which the application pad is gone, which is
/// what tells a pad that came back in application mode from one that never
/// left it.
const BOOT_APPEAR_POLL: Duration = Duration::from_millis(150);
/// The largest file any update path accepts: everything from 0x08000000 up
/// to the Work Louder file slots and the config page, which no update may
/// touch. A valid combined image (bootloader + application slot) is at most
/// exactly this long; ROM DFU would erase whatever pages a longer file
/// covers.
const MAX_FIRMWARE_LEN: usize = (DATA_BASE - FLASH_BASE) as usize;
/// How long the application gets to come back after an update.
const APP_RETURN_TIMEOUT: Duration = Duration::from_secs(10);

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
        /// None on firmware that predates device modes (< 0.8.0).
        mode: Option<DeviceMode>,
        /// The pad's answer to BOOT_INFO: Some when it runs on top of the
        /// resident bootloader (firmware 0.10.0+ over boot 1.x), None on
        /// older firmware or a debugger-flashed app without a bootloader.
        boot: Option<BootInfoReply>,
    },
    Disconnected,
    /// A pad is sitting in its bootloader (1209:0002) and no application-mode
    /// pad is connected. `info` is its BOOT_INFO, or why there is none: the
    /// interface could not be opened (Linux without the udev rule) or it
    /// did not answer usefully (timeout, another protocol version).
    BootloaderPresent {
        serial: String,
        info: Result<BootInfoReply, BootloaderError>,
    },
    /// The bootloader-mode pad left the bus (unplugged, or it booted its
    /// firmware — a `Connected` follows in that case).
    BootloaderGone,
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

/// Why a bootloader-mode pad has no BOOT_INFO to show. The two halves get
/// different advice: an open failure is almost always permissions (the
/// udev rule on Linux), a failed BOOT_INFO is the pad's own doing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BootloaderError {
    /// hidapi refused to open the interface.
    Open(String),
    /// The interface opened, but BOOT_INFO timed out, reported a non-zero
    /// status or an unsupported protocol.
    Info(String),
}

impl std::fmt::Display for BootloaderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Open(e) => write!(
                f,
                "could not be opened: {e}{}",
                if cfg!(target_os = "linux") {
                    " — add the udev rule for 1209:0002 (docs/linux-firmware-updates.md)"
                } else {
                    ""
                }
            ),
            Self::Info(e) => write!(f, "BOOT_INFO failed: {e}"),
        }
    }
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
    /// Reboot into the ST ROM DFU (0483:df11) and leave the pad there. Only
    /// for reinstalling the bootloader with a combined image.
    EnterDfuOnly,
    /// Reboot into the resident bootloader's update mode and leave the pad
    /// there (firmware 0.10.0+).
    EnterBootOnly,
    /// Tell a pad in bootloader mode to validate and start its firmware.
    BootRun,
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
    /// Runtime-only override for one physical key LED. Firmware before 0.7
    /// rejects this command; the worker probes once and then ignores updates.
    SetKeyLedOverride {
        index: u8,
        color: Option<(u8, u8, u8)>,
    },
    ReadKeymap,
    FactoryReset,
    /// Persist a boot identity and restart the pad under it. The session
    /// ends when the pad drops off the bus; it comes back as a fresh
    /// connection (and a fresh `Connected`) with the new mode.
    SetDeviceMode {
        mode: DeviceMode,
    },
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
    /// UI asked for bootloader mode only; same handling as EnterDfu.
    EnterBoot,
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
            SessionEnd::EnterBoot => enter_boot_standalone(&mut api),
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
    // The bootloader-mode pad the UI has already been told about, so it is
    // announced once (with its BOOT_INFO) rather than re-opened every 800 ms
    // — until a command touched the pad, after which it is announced again
    // with fresh BOOT_INFO (an Install that failed half-way leaves the slot
    // erased; the card must say so).
    let mut announcer = BootloaderAnnouncer::default();
    loop {
        // Probe first so a plugged-in pad connects without the initial wait.
        let _ = api.refresh_devices();
        if let Some((path, serial)) = find_raw(api) {
            if let Ok(dev) = api.open_path(&path) {
                return Some((dev, serial));
            }
        }
        announce_bootloader(api, &mut announcer);
        match rx.recv_timeout(SEARCH_PERIOD) {
            Ok(cmd) => {
                if handle_cmd_offline(api, cmd) {
                    announcer.invalidate();
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return None,
        }
    }
}

/// What `announce_bootloader` should do after one look at the bus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Announce {
    /// Query BOOT_INFO and post `BootloaderPresent`.
    Query,
    /// Post `BootloaderGone`.
    Gone,
    Nothing,
}

/// Which bootloader-mode pad the UI knows about (by serial), and whether
/// what it was told may be stale. Pure so the re-announce rules are tested
/// without hidapi.
#[derive(Debug, Default)]
pub struct BootloaderAnnouncer {
    announced: Option<String>,
    /// Set after a command that may have changed the pad (an upload, a
    /// BOOT_RUN): the next sighting re-queries BOOT_INFO even for the same
    /// serial.
    stale: bool,
}

impl BootloaderAnnouncer {
    /// `serial`: the first bootloader-mode pad on the bus, if any.
    pub fn observe(&mut self, serial: Option<&str>) -> Announce {
        match serial {
            Some(serial) => {
                if self.announced.as_deref() == Some(serial) && !self.stale {
                    return Announce::Nothing;
                }
                self.announced = Some(serial.to_string());
                self.stale = false;
                Announce::Query
            }
            None => {
                self.stale = false;
                if self.announced.take().is_some() {
                    Announce::Gone
                } else {
                    Announce::Nothing
                }
            }
        }
    }

    /// The announced pad may have changed (or left): announce it afresh.
    pub fn invalidate(&mut self) {
        self.stale = true;
    }
}

/// While no application-mode pad is around, a pad sitting in its bootloader
/// is reported to the UI together with its BOOT_INFO (so the panel can say
/// which firmware it holds) instead of being ignored. Only the first one is
/// reported: `plan_update` refuses to choose between two anyway.
fn announce_bootloader(api: &HidApi, announcer: &mut BootloaderAnnouncer) {
    let found = boot::find_bootloaders(api);
    let first = found.first();
    match announcer.observe(first.map(|(_, serial)| serial.as_str())) {
        Announce::Query => {
            let Some((path, serial)) = first else {
                return;
            };
            let info = query_bootloader_info(api, path);
            if let Err(e) = &info {
                events::post(UpdateMsg::Log(format!("bootloader {serial}: {e}")));
            }
            events::post(DeviceMsg::BootloaderPresent {
                serial: serial.clone(),
                info,
            });
        }
        Announce::Gone => events::post(DeviceMsg::BootloaderGone),
        Announce::Nothing => {}
    }
}

/// Open a bootloader interface and ask it for BOOT_INFO, telling the two
/// failures apart.
fn query_bootloader_info(api: &HidApi, path: &CStr) -> Result<BootInfoReply, BootloaderError> {
    let mut dev = boot::BootDevice::open_path(api, path).map_err(BootloaderError::Open)?;
    dev.info().map_err(BootloaderError::Info)
}

/// Commands that arrive while no device is open. StartUpdate still runs —
/// it can resume a pad already sitting in the bootloader (or in ROM DFU).
/// Returns true when the command may have changed a pad's mode or contents,
/// so a bootloader-mode pad is announced again with fresh BOOT_INFO.
fn handle_cmd_offline(api: &mut HidApi, cmd: DeviceCmd) -> bool {
    match cmd {
        DeviceCmd::StartUpdate {
            image,
            expected_version,
        } => {
            run_update(api, &image, expected_version.as_deref());
            true
        }
        DeviceCmd::EnterDfuOnly => {
            enter_dfu_standalone(api);
            true
        }
        DeviceCmd::EnterBootOnly => {
            enter_boot_standalone(api);
            true
        }
        DeviceCmd::BootRun => {
            boot_run_standalone(api);
            true
        }
        DeviceCmd::SyncKeymap { .. }
        | DeviceCmd::FactoryReset
        | DeviceCmd::SetDeviceMode { .. } => {
            events::post(DeviceMsg::SyncDone {
                ok: false,
                detail: "device not connected".into(),
            });
            false
        }
        // Nothing to read; the next Connected re-posts the keymap anyway.
        // A brightness preview with no pad is simply moot.
        DeviceCmd::ReadKeymap
        | DeviceCmd::SetLedBrightness { .. }
        | DeviceCmd::SetTransientLedPattern { .. }
        | DeviceCmd::SetKeyLedOverride { .. } => false,
    }
}

/// Just connected: identify the pad, then pull its whole keymap + analog
/// tuning so the UI starts from what is actually on the device.
fn hello(dev: &HidDevice, serial: String) {
    let version = query_version(dev).unwrap_or_else(|| "?".into());
    // Older firmware never answers GET_MODE, and the 500 ms it would take
    // to find that out would delay Connected on every plug-in.
    let mode = if supports_device_mode(&version) {
        query_mode(dev)
    } else {
        None
    };
    // Same for BOOT_INFO — but it is the authority on whether the pad has a
    // bootloader, so a version we cannot parse is probed rather than assumed.
    let boot = if probes_boot_info(&version) {
        query_boot_info(dev)
    } else {
        None
    };
    events::post(DeviceMsg::Connected {
        version,
        serial,
        mode,
        boot,
    });
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
    let mut key_led_override_supported = true;
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
                Ok(DeviceCmd::EnterBootOnly) => return SessionEnd::EnterBoot,
                // A second pad in bootloader mode next to this one: the
                // bootloader has its own interface, so no need to end the
                // session for it.
                Ok(DeviceCmd::BootRun) => boot_run_standalone(api),
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
                Ok(DeviceCmd::SetKeyLedOverride { index, color }) => {
                    if key_led_override_supported && index < 13 {
                        let (enabled, r, g, b) = color
                            .map(|(r, g, b)| (1, r, g, b))
                            .unwrap_or((0, 0, 0, 0));
                        let mut reply = [0u8; 32];
                        key_led_override_supported = command(
                            dev,
                            &[CMD_SET_KEY_LED_OVERRIDE, index, enabled, r, g, b],
                            &mut reply,
                            REPLY_TIMEOUT,
                        )
                        .and_then(|n| expect_ack(n, &reply, "SET_KEY_LED_OVERRIDE"))
                        .is_ok();
                    }
                }
                Ok(DeviceCmd::ReadKeymap) => match read_keymap(dev) {
                    Ok(keymap) => events::post(keymap.into_msg()),
                    Err(e) => events::post(DeviceMsg::SyncDone {
                        ok: false,
                        detail: format!("keymap read failed: {e}"),
                    }),
                },
                Ok(DeviceCmd::SetDeviceMode { mode }) => {
                    // The pad rewrites flash (SAVE budget), acks, then resets
                    // ~50 ms later — the next read fails and this session
                    // ends as Lost, which is the normal path back to search.
                    let mut reply = [0u8; 32];
                    let result = command(
                        dev,
                        &[CMD_SET_MODE, mode.to_wire(), b'M', b'O', b'D', b'E'],
                        &mut reply,
                        SAVE_TIMEOUT,
                    )
                    .and_then(|n| expect_ack(n, &reply, "SET_MODE"));
                    events::post(match result {
                        Ok(()) => DeviceMsg::SyncDone {
                            ok: true,
                            detail: format!("switching to {mode} mode — the pad is restarting"),
                        },
                        Err(e) => DeviceMsg::SyncDone {
                            ok: false,
                            detail: format!("device mode: {e}"),
                        },
                    });
                }
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

/// Locate the raw-HID (usage page 0xFF60) interface of the pad, under
/// whichever of its USB identities it booted with.
fn find_raw(api: &HidApi) -> Option<(CString, String)> {
    for info in api.device_list() {
        if IDENTITIES.contains(&(info.vendor_id(), info.product_id()))
            && info.usage_page() == RAW_USAGE_PAGE
        {
            let serial = info.serial_number().unwrap_or("?").to_string();
            return Some((info.path().to_owned(), serial));
        }
    }
    None
}

fn open_raw(api: &mut HidApi) -> Option<HidDevice> {
    open_raw_with_serial(api).map(|(dev, _)| dev)
}

fn open_raw_with_serial(api: &mut HidApi) -> Option<(HidDevice, String)> {
    let _ = api.refresh_devices();
    let (path, serial) = find_raw(api)?;
    api.open_path(&path).ok().map(|dev| (dev, serial))
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

/// "0.10.0", "0.10.0-rc.1" or "v0.10.0" → (0, 10, 0). None when the string
/// is not three numbers — e.g. the "?" placeholder for a failed VERSION
/// query. A prerelease suffix is stripped first so "0.10.0-rc.1" is 0.10.0,
/// not 0.10.1.
pub fn parse_version(version: &str) -> Option<(u32, u32, u32)> {
    let version = version.trim();
    let version = version.strip_prefix('v').unwrap_or(version);
    let core = version.split('-').next().unwrap_or(version);
    let mut parts = core.split('.').map(|part| part.parse::<u32>().ok());
    match (parts.next(), parts.next(), parts.next()) {
        (Some(Some(major)), Some(Some(minor)), Some(Some(patch))) => Some((major, minor, patch)),
        _ => None,
    }
}

/// Device modes (GET_MODE / SET_MODE) arrived in firmware 0.8.0.
fn supports_device_mode(version: &str) -> bool {
    matches!(parse_version(version), Some((major, minor, _)) if major > 0 || minor >= 8)
}

/// BOOT_INFO arrived in firmware 0.10.0 with the bootloader. A version that
/// does not parse is probed too: the pad may be newer than this app's idea
/// of a version string, and BOOT_INFO — not the version — decides whether
/// the driverless update path exists. Older firmware simply never answers,
/// which costs one reply timeout on those pads only.
fn probes_boot_info(version: &str) -> bool {
    match parse_version(version) {
        Some((major, minor, _)) => major > 0 || minor >= 10,
        None => true,
    }
}

/// BOOT_INFO on the application interface: the 31-byte reply (no page /
/// chunk fields; the 32-byte report pads it). Only status 0 with protocol 1
/// means "this pad has a bootloader this app can drive": status 1 is a
/// 0.10+ application flashed by a debugger over an empty bootloader slot,
/// which must take the ROM DFU path like a 0.9.0 pad.
fn query_boot_info(dev: &HidDevice) -> Option<BootInfoReply> {
    let mut reply = [0u8; 32];
    let n = command(dev, &[CMD_BOOT_INFO], &mut reply, REPLY_TIMEOUT).ok()?;
    accept_boot_info(&reply[..n])
}

/// The pure half of `query_boot_info`.
pub fn accept_boot_info(reply: &[u8]) -> Option<BootInfoReply> {
    let info = BootInfoReply::parse(reply)?;
    (info.status == status::OK && u16::from(info.protocol) == BOOT_PROTOCOL).then_some(info)
}

/// ENTER_BOOT: the firmware acks, then resets into the bootloader's update
/// mode ~50 ms later (1209:0002 appears, 1209:0001 is gone).
fn enter_boot(dev: &HidDevice) -> Result<(), String> {
    let mut reply = [0u8; 32];
    let mut req = vec![CMD_ENTER_BOOT];
    req.extend_from_slice(&ENTER_BOOT_KEY);
    let n = command(dev, &req, &mut reply, REPLY_TIMEOUT)?;
    expect_ack(n, &reply, "ENTER_BOOT")
}

/// EnterBootOnly: like `enter_dfu_standalone`, for the resident bootloader.
fn enter_boot_standalone(api: &mut HidApi) {
    match open_raw(api) {
        Some(dev) => match enter_boot(&dev) {
            Ok(()) => events::post(UpdateMsg::Log(
                "device rebooted into its bootloader (1209:0002)".into(),
            )),
            Err(e) => events::post(UpdateMsg::Log(format!("enter bootloader failed: {e}"))),
        },
        None => events::post(UpdateMsg::Log("device not found".into())),
    }
}

/// BootRun: ask the one pad in bootloader mode to validate and start its
/// firmware. Reported through the update log; the pad comes back as a fresh
/// Connected (or as BootloaderPresent again if its firmware is invalid).
fn boot_run_standalone(api: &mut HidApi) {
    let _ = api.refresh_devices();
    let found = boot::find_bootloaders(api);
    let result = match found.as_slice() {
        [] => Err("no pad in bootloader mode".to_string()),
        [(path, _)] => boot::BootDevice::open_path(api, path).and_then(|mut dev| dev.run()),
        _ => Err("more than one pad is in bootloader mode; connect only one".into()),
    };
    events::post(UpdateMsg::Log(match result {
        Ok(()) => "bootloader: starting the firmware".into(),
        Err(e) => format!("boot firmware failed: {e}"),
    }));
}

/// GET_MODE. None on firmware before 0.8.0, which simply never replies.
fn query_mode(dev: &HidDevice) -> Option<DeviceMode> {
    let mut reply = [0u8; 32];
    let n = command(dev, &[CMD_GET_MODE], &mut reply, REPLY_TIMEOUT).ok()?;
    if n < 2 {
        return None;
    }
    DeviceMode::from_wire(reply[1])
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
/// With no application-mode pad around, a pad parked in its bootloader is
/// asked instead (the bootloader answers the same ENTER_DFU): that is how a
/// bootloader that cannot program its slot gets reinstalled.
fn enter_dfu_standalone(api: &mut HidApi) {
    if let Some(dev) = open_raw(api) {
        events::post(UpdateMsg::Log(match enter_dfu(&dev) {
            Ok(()) => "device rebooted into DFU mode (0483:df11)".into(),
            Err(e) => format!("enter DFU failed: {e}"),
        }));
        return;
    }
    let found = boot::find_bootloaders(api);
    let result = match found.as_slice() {
        [] => Err("device not found (no pad in application or bootloader mode)".to_string()),
        [(path, _)] => {
            boot::BootDevice::open_path(api, path).and_then(|mut dev| dev.enter_rom_dfu())
        }
        _ => Err("more than one pad is in bootloader mode; connect only one".into()),
    };
    events::post(UpdateMsg::Log(match result {
        Ok(()) => "bootloader rebooted into DFU mode (0483:df11)".into(),
        Err(e) => format!("enter DFU failed: {e}"),
    }));
}

// ------------------------------------------------------------------ update --

/// What kind of file the user (or the release catalog) handed us.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageKind {
    /// Bootloader + application, as build-firmware.sh publishes: flashable
    /// whole at 0x08000000 through ROM DFU, or sliced at `boot.app_base`
    /// for the bootloader's HID path. `app` says whether the slice is
    /// stamped (length + CRC verified) or a debugger-style unstamped build,
    /// which only ROM DFU may install.
    Combined { boot: BootInfo, app: Validity },
    /// An application-slot image: header at 0xC0, linked for `app_base`
    /// (recovered from its reset vector). Only the bootloader may write it.
    AppOnly { app_base: u32, app: Validity },
    /// A pre-bootloader (firmware ≤ 0.9.0) whole-flash image: vectors for
    /// 0x08000000, no header, nothing to verify.
    Legacy,
}

impl ImageKind {
    /// The application base the image was linked for and its validity —
    /// None for a legacy image, which has neither.
    fn app(&self) -> Option<(u32, &Validity)> {
        match self {
            ImageKind::Combined { boot, app } => Some((boot.app_base, app)),
            ImageKind::AppOnly { app_base, app } => Some((*app_base, app)),
            ImageKind::Legacy => None,
        }
    }
}

/// Which pads the worker can see, reduced to what the decision needs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PadTopology {
    /// An application-mode pad: its serial and, when it answered BOOT_INFO
    /// with status 0, that reply.
    pub app_pad: Option<(String, Option<BootInfoReply>)>,
    /// Serials of pads in bootloader mode (1209:0002).
    pub boot_pads: Vec<String>,
    /// STM32 ROM DFU devices (0483:df11) — no product identity, could be any
    /// ST board.
    pub dfu_devices: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpdatePlan {
    /// Drive the resident bootloader over HID. `enter_boot`: the pad is in
    /// application mode and must be asked to reboot first. `expect_serial`:
    /// accept only a bootloader with this serial (None when unknown).
    Hid {
        enter_boot: bool,
        expect_serial: Option<String>,
    },
    /// The pre-bootloader path: ROM DFU writes the whole image at
    /// 0x08000000. Now only the one-time migration from firmware ≤ 0.9.0 and
    /// the recovery of a pad parked in ROM DFU.
    RomDfu,
}

/// Recognise the image by the 32-byte block after the vector table: the
/// bootloader info block ("OMKB") makes it a combined image, the application
/// header ("OMKA") an app-only one, neither a legacy whole-flash image. A
/// combined image's application slice is validated here so a broken file is
/// refused before either path starts.
pub fn classify_image(image: &[u8]) -> Result<ImageKind, String> {
    if image.len() < RESET_OFFSET as usize {
        return Err(format!(
            "image is {} bytes — shorter than a vector table plus header",
            image.len()
        ));
    }
    let block = &image[HEADER_OFFSET as usize..];
    if let Some(boot) = BootInfo::parse(block) {
        if boot.app_base <= FLASH_BASE
            || boot.app_base >= FLASH_BASE + FLASH_SIZE
            || boot.app_base % PAGE_SIZE != 0
        {
            return Err(format!(
                "combined image declares an application base of 0x{:08x}, which is not a flash page of this chip",
                boot.app_base
            ));
        }
        let app_off = (boot.app_base - FLASH_BASE) as usize;
        if app_off + RESET_OFFSET as usize > image.len() {
            return Err(format!(
                "combined image ends before its application slot at 0x{:08x}",
                boot.app_base
            ));
        }
        let app = match validate_app(&image[app_off..], boot.app_base, boot.app_size) {
            Ok(validity) => validity,
            Err(e) => {
                return Err(format!(
                    "combined image: the application at 0x{:08x} is invalid ({})",
                    boot.app_base,
                    boot::describe(e)
                ))
            }
        };
        return Ok(ImageKind::Combined { boot, app });
    }
    if AppHeader::parse(block).is_some() {
        // The link address is in the reset vector (cortex-m-rt puts Reset
        // at `_stext` = base + RESET_OFFSET), so the image can be checked
        // here — vectors, header, CRC — before any pad is involved. The
        // bootloader path re-validates against the pad's own BOOT_INFO.
        let reset = u32_at(image, 4);
        let app_base = (reset & !1).wrapping_sub(RESET_OFFSET);
        if reset & 1 == 0
            || app_base <= FLASH_BASE
            || app_base >= DATA_BASE
            || app_base % PAGE_SIZE != 0
        {
            return Err(format!(
                "application image: reset vector 0x{reset:08x} is not linked for an application slot of this chip"
            ));
        }
        let app = validate_app(image, app_base, DATA_BASE - app_base).map_err(|e| {
            format!(
                "application image for 0x{app_base:08x} is invalid ({})",
                boot::describe(e)
            )
        })?;
        return Ok(ImageKind::AppOnly { app_base, app });
    }
    Ok(ImageKind::Legacy)
}

/// ROM DFU writes the file verbatim from 0x08000000, erasing every page it
/// covers. So nothing may reach the data pages (keymap.json, smart_actions,
/// the config page), and a combined image may carry nothing but erased
/// flash (0xFF) after its stamped application — the same rules as
/// `scripts/fw-image.py verify`. An unstamped application has no length to
/// bound the check with, so only the size rule applies to it.
pub fn check_rom_dfu_image(image: &[u8], kind: &ImageKind) -> Result<(), String> {
    if image.len() > MAX_FIRMWARE_LEN {
        return Err(format!(
            "image is {} bytes and would reach into the data pages at 0x{DATA_BASE:08x} (keymap and file slots); ROM DFU may write at most {MAX_FIRMWARE_LEN} bytes",
            image.len()
        ));
    }
    if let ImageKind::Combined {
        boot,
        app: Validity::Stamped(app),
    } = kind
    {
        let end = (boot.app_base - FLASH_BASE) as usize + app.length as usize;
        let junk = image
            .get(end..)
            .map_or(0, |tail| tail.iter().filter(|&&b| b != 0xFF).count());
        if junk > 0 {
            return Err(format!(
                "{junk} bytes of non-0xFF data follow the stamped application image — not a build-firmware.sh output; refusing to write it through ROM DFU"
            ));
        }
    }
    Ok(())
}

/// The decision matrix. Pure so every branch is unit-tested; `run_update`
/// only gathers the inputs and executes the answer.
///
/// The principles: a DFU device next to any pad is refused (a ROM
/// bootloader has no identity — it may be unrelated hardware); two
/// bootloader-mode pads are refused (no way to pick); ROM DFU is entered
/// only for whole-flash images (combined or legacy) — an app-only image
/// through ROM DFU would overwrite the bootloader; a legacy image is refused
/// for a pad that has a bootloader (it would replace the bootloader with an
/// unverifiable image, undoing the brick protection); a pad without a
/// bootloader can only take whole-flash images.
///
/// Two more refusals happen here rather than after the pad was rebooted
/// into its bootloader (where they would leave it parked in update mode
/// with an error): the bootloader path needs a *stamped* application (the
/// bootloader boots nothing whose CRC it cannot check), and an image linked
/// for another application base than the one the pad's BOOT_INFO reports
/// is for a different bootloader. An unstamped application stays
/// acceptable for ROM DFU (it boots on its vector checks, as designed for
/// debugger builds). The base check is repeated after enumeration for the
/// resume path, where the bootloader's BOOT_INFO is the first one seen.
pub fn plan_update(image: &ImageKind, pads: &PadTopology) -> Result<UpdatePlan, String> {
    let any_pad = pads.app_pad.is_some() || !pads.boot_pads.is_empty();
    if pads.dfu_devices > 0 && any_pad {
        return Err(
            "an OpenMicro pad and a separate STM32 DFU device are both connected; disconnect the unrelated DFU device"
                .into(),
        );
    }
    if pads.boot_pads.len() > 1 {
        return Err(
            "more than one pad is in bootloader mode; connect only the pad to update".into(),
        );
    }
    if pads.app_pad.is_some() && !pads.boot_pads.is_empty() {
        return Err(
            "two pads are connected (one of them in bootloader mode); connect only the pad to update"
                .into(),
        );
    }
    let hid = |enter_boot: bool, serial: &str, pad: Option<&BootInfoReply>| {
        let (image_base, validity) = image
            .app()
            .expect("the bootloader path is only planned for images with an application header");
        if let Validity::Unstamped(_) = validity {
            return Err(
                "the application image is not stamped (its header carries no length and CRC), and the bootloader boots nothing it cannot verify — stamp it with scripts/fw-image.py patch, or flash it with a debugger"
                    .to_string(),
            );
        }
        if let Some(pad) = pad {
            if pad.app_base != image_base {
                return Err(format!(
                    "the image places its firmware at 0x{image_base:08x} but the pad's bootloader expects 0x{:08x} — this image is for a different bootloader",
                    pad.app_base
                ));
            }
        }
        Ok(UpdatePlan::Hid {
            enter_boot,
            // hidapi reports no serial on some backends; "?" is the placeholder.
            expect_serial: (serial != "?").then(|| serial.to_string()),
        })
    };
    let app_pad = pads
        .app_pad
        .as_ref()
        .map(|(serial, boot)| (serial.as_str(), boot.as_ref()));
    let boot_pad = pads.boot_pads.first().map(String::as_str);
    let dfu = pads.dfu_devices > 0;
    match image {
        ImageKind::Combined { .. } => match (app_pad, boot_pad, dfu) {
            (Some((serial, Some(pad))), _, _) => hid(true, serial, Some(pad)),
            // A pad without a bootloader: this install is the migration.
            (Some((_, None)), _, _) => Ok(UpdatePlan::RomDfu),
            (None, Some(serial), _) => hid(false, serial, None),
            // A pad parked in ROM DFU by an earlier attempt: resume it.
            (None, None, true) => Ok(UpdatePlan::RomDfu),
            (None, None, false) => Err(not_found_message()),
        },
        ImageKind::AppOnly { .. } => match (app_pad, boot_pad, dfu) {
            (Some((serial, Some(pad))), _, _) => hid(true, serial, Some(pad)),
            (Some((_, None)), _, _) => Err(
                "this is an application-only image and the connected pad has no bootloader (firmware 0.9.0 or older): install a combined openmicro-fw image first"
                    .into(),
            ),
            (None, Some(serial), _) => hid(false, serial, None),
            (None, None, true) => Err(
                "this is an application-only image; flashing it through ROM DFU would overwrite the bootloader — use a combined openmicro-fw image"
                    .into(),
            ),
            (None, None, false) => Err(not_found_message()),
        },
        ImageKind::Legacy => match (app_pad, boot_pad, dfu) {
            (Some((_, Some(_))), _, _) | (None, Some(_), _) => Err(
                "this image predates the bootloader (it has no application header) and would replace the bootloader with an unverifiable whole-flash image; use firmware 0.10.0 or newer for this pad"
                    .into(),
            ),
            (Some((_, None)), _, _) | (None, None, true) => Ok(UpdatePlan::RomDfu),
            (None, None, false) => Err(not_found_message()),
        },
    }
}

fn not_found_message() -> String {
    if cfg!(target_os = "windows") {
        "device not found. Plug the pad in; if Device Manager shows STM32 BOOTLOADER (0483:df11), use DFU driver setup to bind it to WinUSB, then retry Install."
            .into()
    } else {
        "device not found (and no pad in bootloader or DFU mode) — plug the pad in".into()
    }
}

fn describe_kind(kind: &ImageKind) -> String {
    let stamp = |app: &Validity| match app {
        Validity::Stamped(_) => "",
        Validity::Unstamped(_) => " (unstamped)",
    };
    match kind {
        ImageKind::Combined { boot, app } => format!(
            "combined image: bootloader {} + firmware {}{} at 0x{:08x}",
            boot.version_str(),
            app.header().version_str(),
            stamp(app),
            boot.app_base
        ),
        ImageKind::AppOnly { app_base, app } => format!(
            "application-only image: firmware {}{} for 0x{app_base:08x}",
            app.header().version_str(),
            stamp(app)
        ),
        ImageKind::Legacy => "legacy whole-flash image (no header)".into(),
    }
}

/// The whole update: sanity-check the image, work out which path applies
/// to the pads on the bus, run it, wait for the app to come back.
fn run_update(api: &mut HidApi, image_path: &PathBuf, expected_version: Option<&str>) {
    let phase = |s: &str| events::post(UpdateMsg::Phase(s.to_string()));
    let log = |s: String| events::post(UpdateMsg::Log(s));
    let fail = |s: String| events::post(UpdateMsg::Failed(s));

    // -- image sanity --
    let image = match std::fs::read(image_path) {
        Ok(b) => b,
        Err(e) => return fail(format!("cannot read image: {e}")),
    };
    if image.len() < 192 || image.len() > MAX_FIRMWARE_LEN {
        return fail(format!(
            "image is {} bytes — firmware must end below the data pages at 0x{DATA_BASE:08x} (max {MAX_FIRMWARE_LEN} bytes)",
            image.len()
        ));
    }
    let kind = match classify_image(&image) {
        Ok(kind) => kind,
        Err(e) => return fail(e),
    };
    // Whole-flash images (bootloader+app, or legacy) may go through ROM DFU
    // at 0x08000000, so their first two words must be a Cortex-M0 vector
    // table for that address: SP in the F072CB's 16 KiB SRAM, a Thumb reset
    // vector below the config page at 0x0801_F800. (An app-only image was
    // checked for its own base by `classify_image`.)
    if !matches!(kind, ImageKind::AppOnly { .. }) {
        let sp = u32::from_le_bytes(image[0..4].try_into().unwrap());
        let rv_raw = u32::from_le_bytes(image[4..8].try_into().unwrap());
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
    }
    log(format!(
        "image: {} ({} bytes) — {}",
        image_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("?"),
        image.len(),
        describe_kind(&kind)
    ));

    // -- who is on the bus --
    let app_pad = open_raw_with_serial(api).map(|(dev, serial)| {
        let version = query_version(&dev).unwrap_or_else(|| "?".into());
        let boot = if probes_boot_info(&version) {
            query_boot_info(&dev)
        } else {
            None
        };
        (dev, serial, version, boot)
    });
    let boot_pads = boot::find_bootloaders(api);
    let dfu_device = match dfuse::find_bootloader() {
        Ok(device) => device,
        Err(e) => return fail(e),
    };
    let topology = PadTopology {
        app_pad: app_pad
            .as_ref()
            .map(|(_, serial, _, boot)| (serial.clone(), *boot)),
        boot_pads: boot_pads.iter().map(|(_, serial)| serial.clone()).collect(),
        dfu_devices: usize::from(dfu_device.is_some()),
    };
    if let Some((_, serial, version, boot)) = &app_pad {
        log(format!(
            "pad {serial}: firmware {version}, {}",
            match boot {
                Some(info) => format!(
                    "bootloader {} (app base 0x{:08x})",
                    boot::version_string(info.boot_version),
                    info.app_base
                ),
                None => "no bootloader".to_string(),
            }
        ));
    }
    let plan = match plan_update(&kind, &topology) {
        Ok(plan) => plan,
        Err(e) => return fail(e),
    };

    match plan {
        UpdatePlan::RomDfu => {
            if let Err(e) = check_rom_dfu_image(&image, &kind) {
                return fail(e);
            }
            match app_pad {
                Some((dev, ..)) => {
                    log("update path: ROM DFU (one-time bootloader install)".into());
                    phase("Rebooting the pad into ROM DFU mode (one-time bootloader install)…");
                    if let Err(e) = enter_dfu(&dev) {
                        return fail(format!("enter DFU: {e}"));
                    }
                    drop(dev);
                }
                None => log("DFU bootloader already present — resuming recovery".into()),
            }
            run_rom_dfu(api, &image, expected_version)
        }
        UpdatePlan::Hid {
            enter_boot: reboot_first,
            expect_serial,
        } => {
            log("update path: bootloader over USB HID".into());
            let (image_app_base, app_image): (u32, &[u8]) = match kind {
                ImageKind::Combined { boot, .. } => (
                    boot.app_base,
                    &image[(boot.app_base - FLASH_BASE) as usize..],
                ),
                ImageKind::AppOnly { app_base, .. } => (app_base, &image),
                ImageKind::Legacy => {
                    return fail("internal: a legacy image cannot take the bootloader path".into())
                }
            };
            if reboot_first {
                let Some((dev, ..)) = app_pad else {
                    return fail("internal: no pad to reboot into the bootloader".into());
                };
                phase("Rebooting the pad into its bootloader…");
                if let Err(e) = enter_boot(&dev) {
                    return fail(format!("enter bootloader: {e}"));
                }
                drop(dev);
            } else {
                drop(app_pad);
            }
            run_hid_update(
                api,
                app_image,
                image_app_base,
                expect_serial.as_deref(),
                expected_version,
            )
        }
    }
}

/// What one look at the bus means while waiting for the bootloader.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BootWaitStep {
    /// Nothing usable yet; keep polling.
    Wait,
    /// The expected bootloader is at this index of the bootloader list.
    Open(usize),
    /// The application pad went away and came back: the pad reset, but its
    /// bootloader started the firmware instead of staying in update mode.
    /// That is a bootloader fault (a panic or HardFault in update mode
    /// escalates to a reset); waiting longer cannot help.
    AppReturned,
}

/// The pure half of `run_hid_update`'s wait loop: which bootloader to
/// open, when to give up early, and what to say at the deadline.
#[derive(Debug, Default)]
pub struct BootWait {
    /// Set once a probe showed no application pad. Right after the
    /// ENTER_BOOT ack the old enumeration lingers for a moment, so only an
    /// application pad seen *after* the gap counts as "came back".
    app_gone: bool,
    /// A bootloader with a different serial was seen.
    seen_other: bool,
}

impl BootWait {
    /// `boot_serials`: every bootloader-mode pad on the bus; `app_serial`:
    /// the application-mode pad, if one is enumerated; `expect_serial`: the
    /// pad being updated (None when its serial is unknown, in which case
    /// any bootloader — and any application pad — is taken to be it).
    pub fn observe(
        &mut self,
        boot_serials: &[&str],
        app_serial: Option<&str>,
        expect_serial: Option<&str>,
    ) -> BootWaitStep {
        let candidate = match expect_serial {
            Some(serial) => {
                self.seen_other |= boot_serials.iter().any(|s| *s != serial);
                boot_serials.iter().position(|s| *s == serial)
            }
            None => (!boot_serials.is_empty()).then_some(0),
        };
        if let Some(index) = candidate {
            return BootWaitStep::Open(index);
        }
        let app_here = match (app_serial, expect_serial) {
            (Some(app), Some(expected)) => app == expected,
            (Some(_), None) => true,
            (None, _) => false,
        };
        if !app_here {
            self.app_gone = true;
            BootWaitStep::Wait
        } else if self.app_gone {
            BootWaitStep::AppReturned
        } else {
            BootWaitStep::Wait
        }
    }

    /// The failure to report when the deadline passed. `open_error`: the
    /// last error from opening a bootloader that did enumerate.
    pub fn timeout_message(
        &self,
        open_error: Option<&str>,
        expect_serial: Option<&str>,
        waited: Duration,
    ) -> String {
        match open_error {
            // Enumerated but not openable: almost always permissions.
            Some(e) => format!(
                "the bootloader (1209:0002) is connected but could not be opened: {e}{}",
                if cfg!(target_os = "linux") {
                    " — add the udev rule for 1209:0002 (docs/linux-firmware-updates.md) and retry Install"
                } else {
                    " — retry Install"
                }
            ),
            None if self.seen_other => format!(
                "a bootloader appeared, but not this pad's (serial {}); connect only the pad to update",
                expect_serial.unwrap_or("?")
            ),
            None if !self.app_gone => {
                "the pad acknowledged ENTER_BOOT but never left application mode — unplug it, plug it back in while holding the encoder switch, then retry Install"
                    .into()
            }
            None => format!(
                "the bootloader (1209:0002) did not enumerate within {} s — if the pad now shows up as PAD IN BOOTLOADER MODE, use Install there; otherwise unplug the pad, plug it back in while holding the encoder switch, then retry Install",
                waited.as_secs()
            ),
        }
    }
}

/// The failure for [`BootWaitStep::AppReturned`].
const BOOTLOADER_FAULT_MESSAGE: &str = "the pad restarted into its firmware instead of the bootloader (bootloader fault) — use Advanced > Reinstall bootloader via ROM DFU with the combined image";

/// The bootloader path: wait for the update-mode interface, check it is the
/// pad we rebooted, upload the application slice, start it, wait for the
/// application to come back.
fn run_hid_update(
    api: &mut HidApi,
    app_image: &[u8],
    image_app_base: u32,
    expect_serial: Option<&str>,
    expected_version: Option<&str>,
) {
    let phase = |s: &str| events::post(UpdateMsg::Phase(s.to_string()));
    let log = |s: String| events::post(UpdateMsg::Log(s));
    let fail = |s: String| events::post(UpdateMsg::Failed(s));

    phase("Waiting for the bootloader…");
    let deadline = Instant::now() + BOOT_APPEAR_TIMEOUT;
    let mut open_error: Option<String> = None;
    let mut wait = BootWait::default();
    let mut dev = loop {
        let _ = api.refresh_devices();
        let found = boot::find_bootloaders(api);
        let boot_serials: Vec<&str> = found.iter().map(|(_, s)| s.as_str()).collect();
        let app_serial = find_raw(api).map(|(_, serial)| serial);
        match wait.observe(&boot_serials, app_serial.as_deref(), expect_serial) {
            BootWaitStep::Open(index) => match boot::BootDevice::open_path(api, &found[index].0) {
                Ok(dev) => break dev,
                Err(e) => open_error = Some(e),
            },
            BootWaitStep::AppReturned => return fail(BOOTLOADER_FAULT_MESSAGE.into()),
            BootWaitStep::Wait => {}
        }
        if Instant::now() > deadline {
            return fail(wait.timeout_message(
                open_error.as_deref(),
                expect_serial,
                BOOT_APPEAR_TIMEOUT,
            ));
        }
        std::thread::sleep(BOOT_APPEAR_POLL);
    };

    let info = match dev.info() {
        Ok(info) => info,
        Err(e) => return fail(format!("bootloader: {e}")),
    };
    log(format!(
        "bootloader {} · app base 0x{:08x} · firmware {} ({})",
        boot::version_string(info.boot_version),
        info.app_base,
        if info.app_version_str().is_empty() {
            "-"
        } else {
            info.app_version_str()
        },
        boot::describe_app_valid(info.app_valid)
    ));
    // `plan_update` already compared the two when the pad was in
    // application mode; on the resume path this is the first BOOT_INFO.
    if image_app_base != info.app_base {
        return fail(format!(
            "the image places its firmware at 0x{image_app_base:08x} but the pad's bootloader expects 0x{:08x} — this image is for a different bootloader",
            info.app_base
        ));
    }

    phase("Uploading firmware…");
    let mut progress = |percent: u8| {
        events::post(UpdateMsg::Progress(f64::from(percent) / 100.0));
        events::post(UpdateMsg::Phase(format!("Uploading firmware… {percent}%")));
    };
    let report = match dev.upload(app_image, &mut progress) {
        Ok(report) => report,
        // Nothing is lost: the bootloader keeps the pad in update mode until
        // a complete, verified image is in place.
        Err(e) => {
            return fail(format!(
                "upload failed: {e} — the pad stays in bootloader mode; retry Install"
            ))
        }
    };
    log(format!(
        "uploaded {} bytes in {} chunks{}{} · verified by the bootloader",
        report.length,
        report.chunks,
        if report.resumed > 0 {
            format!(", {} resumed", report.resumed)
        } else {
            String::new()
        },
        if report.resyncs > 0 {
            format!(", {} re-synced", report.resyncs)
        } else {
            String::new()
        },
    ));

    phase("Starting the new firmware…");
    events::post(UpdateMsg::Progress(1.0));
    if let Err(e) = dev.run() {
        return fail(format!(
            "start firmware: {e} — the pad stays in bootloader mode"
        ));
    }
    drop(dev);
    wait_for_app(api, expected_version)
}

/// The pre-bootloader path, unchanged: wait for 0483:df11, DfuSe the whole
/// image at 0x08000000, wait for the application.
fn run_rom_dfu(api: &mut HidApi, image: &[u8], expected_version: Option<&str>) {
    let phase = |s: &str| events::post(UpdateMsg::Phase(s.to_string()));
    let fail = |s: String| events::post(UpdateMsg::Failed(s));

    phase("Waiting for the DFU bootloader…");
    let deadline = Instant::now() + Duration::from_secs(8);
    let dfu = loop {
        match dfuse::find_bootloader() {
            Ok(Some(device)) => break device,
            Ok(None) => {}
            Err(e) => return fail(e),
        }
        if Instant::now() > deadline {
            #[cfg(target_os = "windows")]
            return fail(dfuse::windows_driver_required(
                "The pad entered ROM DFU mode, but the bootloader did not become accessible.",
            ));
            #[cfg(not(target_os = "windows"))]
            return fail("DFU bootloader (0483:df11) never enumerated".into());
        }
        std::thread::sleep(Duration::from_millis(200));
    };

    // -- flash --
    if let Err(error) = dfuse::flash(dfu, image, |p, frac| {
        events::post(UpdateMsg::Phase(p.to_string()));
        events::post(UpdateMsg::Progress(frac));
    }) {
        if dfuse::is_windows_driver_required(&error) {
            return fail(error);
        }
        return fail(format!(
            "DFU flashing failed: {error} — recovery: SWD on J2"
        ));
    }

    wait_for_app(api, expected_version)
}

/// After either path: the pad resets, its bootloader validates and starts
/// the application, and the application enumerates — well under a second
/// on the HID path, so the historical 10 s budget has plenty of margin.
fn wait_for_app(api: &mut HidApi, expected_version: Option<&str>) {
    let phase = |s: &str| events::post(UpdateMsg::Phase(s.to_string()));
    let fail = |s: String| events::post(UpdateMsg::Failed(s));

    phase("Waiting for the pad to come back…");
    let deadline = Instant::now() + APP_RETURN_TIMEOUT;
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
            return fail(
                "flashed OK, but the device did not re-enumerate — if it is back in bootloader mode, retry Install"
                    .into(),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use openmicro_layout::{
        app_header_bytes, app_valid, boot_info_bytes, stamp_app, version_bytes, APP_BASE, APP_SIZE,
        BOOT_SIZE, RAM_END, VARIANT_PROD,
    };

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

    // ---- versions -------------------------------------------------------

    #[test]
    fn version_parsing_strips_prefix_and_prerelease() {
        assert_eq!(parse_version("0.10.0"), Some((0, 10, 0)));
        assert_eq!(parse_version("0.10.0-rc.1"), Some((0, 10, 0)));
        assert_eq!(parse_version("v1.2.3"), Some((1, 2, 3)));
        assert_eq!(parse_version("?"), None);
        assert_eq!(parse_version("0.10"), None);
        assert_eq!(parse_version(""), None);
    }

    #[test]
    fn feature_probes_follow_the_version_thresholds() {
        assert!(!supports_device_mode("0.7.9"));
        assert!(supports_device_mode("0.8.0"));
        assert!(supports_device_mode("1.0.0"));
        assert!(!supports_device_mode("?"));

        assert!(!probes_boot_info("0.9.0"));
        assert!(probes_boot_info("0.10.0"));
        assert!(probes_boot_info("0.10.0-rc.1"));
        assert!(probes_boot_info("1.0.0"));
        // Unknown means "ask the pad", never "assume old".
        assert!(probes_boot_info("?"));
    }

    // ---- BOOT_INFO on the application interface --------------------------

    fn app_boot_info(status: u8, protocol: u8) -> [u8; 32] {
        // The application answers in a 32-byte report: the 31-byte reply
        // plus one byte of padding, never the page/chunk extension.
        let mut report = [0u8; 32];
        let n = BootInfoReply {
            status,
            protocol,
            boot_version: [1, 0, 0],
            app_base: APP_BASE,
            app_size: APP_SIZE,
            app_valid: app_valid::VALID,
            app_version: version_bytes("0.10.0"),
            page: None,
            max_chunk: None,
        }
        .encode(&mut report);
        assert_eq!(n, 31);
        report
    }

    #[test]
    fn boot_info_reply_from_the_app_interface_parses_without_the_extension() {
        let report = app_boot_info(0, 1);
        let info = accept_boot_info(&report).expect("accepted");
        assert_eq!(info.boot_version, [1, 0, 0]);
        assert_eq!(info.app_base, APP_BASE);
        assert_eq!(info.app_size, APP_SIZE);
        assert_eq!(info.app_valid, app_valid::VALID);
        assert_eq!(info.app_version_str(), "0.10.0");
        assert_eq!(info.page, None);
        assert_eq!(info.max_chunk, None);
        // Exactly 31 bytes (a backend that trims the report) works too.
        assert_eq!(accept_boot_info(&report[..31]), Some(info));
        // A truncated report does not.
        assert_eq!(accept_boot_info(&report[..30]), None);
    }

    #[test]
    fn boot_info_needs_status_zero_and_protocol_one() {
        // Status 1: a 0.10 app without a bootloader under it.
        assert_eq!(accept_boot_info(&app_boot_info(1, 1)), None);
        // A future protocol this app cannot drive.
        assert_eq!(accept_boot_info(&app_boot_info(0, 2)), None);
        // The wrong opcode altogether.
        let mut report = app_boot_info(0, 1);
        report[0] = CMD_VERSION;
        assert_eq!(accept_boot_info(&report), None);
    }

    // ---- image classification ---------------------------------------------

    fn app_image(app_base: u32, len: usize, version: &str, stamped: bool) -> Vec<u8> {
        let mut img = vec![0u8; len];
        img[0..4].copy_from_slice(&(RAM_END - 16).to_le_bytes());
        img[4..8].copy_from_slice(&((app_base + RESET_OFFSET) | 1).to_le_bytes());
        img[HEADER_OFFSET as usize..RESET_OFFSET as usize]
            .copy_from_slice(&app_header_bytes(version, 0));
        for (i, b) in img[RESET_OFFSET as usize..].iter_mut().enumerate() {
            *b = (i * 3 + 1) as u8;
        }
        if stamped {
            let padded = (img.len() + 3) & !3;
            img.resize(padded, 0xFF);
            stamp_app(&mut img, padded).unwrap();
        }
        img
    }

    fn legacy_image(len: usize) -> Vec<u8> {
        let mut img = vec![0x11u8; len];
        img[0..4].copy_from_slice(&0x2000_4000u32.to_le_bytes());
        img[4..8].copy_from_slice(&0x0800_00C1u32.to_le_bytes());
        img
    }

    fn combined_image(app: &[u8]) -> Vec<u8> {
        let mut img = vec![0xFFu8; BOOT_SIZE as usize];
        img[0..4].copy_from_slice(&0x2000_3FF0u32.to_le_bytes());
        img[4..8].copy_from_slice(&((FLASH_BASE + RESET_OFFSET) | 1).to_le_bytes());
        img[HEADER_OFFSET as usize..RESET_OFFSET as usize]
            .copy_from_slice(&boot_info_bytes("1.0.0", VARIANT_PROD));
        img.extend_from_slice(app);
        img
    }

    #[test]
    fn images_are_recognised_by_their_header_block() {
        assert_eq!(classify_image(&legacy_image(1000)), Ok(ImageKind::Legacy));
        match classify_image(&app_image(APP_BASE, 900, "0.10.0", true)) {
            Ok(ImageKind::AppOnly {
                app_base,
                app: Validity::Stamped(app),
            }) => {
                assert_eq!(app_base, APP_BASE);
                assert_eq!(app.version_str(), "0.10.0");
                assert_eq!(app.length, 900);
            }
            other => panic!("{other:?}"),
        }
        // An application-only image linked for another slot keeps its base.
        match classify_image(&app_image(APP_BASE + 0x800, 900, "0.10.0", false)) {
            Ok(ImageKind::AppOnly {
                app_base,
                app: Validity::Unstamped(_),
            }) => assert_eq!(app_base, APP_BASE + 0x800),
            other => panic!("{other:?}"),
        }
        let combined = combined_image(&app_image(APP_BASE, 900, "0.10.0", true));
        match classify_image(&combined) {
            Ok(ImageKind::Combined {
                boot,
                app: Validity::Stamped(app),
            }) => {
                assert_eq!(boot.version_str(), "1.0.0");
                assert_eq!(boot.app_base, APP_BASE);
                assert_eq!(app.version_str(), "0.10.0");
                assert_eq!(app.length, 900);
            }
            other => panic!("{other:?}"),
        }
        // Unstamped application (a probe-rs style build) still classifies,
        // and says so.
        let combined = combined_image(&app_image(APP_BASE, 900, "0.10.0", false));
        assert!(matches!(
            classify_image(&combined),
            Ok(ImageKind::Combined {
                app: Validity::Unstamped(_),
                ..
            })
        ));
        assert!(describe_kind(&classify_image(&combined).unwrap()).contains("unstamped"));
        // Too short to carry a header.
        assert!(classify_image(&[0u8; 0x40])
            .unwrap_err()
            .contains("shorter"));
    }

    #[test]
    fn broken_combined_images_are_refused_up_front() {
        // Truncated before the application slot.
        let mut short = combined_image(&[]);
        short.truncate(0x1000);
        assert!(classify_image(&short).unwrap_err().contains("ends before"));
        // A corrupted application (CRC).
        let mut bad = combined_image(&app_image(APP_BASE, 900, "0.10.0", true));
        bad[BOOT_SIZE as usize + 600] ^= 0x01;
        let err = classify_image(&bad).unwrap_err();
        assert!(err.contains("CRC"), "{err}");
        // An application linked for the wrong base.
        let wrong = combined_image(&app_image(APP_BASE + 0x800, 900, "0.10.0", true));
        assert!(classify_image(&wrong).unwrap_err().contains("reset vector"));
    }

    #[test]
    fn broken_app_only_images_are_refused_up_front() {
        // A corrupted stamped application.
        let mut bad = app_image(APP_BASE, 900, "0.10.0", true);
        bad[600] ^= 0x01;
        let err = classify_image(&bad).unwrap_err();
        assert!(err.contains("CRC"), "{err}");
        // A reset vector that points nowhere an application slot can be:
        // the bootloader's own pages, an unaligned address, an even one.
        for reset in [
            (FLASH_BASE + RESET_OFFSET) | 1,
            (APP_BASE + 0x100 + RESET_OFFSET) | 1,
            APP_BASE + RESET_OFFSET,
        ] {
            let mut img = app_image(APP_BASE, 900, "0.10.0", true);
            img[4..8].copy_from_slice(&reset.to_le_bytes());
            let err = classify_image(&img).unwrap_err();
            assert!(err.contains("reset vector"), "{reset:08x}: {err}");
        }
    }

    #[test]
    fn rom_dfu_refuses_images_that_reach_the_data_pages_or_carry_junk() {
        let clean = combined_image(&app_image(APP_BASE, 900, "0.10.0", true));
        let kind = classify_image(&clean).unwrap();
        assert_eq!(check_rom_dfu_image(&clean, &kind), Ok(()));
        // Erased-flash padding after the application is fine (a .bin padded
        // to a page boundary).
        let mut padded = clean.clone();
        padded.resize(clean.len() + 1000, 0xFF);
        assert_eq!(check_rom_dfu_image(&padded, &kind), Ok(()));
        // Anything else after the stamped length is not.
        let mut junk = clean.clone();
        junk.extend_from_slice(&[0x00, 0xFF, 0x12]);
        let err = check_rom_dfu_image(&junk, &kind).unwrap_err();
        assert!(err.contains("2 bytes of non-0xFF"), "{err}");
        // Longer than the space below the data pages, whatever it holds.
        let mut long = clean.clone();
        long.resize(MAX_FIRMWARE_LEN + 1, 0xFF);
        let err = check_rom_dfu_image(&long, &kind).unwrap_err();
        assert!(err.contains("data pages"), "{err}");
        let mut legacy = legacy_image(1000);
        legacy.resize(MAX_FIRMWARE_LEN + 4, 0x11);
        assert!(check_rom_dfu_image(&legacy, &ImageKind::Legacy).is_err());
        assert_eq!(
            check_rom_dfu_image(&legacy_image(1000), &ImageKind::Legacy),
            Ok(())
        );
        // An unstamped application has no length: only the size rule applies.
        let mut unstamped = combined_image(&app_image(APP_BASE, 900, "0.10.0", false));
        let kind = classify_image(&unstamped).unwrap();
        unstamped.extend_from_slice(&[0x00, 0x12]);
        assert_eq!(check_rom_dfu_image(&unstamped, &kind), Ok(()));
        assert_eq!(MAX_FIRMWARE_LEN, 0x1B000);
    }

    // ---- the decision matrix ----------------------------------------------

    fn header(version: &str) -> AppHeader {
        AppHeader::parse(&app_header_bytes(version, 0)).unwrap()
    }

    fn stamped(version: &str) -> Validity {
        Validity::Stamped(AppHeader {
            length: 900,
            crc: 0x1234_5678,
            ..header(version)
        })
    }

    fn combined() -> ImageKind {
        ImageKind::Combined {
            boot: BootInfo::parse(&boot_info_bytes("1.0.0", VARIANT_PROD)).unwrap(),
            app: stamped("0.10.0"),
        }
    }

    fn app_only() -> ImageKind {
        ImageKind::AppOnly {
            app_base: APP_BASE,
            app: stamped("0.10.0"),
        }
    }

    fn boot_reply() -> BootInfoReply {
        accept_boot_info(&app_boot_info(0, 1)).unwrap()
    }

    /// `app`: an application-mode pad as (serial, has bootloader).
    fn pads(app: Option<(&str, bool)>, boots: &[&str], dfu_devices: usize) -> PadTopology {
        PadTopology {
            app_pad: app.map(|(serial, has_boot)| (serial.to_string(), has_boot.then(boot_reply))),
            boot_pads: boots.iter().map(|s| s.to_string()).collect(),
            dfu_devices,
        }
    }

    fn hid(enter_boot: bool, serial: &str) -> UpdatePlan {
        UpdatePlan::Hid {
            enter_boot,
            expect_serial: Some(serial.into()),
        }
    }

    #[test]
    fn combined_image_plans() {
        let image = combined();
        assert_eq!(
            plan_update(&image, &pads(Some(("A", true)), &[], 0)),
            Ok(hid(true, "A"))
        );
        // The one-time migration for a pad without a bootloader.
        assert_eq!(
            plan_update(&image, &pads(Some(("A", false)), &[], 0)),
            Ok(UpdatePlan::RomDfu)
        );
        // Resume a pad parked in its bootloader.
        assert_eq!(
            plan_update(&image, &pads(None, &["B"], 0)),
            Ok(hid(false, "B"))
        );
        // Resume a pad parked in ROM DFU by an interrupted migration.
        assert_eq!(
            plan_update(&image, &pads(None, &[], 1)),
            Ok(UpdatePlan::RomDfu)
        );
        let err = plan_update(&image, &pads(None, &[], 0)).unwrap_err();
        assert!(err.contains("device not found"), "{err}");
    }

    #[test]
    fn app_only_image_plans() {
        let image = app_only();
        assert_eq!(
            plan_update(&image, &pads(Some(("A", true)), &[], 0)),
            Ok(hid(true, "A"))
        );
        assert_eq!(
            plan_update(&image, &pads(None, &["B"], 0)),
            Ok(hid(false, "B"))
        );
        // Never through ROM DFU: it would land on the bootloader's pages.
        let err = plan_update(&image, &pads(Some(("A", false)), &[], 0)).unwrap_err();
        assert!(err.contains("no bootloader"), "{err}");
        let err = plan_update(&image, &pads(None, &[], 1)).unwrap_err();
        assert!(err.contains("overwrite the bootloader"), "{err}");
        let err = plan_update(&image, &pads(None, &[], 0)).unwrap_err();
        assert!(err.contains("device not found"), "{err}");
    }

    #[test]
    fn unstamped_applications_never_take_the_bootloader_path() {
        let unstamped_combined = ImageKind::Combined {
            boot: BootInfo::parse(&boot_info_bytes("1.0.0", VARIANT_PROD)).unwrap(),
            app: Validity::Unstamped(header("0.10.0")),
        };
        let unstamped_app = ImageKind::AppOnly {
            app_base: APP_BASE,
            app: Validity::Unstamped(header("0.10.0")),
        };
        // Refused before the pad is rebooted, and on the resume path.
        for image in [&unstamped_combined, &unstamped_app] {
            for topology in [pads(Some(("A", true)), &[], 0), pads(None, &["B"], 0)] {
                let err = plan_update(image, &topology).unwrap_err();
                assert!(err.contains("not stamped"), "{err}");
            }
        }
        // ROM DFU installs it whole: the bootloader boots an unstamped
        // application on its vector checks (debugger builds).
        assert_eq!(
            plan_update(&unstamped_combined, &pads(Some(("A", false)), &[], 0)),
            Ok(UpdatePlan::RomDfu)
        );
        assert_eq!(
            plan_update(&unstamped_combined, &pads(None, &[], 1)),
            Ok(UpdatePlan::RomDfu)
        );
    }

    #[test]
    fn images_for_another_application_base_are_refused_before_enter_boot() {
        let other_base = ImageKind::Combined {
            boot: BootInfo {
                app_base: APP_BASE + 0x1000,
                ..BootInfo::parse(&boot_info_bytes("1.0.0", VARIANT_PROD)).unwrap()
            },
            app: stamped("0.10.0"),
        };
        let other_app = ImageKind::AppOnly {
            app_base: APP_BASE + 0x1000,
            app: stamped("0.10.0"),
        };
        for image in [&other_base, &other_app] {
            let err = plan_update(image, &pads(Some(("A", true)), &[], 0)).unwrap_err();
            assert!(err.contains("different bootloader"), "{err}");
            assert!(
                err.contains("0x08007000") && err.contains("0x08006000"),
                "{err}"
            );
            // A pad already in bootloader mode has not been asked yet: the
            // check happens once its BOOT_INFO is read.
            assert_eq!(
                plan_update(image, &pads(None, &["B"], 0)),
                Ok(hid(false, "B"))
            );
        }
        // The migration path does not care: ROM DFU writes the whole image.
        assert_eq!(
            plan_update(&other_base, &pads(Some(("A", false)), &[], 0)),
            Ok(UpdatePlan::RomDfu)
        );
    }

    #[test]
    fn legacy_image_plans() {
        let image = ImageKind::Legacy;
        // The pre-bootloader flow for pre-bootloader pads is unchanged.
        assert_eq!(
            plan_update(&image, &pads(Some(("A", false)), &[], 0)),
            Ok(UpdatePlan::RomDfu)
        );
        assert_eq!(
            plan_update(&image, &pads(None, &[], 1)),
            Ok(UpdatePlan::RomDfu)
        );
        // But never onto a pad that has a bootloader.
        for topology in [pads(Some(("A", true)), &[], 0), pads(None, &["B"], 0)] {
            let err = plan_update(&image, &topology).unwrap_err();
            assert!(err.contains("predates the bootloader"), "{err}");
        }
        let err = plan_update(&image, &pads(None, &[], 0)).unwrap_err();
        assert!(err.contains("device not found"), "{err}");
    }

    #[test]
    fn ambiguous_topologies_are_refused_for_every_image() {
        for image in [combined(), app_only(), ImageKind::Legacy] {
            // A DFU device next to any pad could be unrelated hardware.
            for topology in [
                pads(Some(("A", true)), &[], 1),
                pads(Some(("A", false)), &[], 1),
                pads(None, &["B"], 1),
            ] {
                let err = plan_update(&image, &topology).unwrap_err();
                assert!(err.contains("unrelated DFU device"), "{err}");
            }
            let err = plan_update(&image, &pads(None, &["B", "C"], 0)).unwrap_err();
            assert!(err.contains("more than one pad"), "{err}");
            let err = plan_update(&image, &pads(Some(("A", true)), &["B"], 0)).unwrap_err();
            assert!(err.contains("two pads"), "{err}");
        }
    }

    #[test]
    fn unknown_serials_do_not_pin_the_bootloader() {
        assert_eq!(
            plan_update(&combined(), &pads(Some(("?", true)), &[], 0)),
            Ok(UpdatePlan::Hid {
                enter_boot: true,
                expect_serial: None,
            })
        );
        assert_eq!(
            plan_update(&app_only(), &pads(None, &["?"], 0)),
            Ok(UpdatePlan::Hid {
                enter_boot: false,
                expect_serial: None,
            })
        );
    }

    // ---- waiting for the bootloader after ENTER_BOOT -------------------------

    #[test]
    fn the_wait_opens_the_expected_bootloader_once_it_enumerates() {
        let mut wait = BootWait::default();
        // Right after the ack the application is still enumerated.
        assert_eq!(wait.observe(&[], Some("A"), Some("A")), BootWaitStep::Wait);
        // The pad resets: nothing on the bus for a moment.
        assert_eq!(wait.observe(&[], None, Some("A")), BootWaitStep::Wait);
        // Another pad's bootloader is not ours; ours is, wherever it is listed.
        assert_eq!(wait.observe(&["X"], None, Some("A")), BootWaitStep::Wait);
        assert_eq!(
            wait.observe(&["X", "A"], None, Some("A")),
            BootWaitStep::Open(1)
        );
        // With an unknown serial the first bootloader is taken.
        let mut wait = BootWait::default();
        assert_eq!(wait.observe(&[], Some("?"), None), BootWaitStep::Wait);
        assert_eq!(wait.observe(&["B"], None, None), BootWaitStep::Open(0));
    }

    #[test]
    fn the_wait_fails_fast_when_the_pad_comes_back_in_application_mode() {
        let mut wait = BootWait::default();
        assert_eq!(wait.observe(&[], Some("A"), Some("A")), BootWaitStep::Wait);
        assert_eq!(wait.observe(&[], None, Some("A")), BootWaitStep::Wait);
        // A different application pad is not ours.
        assert_eq!(wait.observe(&[], Some("Z"), Some("A")), BootWaitStep::Wait);
        // Ours, after the gap: the bootloader jumped to the firmware.
        assert_eq!(
            wait.observe(&[], Some("A"), Some("A")),
            BootWaitStep::AppReturned
        );
        // The lingering enumeration alone (no gap seen) is never a verdict.
        let mut wait = BootWait::default();
        for _ in 0..5 {
            assert_eq!(wait.observe(&[], Some("A"), Some("A")), BootWaitStep::Wait);
        }
        // Unknown serial: any application pad after the gap counts.
        let mut wait = BootWait::default();
        assert_eq!(wait.observe(&[], None, None), BootWaitStep::Wait);
        assert_eq!(
            wait.observe(&[], Some("?"), None),
            BootWaitStep::AppReturned
        );
        assert!(BOOTLOADER_FAULT_MESSAGE.contains("Reinstall bootloader via ROM DFU"));
    }

    #[test]
    fn the_wait_timeout_explains_what_it_saw() {
        let waited = Duration::from_secs(10);
        // Nothing ever showed up after the pad left.
        let mut wait = BootWait::default();
        wait.observe(&[], None, Some("A"));
        let msg = wait.timeout_message(None, Some("A"), waited);
        assert!(msg.contains("PAD IN BOOTLOADER MODE"), "{msg}");
        assert!(msg.contains("holding the encoder switch"), "{msg}");
        assert!(msg.contains("10 s"), "{msg}");
        // The application never left the bus.
        let mut wait = BootWait::default();
        wait.observe(&[], Some("A"), Some("A"));
        let msg = wait.timeout_message(None, Some("A"), waited);
        assert!(msg.contains("never left application mode"), "{msg}");
        // Some other pad's bootloader.
        let mut wait = BootWait::default();
        wait.observe(&["X"], None, Some("A"));
        let msg = wait.timeout_message(None, Some("A"), waited);
        assert!(msg.contains("not this pad's (serial A)"), "{msg}");
        // Enumerated but not openable wins over everything else.
        let msg = wait.timeout_message(Some("EACCES"), Some("A"), waited);
        assert!(msg.contains("could not be opened: EACCES"), "{msg}");
        assert_eq!(msg.contains("udev"), cfg!(target_os = "linux"));
        assert_eq!(
            BOOT_APPEAR_TIMEOUT.as_secs(),
            if cfg!(target_os = "windows") { 30 } else { 10 }
        );
    }

    // ---- announcing a pad in bootloader mode ----------------------------------

    #[test]
    fn a_bootloader_pad_is_announced_once_until_something_touches_it() {
        let mut announcer = BootloaderAnnouncer::default();
        assert_eq!(announcer.observe(None), Announce::Nothing);
        assert_eq!(announcer.observe(Some("A")), Announce::Query);
        assert_eq!(announcer.observe(Some("A")), Announce::Nothing);
        // Another pad replaces it.
        assert_eq!(announcer.observe(Some("B")), Announce::Query);
        assert_eq!(announcer.observe(Some("B")), Announce::Nothing);
        // An Install / Boot firmware ran: the same pad is queried again so
        // the card shows what the slot holds now.
        announcer.invalidate();
        assert_eq!(announcer.observe(Some("B")), Announce::Query);
        assert_eq!(announcer.observe(Some("B")), Announce::Nothing);
        // Leaving is reported exactly once — also right after a command.
        announcer.invalidate();
        assert_eq!(announcer.observe(None), Announce::Gone);
        assert_eq!(announcer.observe(None), Announce::Nothing);
        assert_eq!(announcer.observe(Some("B")), Announce::Query);
    }

    #[test]
    fn bootloader_errors_keep_the_udev_hint_for_open_failures_only() {
        let open = BootloaderError::Open("Permission denied".into()).to_string();
        assert!(
            open.contains("could not be opened: Permission denied"),
            "{open}"
        );
        assert_eq!(open.contains("udev"), cfg!(target_os = "linux"));
        let info = BootloaderError::Info("bootloader speaks protocol 2".into()).to_string();
        assert!(
            info.contains("BOOT_INFO failed: bootloader speaks protocol 2"),
            "{info}"
        );
        assert!(!info.contains("udev"));
    }
}
