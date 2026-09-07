//! [`Programmer`] over the on-chip flash for the application slot.
//!
//! The protocol layer speaks in offsets relative to `APP_BASE`; embassy's
//! flash driver wants offsets relative to `FLASH_BASE` (0x08000000) with
//! 4-byte writes and 2 KiB page erases. Everything is bounds-checked here
//! so that no request, however malformed, can touch the bootloader pages
//! below the slot or the file slots / config page from `DATA_BASE` up.

use core::sync::atomic::{compiler_fence, Ordering};

use embassy_stm32::flash::{Blocking, Flash};
use openmicro_layout::{APP_BASE, APP_SIZE, DATA_BASE, FLASH_BASE, PAGE_SIZE};

use crate::update::Programmer;

/// Slot start as an embassy flash offset.
const SLOT_OFFSET: u32 = APP_BASE - FLASH_BASE;

pub struct FlashProgrammer<'d> {
    flash: Flash<'d, Blocking>,
}

impl<'d> FlashProgrammer<'d> {
    pub fn new(flash: Flash<'d, Blocking>) -> Self {
        FlashProgrammer { flash }
    }
}

/// `[offset, offset + len)` (slot-relative) lies inside the slot and, as an
/// absolute range, ends before `DATA_BASE`. The second check is implied by
/// the first (`APP_END == DATA_BASE`) and kept explicit on purpose.
fn in_slot(offset: u32, len: u32) -> bool {
    match offset.checked_add(len) {
        Some(end) => end <= APP_SIZE && APP_BASE + end <= DATA_BASE,
        None => false,
    }
}

impl Programmer for FlashProgrammer<'_> {
    fn erase(&mut self, from: u32, to: u32) -> Result<(), ()> {
        if from % PAGE_SIZE != 0 || to % PAGE_SIZE != 0 || from >= to || !in_slot(from, to - from) {
            return Err(());
        }
        // Stalls the core ~20-40 ms per page; USB NAKs meanwhile, which the
        // host tolerates (BEGIN has an 8 s timeout).
        self.flash
            .blocking_erase(SLOT_OFFSET + from, SLOT_OFFSET + to)
            .map_err(|_| ())
    }

    fn write(&mut self, offset: u32, data: &[u8]) -> Result<(), ()> {
        let len = data.len() as u32;
        if len == 0 || offset % 4 != 0 || len % 4 != 0 || !in_slot(offset, len) {
            return Err(());
        }
        self.flash
            .blocking_write(SLOT_OFFSET + offset, data)
            .map_err(|_| ())
    }

    fn slot(&self) -> &[u8] {
        // The slice aliases memory the driver just changed through volatile
        // half-word stores; the fence keeps the compiler from reusing loads
        // it may have hoisted across those calls.
        compiler_fence(Ordering::SeqCst);
        // SAFETY: the slot is memory-mapped flash, readable at any time, and
        // nothing hands out a mutable reference to it.
        unsafe { core::slice::from_raw_parts(APP_BASE as *const u8, APP_SIZE as usize) }
    }
}
