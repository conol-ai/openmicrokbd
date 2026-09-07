//! The application's side of the resident-bootloader contract.
//!
//! Since fw 0.10.0 the pad boots through a bootloader that owns the first
//! 24 KiB of flash (`../../boot`); this image is linked at `APP_BASE`
//! (0x08006000, see memory.x) and is only ever started by it. Everything the
//! two images and the host must agree on — addresses, header formats, the
//! handoff words, protocol constants — lives in `openmicro-layout`
//! (`../../layout`); this module is the thin runtime glue around it:
//!
//! * [`APP_HEADER`] — the 32-byte header at `APP_BASE + 0xC0`, right behind
//!   the vector table (`.app_header`, placed by memory.x). At compile time it
//!   holds the magic, the header version, the board-variant flag (`proto`
//!   or not — the bootloader refuses to install an image built for the
//!   other pin map) and this crate's version; the release build patches
//!   image length and CRC-32 into the .bin afterwards (`scripts/fw-image.py
//!   patch`) and that is what the bootloader verifies before every jump. An
//!   ELF flashed by a debugger keeps length and CRC at zero ("unstamped");
//!   the bootloader then checks only the vector table and boots it anyway,
//!   so `cargo run` keeps working during development. (Its HID update
//!   protocol, by contrast, refuses unstamped uploads.)
//!
//! * [`remap_vectors_to_sram`] — the Cortex-M0 has no VTOR, so exceptions
//!   are always fetched from address 0, which is the bootloader's table.
//!   The fix is SYSCFG's memory remap: a copy of this image's table at
//!   0x20000000 (RAM the linker never touches — memory.x starts RAM at
//!   0x200000C0) with `MEM_MODE = SRAM`. The bootloader does this before it
//!   jumps, but `embassy_stm32::init` pulses SYSCFG through its RCC reset,
//!   which puts MEM_MODE back to main flash; `main` therefore re-applies the
//!   remap with interrupts still masked, before any of them can fire.
//!
//! * the handoff words at the top of RAM (0x20003FF0..0x20004000, outside
//!   both images' RAM regions). SRAM survives `SCB::sys_reset` — only a
//!   power cycle clears it — so they carry a request across a reset: the
//!   application writes a request word plus its complement (power-up garbage
//!   never matches) and resets; the bootloader clears the pair, then either
//!   stays in its HID update mode, jumps to the ST ROM DFU, or writes the
//!   boot reason and starts the application. Replaces the linker-placed
//!   `.uninit.DFU_MAGIC` word that fw ≤ 0.9.0 used. The fourth word is the
//!   bootloader's fault counter: a fault in this image's init window (before
//!   [`remap_vectors_to_sram`] has run, exceptions still go through the
//!   bootloader's table) counts there, and the bootloader stays in update
//!   mode on the next boot rather than start us again. It is *this* image
//!   that clears the counter, with [`clear_faults`], once it is running.
//!
//! Flash is tight with logging compiled in (the slot is 84 KiB; see the
//! README's build stats), so this module avoids anything that drags in
//! `core::fmt` or UTF-8 validation and reads the two flash blocks with
//! plain volatile word loads.

use core::ptr;
use embassy_stm32::pac;
use embassy_stm32::pac::syscfg::vals::MemMode;
use openmicro_layout::{
    app_header_bytes, app_valid, flags_for_variant, status, version_bytes, version_triple_bytes,
    BootInfo, BootInfoReply, APP_BASE, BOOT_BASE, HANDOFF_FAULTS, HANDOFF_REASON, HANDOFF_REQUEST,
    HANDOFF_REQUEST_INV, HEADER_LEN, HEADER_OFFSET, RAM_BASE, REASON_AFTER_UPDATE, REASON_NORMAL,
    REASON_SWITCH, REQ_BOOTLOADER, REQ_ROM_DFU, VARIANT_PROD, VARIANT_PROTO, VECTORS_LEN,
};

/// The board this image was built for; the bootloader records the same in
/// its info block and refuses to install an image whose header flag
/// disagrees (a `proto` build drives different encoder/LED pins).
const VARIANT: u16 = if cfg!(feature = "proto") {
    VARIANT_PROTO
} else {
    VARIANT_PROD
};

/// The application image header, linked at `APP_BASE + HEADER_OFFSET`.
///
/// `#[used]` + `#[no_mangle]` keep it through LTO (no code reads the static:
/// the bootloader and the host read the bytes in flash, and the release
/// pipeline rewrites the length/CRC fields after linking, which is why every
/// runtime read below goes through the flash address, never this symbol).
#[link_section = ".app_header"]
#[used]
#[no_mangle]
pub static APP_HEADER: [u8; HEADER_LEN as usize] =
    app_header_bytes(env!("CARGO_PKG_VERSION"), flags_for_variant(VARIANT));

/// This image's version as the BOOT_INFO reply carries it (folded at
/// compile time; the runtime loop would cost flash for nothing).
const APP_VERSION: [u8; 16] = version_bytes(crate::FW_VERSION);

/// Which image the next boot should end up in (see the handoff words).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Request {
    /// Stay in the resident bootloader's driverless HID update mode.
    Bootloader,
    /// Jump to the ST ROM DFU (0483:DF11). Only for reinstalling the
    /// bootloader itself with the combined image; the bootloader performs
    /// the jump from reset state, which is what the ROM expects.
    RomDfu,
}

/// Copies this image's vector table to the bottom of SRAM and maps SRAM to
/// address 0. Idempotent; must run with interrupts masked (a pending
/// interrupt taken half-way through the copy would vector into garbage).
pub fn remap_vectors_to_sram() {
    // SYSCFG lives on APB2 and is clock-gated; the dummy read-back makes
    // sure the enable has taken effect before the first register access.
    pac::RCC.apb2enr().modify(|w| w.set_syscfgen(true));
    let _ = pac::RCC.apb2enr().read();

    // The table is read from its flash home, not from address 0: what is
    // mapped there right now depends on who called us (the bootloader's
    // table after init's SYSCFG reset, our own copy on a repeat call).
    let src = APP_BASE as *const u32;
    let dst = RAM_BASE as *mut u32;
    for i in 0..(VECTORS_LEN / 4) as usize {
        // SAFETY: both ranges are fixed by the memory map — the table is the
        // first 0xC0 bytes of this image and 0x20000000..0x200000C0 is
        // reserved for its copy (memory.x keeps every section and the stack
        // above it). Volatile so the copy is never elided or reordered.
        unsafe { ptr::write_volatile(dst.add(i), ptr::read_volatile(src.add(i))) };
    }

    pac::SYSCFG.cfgr1().modify(|w| w.set_mem_mode(MemMode::SRAM));
    // Make the remap visible to the next fetch (the same barrier pair the
    // ROM-DFU jump in fw 0.9.0 used for the SYSTEM_FLASH remap).
    cortex_m::asm::dsb();
    cortex_m::asm::isb();
}

/// Writes the request into the handoff words and resets. The bootloader
/// picks it up on the way back up. Never returns; the caller should have
/// let its USB ack reach the host first.
pub fn request(r: Request) -> ! {
    let word = match r {
        Request::Bootloader => REQ_BOOTLOADER,
        Request::RomDfu => REQ_ROM_DFU,
    };
    write_handoff(word, !word);
    cortex_m::peripheral::SCB::sys_reset()
}

/// Clears any pending request before a plain reset, so a reboot for a
/// device-mode change can never replay an older request. A zero pair does
/// not match (0 != !0) and 0 is not a request value anyway.
pub fn clear_request() {
    write_handoff(0, 0);
}

fn write_handoff(word: u32, inv: u32) {
    // SAFETY: fixed addresses at the top of SRAM, excluded from this image's
    // RAM region by memory.x, so nothing else aliases them.
    unsafe {
        ptr::write_volatile(HANDOFF_REQUEST as *mut u32, word);
        ptr::write_volatile(HANDOFF_REQUEST_INV as *mut u32, inv);
    }
    cortex_m::asm::dsb();
}

/// Clears the bootloader's fault counter: we are running, so whatever
/// faulted on an earlier boot did not happen again. Call once the vector
/// table is ours again ([`remap_vectors_to_sram`]) — up to that point a
/// fault still goes through the bootloader's table and must keep counting,
/// or a firmware that dies in its init window would be restarted forever
/// instead of leaving the pad in update mode for a fix.
pub fn clear_faults() {
    // SAFETY: fixed address at the top of SRAM, outside this image's RAM.
    unsafe { ptr::write_volatile(HANDOFF_FAULTS as *mut u32, 0) };
}

/// Why the bootloader started us (`REASON_*` in the layout crate; anything
/// else means "unknown", e.g. an image started straight by a debugger).
pub fn reason() -> u32 {
    // SAFETY: fixed address at the top of SRAM, outside this image's RAM.
    unsafe { ptr::read_volatile(HANDOFF_REASON as *const u32) }
}

/// Human-readable form of [`reason`] for the boot log.
pub fn reason_str(r: u32) -> &'static str {
    match r {
        REASON_NORMAL => "normal",
        REASON_AFTER_UPDATE => "first boot after update",
        REASON_SWITCH => "encoder switch was held at power-up (update mode visited)",
        _ => "unknown (no bootloader handoff)",
    }
}

/// The bootloader's info block at `BOOT_BASE + HEADER_OFFSET`, or `None` if
/// the flash below us holds no bootloader (should not happen — nothing
/// else can have started us — except under a debugger on a bare chip).
pub fn boot_info() -> Option<BootInfo> {
    BootInfo::parse(&read_block(BOOT_BASE + HEADER_OFFSET))
}

/// Whether this image's header in flash carries a length/CRC stamp — which
/// the compile-time [`APP_HEADER`] cannot tell us. The magic needs no
/// re-check: we are running, so the bootloader already validated it.
pub fn is_stamped() -> bool {
    // SAFETY: the length and crc words of our own header, inside flash.
    let len = unsafe { ptr::read_volatile((APP_BASE + HEADER_OFFSET + 4) as *const u32) };
    let crc = unsafe { ptr::read_volatile((APP_BASE + HEADER_OFFSET + 8) as *const u32) };
    len != 0 || crc != 0
}

/// Builds the BOOT_INFO reply (31 bytes, `openmicro_layout::BootInfoReply`)
/// into a raw-HID report. Status 0 with the bootloader's protocol, version
/// and layout when the info block is present; status 1 with everything
/// zeroed when it is missing. `app_valid` is 1 (stamped) or 2 (unstamped —
/// a debugger-flashed ELF) for the running image: whichever it is, the
/// bootloader validated it before starting us. Returns the bytes written.
///
/// Kept out of line: inlined into the updater's state machine (the largest
/// function in the image) it measured ~100 bytes bigger.
#[inline(never)]
pub fn boot_info_reply(buf: &mut [u8; 32]) -> usize {
    let reply = match boot_info() {
        Some(info) => BootInfoReply {
            status: status::OK,
            protocol: info.protocol as u8,
            boot_version: version_triple_bytes(&info.version),
            app_base: info.app_base,
            app_size: info.app_size,
            app_valid: if is_stamped() {
                app_valid::VALID
            } else {
                app_valid::UNSTAMPED
            },
            app_version: APP_VERSION,
            page: None,
            max_chunk: None,
        },
        None => BootInfoReply {
            status: 1,
            ..Default::default()
        },
    };
    reply.encode(buf)
}

/// One 32-byte block from flash, read word by word and volatile: the bytes
/// were written by the bootloader build or patched into the .bin after
/// linking, so the compiler must not assume anything about them.
fn read_block(addr: u32) -> [u8; HEADER_LEN as usize] {
    let mut out = [0u8; HEADER_LEN as usize];
    let src = addr as *const u32;
    for (i, chunk) in out.chunks_exact_mut(4).enumerate() {
        // SAFETY: `addr` is one of the two header slots in the memory map,
        // both inside flash and 4-byte aligned.
        let w = unsafe { ptr::read_volatile(src.add(i)) };
        chunk.copy_from_slice(&w.to_le_bytes());
    }
    out
}
