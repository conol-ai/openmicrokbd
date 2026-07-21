//! The device worker thread: owns the process's one HidApi instance, watches
//! for the OpenMicro coming and going, and runs firmware updates end-to-end.
//!
//! Talks the firmware's raw-HID updater protocol (../fw/src/main.rs):
//!   OUT [0x01, ...]              -> IN [0x01, len, version ascii...]
//!   OUT [0x02, 'D','F','U','!']  -> IN [0x02, 0x01], device reboots into
//!                                   the ROM DFU bootloader (0483:df11).
//!
//! Everything is reported to the UI via `Cx::post_action`.

use hidapi::{HidApi, HidDevice};
use makepad_widgets::Cx;
use std::ffi::CString;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crate::dfuse;

pub const VID: u16 = 0x1209;
pub const PID: u16 = 0x0001;
const RAW_USAGE_PAGE: u16 = 0xFF60;

const CMD_VERSION: u8 = 0x01;
const CMD_ENTER_DFU: u8 = 0x02;

/// Posted to the UI whenever the device's presence or identity changes.
#[derive(Debug, Clone)]
pub enum DeviceMsg {
    Connected { version: String, serial: String },
    Disconnected,
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
    StartUpdate { image: PathBuf },
    EnterDfuOnly,
}

pub fn spawn_worker() -> mpsc::Sender<DeviceCmd> {
    let (tx, rx) = mpsc::channel::<DeviceCmd>();
    std::thread::spawn(move || {
        let mut api = match HidApi::new() {
            Ok(api) => api,
            Err(e) => {
                Cx::post_action(UpdateMsg::Failed(format!("HID init failed: {e}")));
                return;
            }
        };
        let mut connected = false;
        loop {
            match rx.recv_timeout(Duration::from_millis(1000)) {
                Ok(DeviceCmd::StartUpdate { image }) => {
                    run_update(&mut api, &image);
                    connected = false; // force a fresh Connected/Disconnected post
                }
                Ok(DeviceCmd::EnterDfuOnly) => {
                    match open_raw(&mut api) {
                        Some(dev) => match enter_dfu(&dev) {
                            Ok(()) => Cx::post_action(UpdateMsg::Log(
                                "device rebooted into DFU mode (0483:df11)".into(),
                            )),
                            Err(e) => {
                                Cx::post_action(UpdateMsg::Log(format!("enter DFU failed: {e}")))
                            }
                        },
                        None => Cx::post_action(UpdateMsg::Log("device not found".into())),
                    }
                    connected = false;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            }

            // Presence poll (edge-triggered posts only).
            let now = poll_presence(&mut api);
            if now != connected {
                connected = now;
                if !connected {
                    Cx::post_action(DeviceMsg::Disconnected);
                }
            }
        }
    });
    tx
}

/// Refresh the device list; if the pad is present post Connected (with a live
/// version query) and return true.
fn poll_presence(api: &mut HidApi) -> bool {
    let _ = api.refresh_devices();
    let Some((path, serial)) = find_raw(api) else {
        return false;
    };
    let Ok(dev) = api.open_path(&path) else {
        return false;
    };
    let version = query_version(&dev).unwrap_or_else(|| "?".into());
    Cx::post_action(DeviceMsg::Connected { version, serial });
    true
}

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

/// One command round-trip on the raw interface. hidapi wants a leading
/// report-ID byte (0x00 — the interface defines no report IDs).
fn command(dev: &HidDevice, cmd: &[u8], reply: &mut [u8; 32]) -> Result<usize, String> {
    let mut out = [0u8; 33];
    out[1..1 + cmd.len()].copy_from_slice(cmd);
    dev.write(&out).map_err(|e| e.to_string())?;
    dev.read_timeout(reply, 1000).map_err(|e| e.to_string())
}

fn query_version(dev: &HidDevice) -> Option<String> {
    let mut reply = [0u8; 32];
    let n = command(dev, &[CMD_VERSION], &mut reply).ok()?;
    if n < 2 || reply[0] != CMD_VERSION {
        return None;
    }
    let len = (reply[1] as usize).min(30);
    core::str::from_utf8(&reply[2..2 + len])
        .ok()
        .map(|s| s.to_string())
}

fn enter_dfu(dev: &HidDevice) -> Result<(), String> {
    let mut reply = [0u8; 32];
    let n = command(dev, &[CMD_ENTER_DFU, b'D', b'F', b'U', b'!'], &mut reply)?;
    if n >= 2 && reply[0] == CMD_ENTER_DFU && reply[1] == 0x01 {
        Ok(())
    } else {
        Err("device did not acknowledge the DFU command".into())
    }
}

/// The whole update: sanity-check the image, drop the device into the ROM
/// bootloader, flash over DFU, wait for the app to come back.
fn run_update(api: &mut HidApi, image_path: &PathBuf) {
    let phase = |s: &str| Cx::post_action(UpdateMsg::Phase(s.to_string()));
    let log = |s: String| Cx::post_action(UpdateMsg::Log(s));
    let fail = |s: String| Cx::post_action(UpdateMsg::Failed(s));

    // -- image sanity: a Cortex-M0 vector table for this exact chip --
    let image = match std::fs::read(image_path) {
        Ok(b) => b,
        Err(e) => return fail(format!("cannot read image: {e}")),
    };
    if image.len() < 192 || image.len() > 128 * 1024 {
        return fail(format!(
            "image is {} bytes — not a plausible 128K-flash firmware",
            image.len()
        ));
    }
    let sp = u32::from_le_bytes(image[0..4].try_into().unwrap());
    // Reset vector carries the thumb bit; mask it for the range check.
    let rv = u32::from_le_bytes(image[4..8].try_into().unwrap()) & !1;
    if !(0x2000_0000..=0x2000_8000).contains(&sp) || !(0x0800_0000..0x0802_0000).contains(&rv) {
        return fail(format!(
            "not an OpenMicro firmware image (SP={sp:08x} RV={rv:08x}) — expected a raw .bin for 0x08000000"
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

    // -- get the device into the bootloader (or find it already there) --
    if dfuse::find_bootloader().is_none() {
        phase("Rebooting the pad into DFU mode…");
        match open_raw(api) {
            Some(dev) => {
                if let Err(e) = enter_dfu(&dev) {
                    return fail(format!("enter DFU: {e}"));
                }
            }
            None => {
                return fail(
                    "device not found (and no DFU bootloader present) — plug the pad in".into(),
                )
            }
        }
    } else {
        log("DFU bootloader already present — resuming".into());
    }

    phase("Waiting for the DFU bootloader…");
    let deadline = Instant::now() + Duration::from_secs(8);
    let dfu = loop {
        if let Some(d) = dfuse::find_bootloader() {
            break d;
        }
        if Instant::now() > deadline {
            return fail("DFU bootloader (0483:df11) never enumerated".into());
        }
        std::thread::sleep(Duration::from_millis(200));
    };

    // -- flash --
    if let Err(e) = dfuse::flash(dfu, &image, |p, frac| {
        Cx::post_action(UpdateMsg::Phase(p.to_string()));
        Cx::post_action(UpdateMsg::Progress(frac));
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
            Cx::post_action(UpdateMsg::Done { version });
            return;
        }
        if Instant::now() > deadline {
            return fail("flashed OK, but the device did not re-enumerate".into());
        }
    }
}
