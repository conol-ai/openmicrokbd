//! The pad's "file system" for the Work Louder protocol: two fixed slots in
//! the top of flash, one per file the Input app reads and writes —
//! `keymap.json` and `smart_actions.json` — plus the built-in default keymap
//! that stands in until a keymap has been written.
//!
//! Each slot starts with a 32-byte header (magic, size, SHA-1) followed by
//! the file body; readers get the body straight out of memory-mapped flash.
//! A write erases the slot, programs the body as chunks arrive (through a
//! small buffer, in multiples of the driver's `WRITE_SIZE`, padded with
//! 0xFF at the end) and finishes by programming the header, so a torn write reads as "no file" and the
//! default takes over again.
//!
//! Flash map (128 KiB part): image up to `0x1B000` (memory.x), keymap slot
//! `0x1B000..0x1E000` (12 KiB), smart-actions slot `0x1E000..0x1F800`
//! (6 KiB), the keymap/config page at `0x1F800` as before.

use core::cell::RefCell;
use embassy_stm32::flash::{Blocking, Flash, WRITE_SIZE};

use super::layout::DEFAULT_KEYMAP;
use super::sha1::{self, Sha1};
use super::wire::FileStore;

const FLASH_BASE: u32 = 0x0800_0000;
const HEADER_LEN: usize = 32;
const MAGIC: u32 = 0x4F4D_4631; // "OMF1"

struct Slot {
    name: &'static [u8],
    offset: u32,
    len: u32,
    /// Served when the slot is empty.
    default: Option<&'static [u8]>,
}

const SLOTS: [Slot; 2] = [
    Slot {
        name: b"keymap.json",
        offset: 0x1B000,
        len: 0x3000,
        default: Some(DEFAULT_KEYMAP.as_bytes()),
    },
    Slot {
        name: b"smart_actions.json",
        offset: 0x1E000,
        len: 0x1800,
        default: None,
    },
];

fn slot_index(name: &[u8]) -> Option<usize> {
    let name = name.strip_prefix(b"/").unwrap_or(name);
    SLOTS.iter().position(|s| s.name == name)
}

fn header(slot: &Slot) -> (u32, u32, [u8; 20]) {
    // Safety: valid, always-mapped flash addresses.
    let base = (FLASH_BASE + slot.offset) as *const u8;
    let mut h = [0u8; HEADER_LEN];
    for (i, b) in h.iter_mut().enumerate() {
        *b = unsafe { base.add(i).read_volatile() };
    }
    let magic = u32::from_le_bytes([h[0], h[1], h[2], h[3]]);
    let size = u32::from_le_bytes([h[4], h[5], h[6], h[7]]);
    let mut sha = [0u8; 20];
    sha.copy_from_slice(&h[8..28]);
    (magic, size, sha)
}

/// The slot's body if it holds a valid file.
fn stored(slot: &Slot) -> Option<(&'static [u8], [u8; 20])> {
    let (magic, size, sha) = header(slot);
    if magic != MAGIC || size as usize > slot.len as usize - HEADER_LEN {
        return None;
    }
    let body = (FLASH_BASE + slot.offset) as usize + HEADER_LEN;
    // Safety: flash is memory-mapped and never unmapped; the slice stays
    // valid until the slot is erased, which only happens on a new write —
    // callers hold these slices only within one request.
    Some((
        unsafe { core::slice::from_raw_parts(body as *const u8, size as usize) },
        sha,
    ))
}

/// The named file as it is right now — the stored copy, else the built-in
/// default. Needs no store instance: the slots are memory-mapped flash.
pub fn read(name: &[u8]) -> Option<&'static [u8]> {
    let slot = &SLOTS[slot_index(name)?];
    match stored(slot) {
        Some((body, _)) => Some(body),
        None => slot.default,
    }
}

struct Writing {
    slot: usize,
    /// Bytes accepted so far (buffered or programmed).
    pos: usize,
    sha: Sha1,
    /// Programming buffer: flushed in aligned 64-byte pieces.
    buf: [u8; 64],
    fill: usize,
    ok: bool,
}

pub struct FlashStore {
    flash: &'static RefCell<Flash<'static, Blocking>>,
    writing: Option<Writing>,
    /// SHA-1 of the built-in keymap, computed once.
    default_sha: Option<[u8; 20]>,
}

impl FlashStore {
    pub const fn new(flash: &'static RefCell<Flash<'static, Blocking>>) -> Self {
        Self {
            flash,
            writing: None,
            default_sha: None,
        }
    }

    fn default_digest(&mut self) -> [u8; 20] {
        match self.default_sha {
            Some(d) => d,
            None => {
                let d = sha1::digest(DEFAULT_KEYMAP.as_bytes());
                self.default_sha = Some(d);
                d
            }
        }
    }

    fn program(&mut self, offset: u32, data: &[u8]) -> bool {
        debug_assert!(data.len() % WRITE_SIZE == 0 && offset % WRITE_SIZE as u32 == 0);
        self.flash.borrow_mut().blocking_write(offset, data).is_ok()
    }

    fn erase_slot(&mut self, slot: &Slot) -> bool {
        self.flash
            .borrow_mut()
            .blocking_erase(slot.offset, slot.offset + slot.len)
            .is_ok()
    }

    fn flush(&mut self, w: &mut Writing, all: bool) -> bool {
        let take = if all { w.fill } else { w.fill & !63 };
        if take == 0 {
            return true;
        }
        let slot = &SLOTS[w.slot];
        // pos counts accepted bytes; the programmed offset trails by what
        // is still buffered.
        let programmed = w.pos - w.fill;
        let at = slot.offset + HEADER_LEN as u32 + programmed as u32;
        let mut chunk = [0xFFu8; 64];
        let len = take.min(64);
        chunk[..len].copy_from_slice(&w.buf[..len]);
        // The last piece is padded with 0xFF (erased) up to the program unit;
        // the size in the header says where the file really ends.
        let padded = len.div_ceil(WRITE_SIZE) * WRITE_SIZE;
        if !self.program(at, &chunk[..padded]) {
            return false;
        }
        w.buf.copy_within(len..w.fill, 0);
        w.fill -= len;
        true
    }
}

impl FileStore for FlashStore {
    fn each_file(&mut self, f: &mut dyn FnMut(&[u8], usize, &[u8; 20])) {
        for (i, slot) in SLOTS.iter().enumerate() {
            if let Some((body, sha)) = stored(slot) {
                f(slot.name, body.len(), &sha);
            } else if let Some(d) = slot.default {
                // Only the keymap has a default; index kept for clarity.
                let _ = i;
                let sha = self.default_digest();
                f(slot.name, d.len(), &sha);
            }
        }
    }

    fn read(&self, name: &[u8]) -> Option<&'static [u8]> {
        read(name)
    }

    fn begin_write(&mut self, name: &[u8]) -> bool {
        let Some(idx) = slot_index(name) else {
            return false;
        };
        self.writing = None;
        if !self.erase_slot(&SLOTS[idx]) {
            return false;
        }
        self.writing = Some(Writing {
            slot: idx,
            pos: 0,
            sha: Sha1::new(),
            buf: [0; 64],
            fill: 0,
            ok: true,
        });
        true
    }

    fn write(&mut self, data: &[u8]) -> bool {
        let Some(mut w) = self.writing.take() else {
            return false;
        };
        let cap = SLOTS[w.slot].len as usize - HEADER_LEN;
        let mut ok = w.ok;
        let mut rest = data;
        while ok && !rest.is_empty() {
            if w.pos + 1 > cap {
                ok = false;
                break;
            }
            let room = (64 - w.fill).min(rest.len()).min(cap - w.pos);
            w.buf[w.fill..w.fill + room].copy_from_slice(&rest[..room]);
            w.sha.update(&rest[..room]);
            w.fill += room;
            w.pos += room;
            rest = &rest[room..];
            if w.fill == 64 && !self.flush(&mut w, false) {
                ok = false;
            }
        }
        w.ok = ok;
        self.writing = Some(w);
        ok
    }

    fn finish_write(&mut self) -> bool {
        let Some(mut w) = self.writing.take() else {
            return false;
        };
        if !w.ok || !self.flush(&mut w, true) {
            return false;
        }
        let slot = &SLOTS[w.slot];
        let sha = core::mem::replace(&mut w.sha, Sha1::new()).finish();
        let mut h = [0xFFu8; HEADER_LEN];
        h[0..4].copy_from_slice(&MAGIC.to_le_bytes());
        h[4..8].copy_from_slice(&(w.pos as u32).to_le_bytes());
        h[8..28].copy_from_slice(&sha);
        self.program(slot.offset, &h)
    }

    fn abort_write(&mut self) {
        self.writing = None;
    }

    fn delete(&mut self, name: &[u8]) -> bool {
        let Some(idx) = slot_index(name) else {
            return false;
        };
        self.writing = None;
        self.erase_slot(&SLOTS[idx])
    }
}
