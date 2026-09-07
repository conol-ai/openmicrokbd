//! The four RAM handoff words at the top of SRAM (0x20003FF0..0x20004000).
//!
//! They survive `SCB::sys_reset` (SRAM keeps its contents; only a power
//! cycle clears it) and sit outside both images' linker RAM regions, so
//! cortex-m-rt's .bss/.data init never touches them and every access here
//! is a volatile access to a fixed address. Values and addresses come from
//! layout/src/lib.rs; the application writes the same words.
//!
//! | word     | meaning                                                   |
//! |----------|-----------------------------------------------------------|
//! | REQUEST  | REQ_BOOTLOADER / REQ_ROM_DFU / REQ_RUN, else "none"        |
//! | !REQUEST | complement; power-up garbage never matches                |
//! | REASON   | why the app was started (normal / after update / switch)  |
//! | FAULTS   | `FAULTS_TAG | count`: panic/HardFault escalation counter   |

use core::ptr::{read_volatile, write_volatile};
use openmicro_layout::{HANDOFF_FAULTS, HANDOFF_REASON, HANDOFF_REQUEST, HANDOFF_REQUEST_INV};

#[inline(always)]
fn read(addr: u32) -> u32 {
    // SAFETY: a fixed, word-aligned SRAM address reserved by both memory.x
    // files; nothing else aliases it.
    unsafe { read_volatile(addr as *const u32) }
}

#[inline(always)]
fn write(addr: u32, value: u32) {
    // SAFETY: as above.
    unsafe { write_volatile(addr as *mut u32, value) }
}

/// The pending request if REQUEST and !REQUEST agree, clearing both so a
/// request is acted on exactly once (no re-entry loop after the reset that
/// follows). Returns `None` for the all-zero or power-up-garbage case.
pub fn take_request() -> Option<u32> {
    let req = read(HANDOFF_REQUEST);
    let inv = read(HANDOFF_REQUEST_INV);
    write(HANDOFF_REQUEST, 0);
    write(HANDOFF_REQUEST_INV, 0);
    (inv == !req).then_some(req)
}

/// Arms a request for the next boot; the caller resets right after.
pub fn write_request(req: u32) {
    write(HANDOFF_REQUEST, req);
    write(HANDOFF_REQUEST_INV, !req);
}

pub fn reason() -> u32 {
    read(HANDOFF_REASON)
}

pub fn set_reason(reason: u32) {
    write(HANDOFF_REASON, reason);
}

/// Raw faults word; decode with `openmicro_layout::faults_count`.
pub fn faults() -> u32 {
    read(HANDOFF_FAULTS)
}

pub fn set_faults(word: u32) {
    write(HANDOFF_FAULTS, word);
}
