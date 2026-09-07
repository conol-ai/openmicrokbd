//! The bootloader's HID update protocol as a pure state machine over a
//! [`Programmer`] (the application slot in flash).
//!
//! No embassy, no hardware, no `unsafe`: `boot/host-tests` compiles this
//! file for the host with `#[path]` and drives it against a RAM model of
//! the slot, which is where the protocol is actually tested. `main.rs`
//! only moves reports in and out, runs the LED cue, and performs the
//! delays, handoff writes and resets an [`Action`] asks for.
//!
//! Requests are 64-byte HID reports whose byte 0 is the opcode; replies
//! echo it (bit 7 never set) and carry a status in byte 1 where the table
//! in boot/README.md says so. Multi-byte fields are little-endian. Opcodes
//! and status codes live in `openmicro_layout::{op, status}`.
//!
//! Upload state machine (one image at a time):
//!
//! ```text
//! UPDATE_BEGIN(length, crc)  crc != 0; erase ceil(length / page) pages, expect offset 0
//! UPDATE_DATA(offset, data)* program 4-byte words in order (offset == expected);
//!                            the header's length/crc words must equal BEGIN's
//! UPDATE_END                 flush, validate header + CRC + variant from flash
//! ```
//!
//! Only *stamped* images get in this way: the boot path accepts an
//! unstamped header (length == crc == 0, what a debugger flashes from the
//! ELF) on its vector table alone, so the protocol must never let a
//! partial upload leave one behind — BEGIN refuses crc 0 and DATA refuses
//! the header chunk unless its length and crc words are the ones BEGIN
//! announced, before they reach flash. Any status other than OK leaves the
//! slot as it is; the header/CRC check before every boot is what turns a
//! half-written slot into "stay in update mode" instead of a hang. A failed
//! flash operation, a refused header chunk or a completed-but-invalid
//! upload drops the BEGIN state (the host must start over); an out-of-order
//! chunk keeps it and reports the offset to resume from.

use openmicro_layout::{
    app_valid, op, status, u32_at, validate_app_for, version_triple, AppError,
    BootInfoReply, Validity, APP_BASE, APP_SIZE, BOOT_PROTOCOL, BOOT_REPORT_LEN, ENTER_DFU_KEY,
    HEADER_OFFSET, MAX_CHUNK, PAGE_SIZE, RESET_OFFSET,
};

/// Slot offsets of the header's length and crc words, the two DATA compares
/// against BEGIN.
const LENGTH_OFFSET: u32 = HEADER_OFFSET + 4;
const CRC_OFFSET: u32 = HEADER_OFFSET + 8;

/// Whatever programs the application slot. Offsets are relative to
/// `APP_BASE`; the implementation must refuse anything outside the slot.
pub trait Programmer {
    /// Erase `[from, to)`; both page-aligned, `to` exclusive.
    fn erase(&mut self, from: u32, to: u32) -> Result<(), ()>;
    /// Program `data` (a non-empty multiple of 4 bytes) at a 4-byte aligned
    /// offset that has been erased since the last write there.
    fn write(&mut self, offset: u32, data: &[u8]) -> Result<(), ()>;
    /// The whole slot as it currently reads (`APP_SIZE` bytes).
    fn slot(&self) -> &[u8];
}

/// What `main.rs` must do with the reply it was handed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Send the reply; the first `n` bytes carry the answer, the rest is zero.
    Reply(usize),
    /// Send the reply, let it leave the bus, arm REQ_ROM_DFU and reset.
    ResetToDfu,
    /// Send the reply, wait, arm REQ_RUN and reset.
    ResetToApp,
}

/// What a reply means for the LED cue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Nothing to show (queries, successful steps).
    Quiet,
    /// Any error status: the red blinks.
    Error,
    /// A valid UPDATE_END: the green flash.
    UpdateDone,
}

#[derive(Clone, Copy)]
struct Begin {
    length: u32,
    crc: u32,
}

pub struct Updater<P: Programmer> {
    prog: P,
    /// The bootloader's CARGO_PKG_VERSION, for VERSION and BOOT_INFO.
    version: &'static str,
    boot_version: [u8; 3],
    /// The bootloader's own board variant (`VARIANT_PROD` / `VARIANT_PROTO`):
    /// an image whose header flags say otherwise is refused at END.
    variant: u16,
    /// The upload in progress, if UPDATE_BEGIN succeeded and nothing since
    /// ended or aborted it.
    begin: Option<Begin>,
    /// Bytes programmed so far (a multiple of 4).
    written: u32,
    /// Tail bytes not yet forming a whole word. Only a final chunk can leave
    /// them (earlier chunks must be word multiples); UPDATE_END flushes
    /// them padded with 0xFF.
    pending: [u8; 4],
    pending_len: usize,
    /// Set once a BOOT_INFO has been answered: proof that update mode
    /// enumerated and is talking, so `main` can clear the fault counter.
    info_answered: bool,
}

impl<P: Programmer> Updater<P> {
    pub fn new(prog: P, version: &'static str, variant: u16) -> Self {
        Updater {
            prog,
            version,
            boot_version: version_triple(version),
            variant,
            begin: None,
            written: 0,
            pending: [0; 4],
            pending_len: 0,
            info_answered: false,
        }
    }

    /// Handles one request report and fills `rep` (zeroed first, opcode
    /// echoed in byte 0). Never panics on short or malformed requests.
    pub fn handle(&mut self, req: &[u8], rep: &mut [u8; BOOT_REPORT_LEN]) -> Action {
        *rep = [0; BOOT_REPORT_LEN];
        let opcode = req.first().copied().unwrap_or(0);
        rep[0] = opcode;
        match opcode {
            op::VERSION => Action::Reply(self.version_reply(rep)),
            op::ENTER_DFU => {
                if req.len() >= 5 && req[1..5] == ENTER_DFU_KEY {
                    rep[1] = 1;
                    Action::ResetToDfu
                } else {
                    // Refused rather than ignored: a reply beats a host timeout.
                    rep[1] = 0;
                    Action::Reply(2)
                }
            }
            op::BOOT_INFO => Action::Reply(self.boot_info(rep)),
            op::UPDATE_BEGIN => {
                rep[1] = self.begin(req);
                Action::Reply(2)
            }
            op::UPDATE_DATA => {
                let (st, next) = self.data(req);
                rep[1] = st;
                rep[2..6].copy_from_slice(&next.to_le_bytes());
                Action::Reply(6)
            }
            op::UPDATE_END => {
                rep[1] = self.end();
                Action::Reply(2)
            }
            op::BOOT_RUN => {
                rep[1] = status::OK;
                Action::ResetToApp
            }
            _ => {
                rep[1] = status::UNKNOWN;
                Action::Reply(2)
            }
        }
    }

    /// True exactly once, after the first BOOT_INFO reply.
    pub fn take_info_answered(&mut self) -> bool {
        core::mem::replace(&mut self.info_answered, false)
    }

    /// For the host tests, which inspect the RAM slot after each step.
    #[allow(dead_code)]
    pub fn programmer(&self) -> &P {
        &self.prog
    }

    /// `[0x01, len, "boot X.Y.Z"]`.
    fn version_reply(&self, rep: &mut [u8; BOOT_REPORT_LEN]) -> usize {
        const PREFIX: &[u8] = b"boot ";
        let v = self.version.as_bytes();
        let n = v.len().min(BOOT_REPORT_LEN - 2 - PREFIX.len());
        rep[1] = (PREFIX.len() + n) as u8;
        rep[2..2 + PREFIX.len()].copy_from_slice(PREFIX);
        rep[2 + PREFIX.len()..2 + PREFIX.len() + n].copy_from_slice(&v[..n]);
        2 + PREFIX.len() + n
    }

    /// The 34-byte extended BOOT_INFO reply; `app_valid`/`app_version` come
    /// from validating the slot right now (an erased or half-written slot
    /// reports NONE).
    fn boot_info(&mut self, rep: &mut [u8; BOOT_REPORT_LEN]) -> usize {
        let (valid, version) = match validate_app_for(self.prog.slot(), APP_BASE, APP_SIZE, self.variant) {
            Ok(v) => (v.app_valid_code(), v.header().version),
            Err(_) => (app_valid::NONE, [0; 16]),
        };
        let reply = BootInfoReply {
            status: status::OK,
            protocol: BOOT_PROTOCOL as u8,
            boot_version: self.boot_version,
            app_base: APP_BASE,
            app_size: APP_SIZE,
            app_valid: valid,
            app_version: version,
            page: Some(PAGE_SIZE as u16),
            max_chunk: Some(MAX_CHUNK as u8),
        };
        self.info_answered = true;
        reply.encode(&mut rep[..])
    }

    /// `[0x21, length u32, crc u32]`: validates the length and crc, erases
    /// the pages the image will occupy and restarts the upload. Always drops
    /// any previous BEGIN state, so a retry after a partial upload restarts.
    fn begin(&mut self, req: &[u8]) -> u8 {
        self.abort();
        if req.len() < 9 {
            return status::BAD_LENGTH;
        }
        let length = u32_at(req, 1);
        let crc = u32_at(req, 5);
        if length < RESET_OFFSET || length > APP_SIZE || length % 4 != 0 {
            return status::BAD_LENGTH;
        }
        // crc 0 is what an unstamped header carries: not an upload we take,
        // and not worth erasing the slot for (the old image stays valid).
        if crc == 0 {
            return status::HEADER;
        }
        let pages = length.div_ceil(PAGE_SIZE);
        if self.prog.erase(0, pages * PAGE_SIZE).is_err() {
            return status::FLASH;
        }
        self.begin = Some(Begin { length, crc });
        status::OK
    }

    /// `[0x22, offset u32, n u8, data[n]]` → `(status, next_offset)`. The
    /// chunk must be the next one (`offset == expected`), at most MAX_CHUNK
    /// bytes, a multiple of 4 unless it completes the image, and inside
    /// `length`; the chunk carrying the header's length/crc words must
    /// carry the values BEGIN announced. Programs whole words immediately.
    fn data(&mut self, req: &[u8]) -> (u8, u32) {
        let Some(begin) = self.begin else {
            return (status::NO_BEGIN, 0);
        };
        let expected = self.expected();
        if req.len() < 6 {
            return (status::BAD_LENGTH, expected);
        }
        let offset = u32_at(req, 1);
        let n = req[5] as usize;
        if n == 0 || n > MAX_CHUNK || req.len() < 6 + n {
            return (status::BAD_LENGTH, expected);
        }
        let end = match offset.checked_add(n as u32) {
            Some(end) if end <= begin.length => end,
            _ => return (status::BAD_LENGTH, expected),
        };
        if n % 4 != 0 && end != begin.length {
            return (status::BAD_LENGTH, expected);
        }
        if offset != expected {
            // A lost reply makes the host resend: tell it where we are.
            return (status::OUT_OF_ORDER, expected);
        }

        // The header's length and crc words must be the ones BEGIN announced,
        // checked before they reach flash: an unstamped image (both zero) or
        // a mix-up of image and crc is refused here, with the slot still
        // lacking its magic — never after the whole upload, and never as a
        // bootable-looking header over a partial body. Chunks before the
        // last are word multiples at word offsets, so a word is either fully
        // inside a chunk or not in it at all.
        let data = &req[6..6 + n];
        for (word, want) in [(LENGTH_OFFSET, begin.length), (CRC_OFFSET, begin.crc)] {
            if offset <= word && word + 4 <= end && u32_at(data, (word - offset) as usize) != want {
                self.abort();
                return (status::HEADER, 0);
            }
        }

        // Whole words go to flash now; a tail shorter than a word waits for
        // the next chunk (impossible after a word-multiple chunk) or END.
        let mut buf = [0u8; 4 + MAX_CHUNK];
        buf[..self.pending_len].copy_from_slice(&self.pending[..self.pending_len]);
        buf[self.pending_len..self.pending_len + n].copy_from_slice(&req[6..6 + n]);
        let total = self.pending_len + n;
        let whole = total & !3;
        if whole > 0 {
            if self.prog.write(self.written, &buf[..whole]).is_err() {
                // The slot is now indeterminate: only a new BEGIN (erase)
                // can make it consistent again.
                self.abort();
                return (status::FLASH, 0);
            }
            self.written += whole as u32;
        }
        let rest = total - whole;
        self.pending[..rest].copy_from_slice(&buf[whole..total]);
        self.pending_len = rest;
        (status::OK, self.expected())
    }

    /// `[0x23]`: flushes a partial word padded with 0xFF, then validates the
    /// slot from flash exactly as the boot path will. OK only if the image
    /// passes `validate_app` as a *stamped* image, was built for this
    /// bootloader's board variant, *and* the CRC the host announced in
    /// BEGIN is the one in the header (a mix-up of images is a CRC error
    /// too).
    fn end(&mut self) -> u8 {
        let Some(begin) = self.begin else {
            return status::NO_BEGIN;
        };
        if self.expected() != begin.length {
            // Not all bytes arrived: keep the state so the host can resume
            // (it learns the offset from the next DATA reply).
            return status::OUT_OF_ORDER;
        }
        if self.pending_len > 0 {
            let mut word = [0xFFu8; 4];
            word[..self.pending_len].copy_from_slice(&self.pending[..self.pending_len]);
            self.pending_len = 0;
            if self.prog.write(self.written, &word).is_err() {
                self.abort();
                return status::FLASH;
            }
            self.written += 4;
        }
        self.abort();
        // `validate_app_for` applies the same rules the boot path uses,
        // including the board-variant flag: an image built for the other
        // pin map is refused here and would not be started either.
        match validate_app_for(self.prog.slot(), APP_BASE, APP_SIZE, self.variant) {
            // `begin`/`data` already keep unstamped images out; this covers
            // a slot that reads back zeros where the stamp should be.
            Ok(Validity::Unstamped(_)) => status::HEADER,
            Ok(v) if v.header().crc == begin.crc => status::OK,
            Ok(_) | Err(AppError::Crc) => status::CRC,
            Err(_) => status::HEADER,
        }
    }

    fn expected(&self) -> u32 {
        self.written + self.pending_len as u32
    }

    fn abort(&mut self) {
        self.begin = None;
        self.written = 0;
        self.pending_len = 0;
    }
}

/// Classifies a reply for the LED cue.
pub fn outcome(rep: &[u8]) -> Outcome {
    let (opcode, st) = match rep {
        [o, s, ..] => (*o, *s),
        _ => return Outcome::Error,
    };
    match opcode {
        op::UPDATE_END if st == status::OK => Outcome::UpdateDone,
        op::UPDATE_BEGIN | op::UPDATE_DATA | op::UPDATE_END => {
            if st == status::OK {
                Outcome::Quiet
            } else {
                Outcome::Error
            }
        }
        // Byte 1 is an ack flag here, not a status.
        op::ENTER_DFU => {
            if st == 1 {
                Outcome::Quiet
            } else {
                Outcome::Error
            }
        }
        op::VERSION | op::BOOT_INFO | op::BOOT_RUN => Outcome::Quiet,
        _ => Outcome::Error,
    }
}
