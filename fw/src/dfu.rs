//! Resets: back into the resident bootloader's ROM-DFU path, or plain.
//!
//! The board deliberately has no BOOT0 button — BOOT0 is strapped low
//! (rboot, 10K), NRST is unconnected, and SWD on J2 is the last-resort
//! recovery path. Everything else is software-triggered through the vendor
//! HID interface (main.rs):
//!
//! * opcode 0x12 ENTER_BOOT → the resident bootloader's driverless HID
//!   update mode. This is how firmware updates work since 0.10.0.
//! * opcode 0x02 ENTER_DFU → the ST ROM DFU (AN2606, 0483:DF11). Kept for
//!   the one-time migration from fw ≤ 0.9.0 and for reinstalling the
//!   bootloader itself with the combined image; needs a WinUSB driver on
//!   Windows and can brick the pad if interrupted, so the host only offers
//!   it behind a warning.
//!
//! Until 0.9.0 the application itself jumped into system memory from a
//! magic word in `.uninit` RAM, checked at the top of `main` before any
//! peripheral init. Now the *bootloader* performs that jump — it reads the
//! request from the fixed handoff words at reset, in exactly the clean
//! reset state the ROM expects — so this module only files the request
//! (boot.rs) and resets.

use crate::boot::{self, Request};

/// Arm the ROM-DFU request and reset; the bootloader finishes the job on
/// the way back up. The name predates the resident bootloader: "bootloader"
/// here means ST's ROM one, which is what opcode 0x02 has always meant.
pub fn reboot_into_bootloader() -> ! {
    boot::request(Request::RomDfu)
}

/// Plain application reset (through the bootloader, which re-validates the
/// image and starts it again): the way a device-mode change takes effect,
/// since the USB identity is only chosen at boot. The reset drops the USB
/// pull-up, so the host sees a clean unplug/replug.
pub fn reboot() -> ! {
    boot::clear_request();
    cortex_m::peripheral::SCB::sys_reset()
}
