//! App-triggered entry into the STM32F072 ROM DFU bootloader.
//!
//! The board deliberately has no BOOT0 button — BOOT0 is strapped low
//! (rboot, 10K) and SWD on J2 is the debug/recovery path. Field updates are
//! instead software-triggered: the host app writes ENTER_DFU to the vendor
//! HID interface, we stamp a magic word in noinit RAM and reset, and the
//! next boot diverts into system memory before touching any peripheral.
//! The ROM bootloader (AN2606) then enumerates as USB DFU 0483:df11 on the
//! same USB-C port for a standard DfuSe download.

use core::mem::MaybeUninit;
use core::ptr::addr_of_mut;

/// F07x system-memory bootloader base (AN2606).
const SYSTEM_MEMORY: u32 = 0x1FFF_C800;
const MAGIC: u32 = 0xB007_10AD;

/// Survives a system reset (SRAM keeps its contents; only power-on clears
/// it) and is skipped by cortex-m-rt's .bss/.data init.
#[link_section = ".uninit.DFU_MAGIC"]
static mut DFU_MAGIC: MaybeUninit<u32> = MaybeUninit::uninit();

/// Must run before `embassy_stm32::init` — the ROM bootloader expects the
/// reset-state clock/peripheral configuration it was designed to start from.
pub fn check_and_enter() {
    unsafe {
        let slot = addr_of_mut!(DFU_MAGIC).cast::<u32>();
        if slot.read_volatile() == MAGIC {
            slot.write_volatile(0);
            let sp = (SYSTEM_MEMORY as *const u32).read_volatile();
            let rv = ((SYSTEM_MEMORY + 4) as *const u32).read_volatile();
            cortex_m::asm::bootstrap(sp as *const u32, rv as *const u32);
        }
    }
}

/// Arm the magic and reset; `check_and_enter` finishes the job on the way
/// back up.
pub fn reboot_into_bootloader() -> ! {
    unsafe {
        addr_of_mut!(DFU_MAGIC).cast::<u32>().write_volatile(MAGIC);
    }
    cortex_m::peripheral::SCB::sys_reset();
}
