//! Client for the OpenMicro resident bootloader's HID update mode.
//!
//! The bootloader (boot/, flashed once through ROM DFU with a combined image)
//! enumerates as 1209:0002 "OpenMicro Bootloader" with one vendor HID
//! interface (usage page 0xFF60) of **64-byte** reports and no report IDs.
//! Its protocol is the one in ../layout/src/lib.rs (`op`, `status`): every
//! reply echoes the opcode in byte 0, multi-byte fields are little-endian,
//! and an upload is UPDATE_BEGIN (erase) → UPDATE_DATA × n (program) →
//! UPDATE_END (validate) → BOOT_RUN.
//!
//! The protocol logic lives in [`Uploader`], generic over a [`Transport`], so
//! the reply matching, chunking and resume rules are unit-tested against a
//! scripted fake bootloader; [`BootDevice`] is the hidapi transport around it.
//!
//! Why the reply matching is fussy: hidapi hands us reports in order, but a
//! reply that arrived after we gave up waiting for it (a flash stall, a slow
//! host) is still queued and would otherwise be taken as the answer to the
//! *next* request. UPDATE_DATA replies therefore carry `next_offset`, and the
//! uploader only accepts the one that confirms exactly the chunk it just
//! sent; anything else is dropped. A status 3 (out of order) reply tells us
//! where the bootloader really is — typically one chunk ahead, because our
//! resend of a chunk whose reply we missed — and the upload resumes there.
//! After a timeout the uploader first re-synchronises with BOOT_INFO so a
//! late DATA reply is flushed before the chunk is sent again.

use std::ffi::{CStr, CString};
use std::time::{Duration, Instant};

use hidapi::{HidApi, HidDevice};
use openmicro_layout::{
    app_valid, op, status, validate_app, AppError, BootInfoReply, Validity, BOOT_MANUFACTURER,
    BOOT_PID, BOOT_PRODUCT, BOOT_PROTOCOL, BOOT_REPORT_LEN, BOOT_VID, ENTER_DFU_KEY, MAX_CHUNK,
    RAW_USAGE_PAGE,
};

/// One request/reply round trip needs both halves because a reply can be
/// rejected (stale) and the read then continues within the same deadline.
pub trait Transport {
    /// Write one report (`report` is the payload without a report id).
    fn send(&mut self, report: &[u8]) -> Result<(), String>;
    /// Wait up to `timeout` for one report. `Ok(None)` means the timeout
    /// elapsed with nothing to read; an `Err` is a broken link (unplugged).
    fn recv(&mut self, timeout: Duration) -> Result<Option<Vec<u8>>, String>;
}

/// Per-operation reply deadlines. UPDATE_BEGIN erases up to 42 pages
/// (~25 ms each), so it gets the long budget; everything else is a single
/// report of work.
#[derive(Clone, Copy, Debug)]
pub struct Timeouts {
    pub info: Duration,
    pub begin: Duration,
    pub data: Duration,
    pub end: Duration,
    pub run: Duration,
}

impl Default for Timeouts {
    fn default() -> Self {
        Self {
            info: Duration::from_secs(1),
            begin: Duration::from_secs(8),
            data: Duration::from_secs(2),
            end: Duration::from_secs(2),
            run: Duration::from_secs(2),
        }
    }
}

/// How many DATA timeouts in a row (each followed by a BOOT_INFO re-sync)
/// before the link is declared dead. A single stall is normal; five means the
/// pad is gone or wedged.
const MAX_CONSECUTIVE_TIMEOUTS: u32 = 5;
/// Out-of-order replies that do not move `next_offset` forward are a loop,
/// not a resume; stop after this many.
const MAX_STALLED_RESUMES: u32 = 8;

/// What an upload did, for the log line and for tests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UploadReport {
    pub length: u32,
    pub chunks: u32,
    /// UPDATE_DATA replies with status 3 that moved the write pointer.
    pub resumed: u32,
    /// DATA timeouts that were recovered with a BOOT_INFO re-sync.
    pub resyncs: u32,
}

/// The bootloader protocol over any [`Transport`].
pub struct Uploader<T: Transport> {
    transport: T,
    pub timeouts: Timeouts,
}

impl<T: Transport> Uploader<T> {
    pub fn new(transport: T) -> Self {
        Self {
            transport,
            timeouts: Timeouts::default(),
        }
    }

    /// Send `req` and return the first reply within `timeout` that echoes
    /// the opcode and satisfies `accept`. `Ok(None)` is a timeout; replies
    /// for other opcodes (or rejected by `accept`) are stale and skipped.
    fn request(
        &mut self,
        req: &[u8],
        timeout: Duration,
        mut accept: impl FnMut(&[u8]) -> bool,
    ) -> Result<Option<Vec<u8>>, String> {
        debug_assert!(!req.is_empty() && req.len() <= BOOT_REPORT_LEN);
        self.transport.send(req)?;
        let deadline = Instant::now() + timeout;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Ok(None);
            }
            match self.transport.recv(left)? {
                None => return Ok(None),
                Some(reply) if !reply.is_empty() && reply[0] == req[0] && accept(&reply) => {
                    return Ok(Some(reply));
                }
                Some(_) => {} // stale or foreign report — keep reading
            }
        }
    }

    /// A request whose reply is `[op, status, ...]` and must arrive.
    fn simple(&mut self, req: &[u8], timeout: Duration, what: &str) -> Result<Vec<u8>, String> {
        match self.request(req, timeout, |r| r.len() >= 2)? {
            Some(reply) => Ok(reply),
            None => Err(format!("{what}: no reply from the bootloader")),
        }
    }

    /// BOOT_INFO. Only a status-0, protocol-1 answer counts as a bootloader
    /// this app knows how to drive.
    pub fn info(&mut self) -> Result<BootInfoReply, String> {
        let reply = self.simple(&[op::BOOT_INFO], self.timeouts.info, "BOOT_INFO")?;
        let info = BootInfoReply::parse(&reply)
            .ok_or_else(|| format!("BOOT_INFO: malformed reply ({} bytes)", reply.len()))?;
        if info.status != status::OK {
            return Err(format!(
                "BOOT_INFO: bootloader reports status {}",
                info.status
            ));
        }
        if u16::from(info.protocol) != BOOT_PROTOCOL {
            return Err(format!(
                "bootloader speaks protocol {} but this app needs {}",
                info.protocol, BOOT_PROTOCOL
            ));
        }
        Ok(info)
    }

    /// Program `image` (an application image as linked for the pad's app
    /// base: vector table, stamped header, code) into the application slot.
    /// The image is validated locally against the pad's own BOOT_INFO first,
    /// so a wrong-base or unstamped file never reaches the flash.
    /// `progress` gets 0..=100 (percent), monotonic.
    pub fn upload(
        &mut self,
        image: &[u8],
        progress: &mut dyn FnMut(u8),
    ) -> Result<UploadReport, String> {
        let info = self.info()?;
        let header = match validate_app(image, info.app_base, info.app_size) {
            Ok(Validity::Stamped(header)) => header,
            Ok(Validity::Unstamped(_)) => {
                return Err(
                    "image is not stamped (length and crc are zero): stamp it with scripts/fw-image.py patch, or flash it with a debugger"
                        .into(),
                )
            }
            Err(e) => return Err(format!("not a valid application image: {}", describe(e))),
        };
        let length = header.length;
        let image = &image[..length as usize];

        // Chunk size: the bootloader advertises its limit; older/foreign
        // values are clamped to the protocol maximum and to a whole number
        // of flash words (the last chunk may be shorter, per the protocol).
        let chunk = info
            .max_chunk
            .map(usize::from)
            .filter(|c| (4..=MAX_CHUNK).contains(c))
            .unwrap_or(MAX_CHUNK)
            & !3;
        let mut report = UploadReport {
            length,
            ..UploadReport::default()
        };
        let mut last_percent = None;
        let mut emit = |offset: u32, progress: &mut dyn FnMut(u8)| {
            let percent = (u64::from(offset) * 100 / u64::from(length.max(1))) as u8;
            if last_percent != Some(percent) {
                last_percent = Some(percent);
                progress(percent);
            }
        };

        // -- BEGIN: erase the slot; the pad replies once every page is gone.
        let mut begin = vec![op::UPDATE_BEGIN];
        begin.extend_from_slice(&length.to_le_bytes());
        begin.extend_from_slice(&header.crc.to_le_bytes());
        let reply = self.simple(&begin, self.timeouts.begin, "UPDATE_BEGIN")?;
        match reply[1] {
            status::OK => {}
            status::BAD_LENGTH => {
                return Err(format!(
                    "UPDATE_BEGIN: the bootloader rejected the image length ({length} bytes)"
                ))
            }
            status::FLASH => return Err("UPDATE_BEGIN: flash erase failed".into()),
            other => return Err(format!("UPDATE_BEGIN: unexpected status {other}")),
        }
        emit(0, progress);

        // -- DATA: one chunk per report, confirmed by its next_offset.
        let mut offset: u32 = 0;
        let mut timeouts_in_a_row = 0u32;
        let mut stalled_resumes = 0u32;
        while offset < length {
            let n = chunk.min((length - offset) as usize);
            let mut req = Vec::with_capacity(6 + n);
            req.push(op::UPDATE_DATA);
            req.extend_from_slice(&offset.to_le_bytes());
            req.push(n as u8);
            req.extend_from_slice(&image[offset as usize..offset as usize + n]);
            let expected_next = offset + n as u32;

            let reply = self.request(&req, self.timeouts.data, |r| {
                // Status 0 must confirm *this* chunk; everything else (an
                // out-of-order pointer, a flash error) is informative on
                // its own. Short replies are junk.
                r.len() >= 6 && (r[1] != status::OK || u32_at(r, 2) == expected_next)
            })?;
            let Some(reply) = reply else {
                // Timeout. The reply may still be in flight: drain it with a
                // cheap BOOT_INFO round trip (which also proves the pad is
                // alive), then send the same chunk again. If the pad did
                // program it, the resend comes back as status 3 with the
                // pointer one chunk ahead and we simply move on.
                timeouts_in_a_row += 1;
                if timeouts_in_a_row > MAX_CONSECUTIVE_TIMEOUTS {
                    return Err(format!(
                        "UPDATE_DATA at offset {offset}: the bootloader stopped answering"
                    ));
                }
                self.info().map_err(|e| {
                    format!("UPDATE_DATA at offset {offset}: no reply, and re-sync failed: {e}")
                })?;
                report.resyncs += 1;
                continue;
            };
            timeouts_in_a_row = 0;
            let next = u32_at(&reply, 2);
            match reply[1] {
                status::OK => {
                    offset = next;
                    stalled_resumes = 0;
                    report.chunks += 1;
                }
                status::OUT_OF_ORDER => {
                    if next > length || !next.is_multiple_of(4) {
                        return Err(format!(
                            "UPDATE_DATA: the bootloader expects offset {next}, outside the {length}-byte image"
                        ));
                    }
                    if next > offset {
                        report.resumed += 1;
                        stalled_resumes = 0;
                    } else {
                        stalled_resumes += 1;
                        if stalled_resumes > MAX_STALLED_RESUMES {
                            return Err(format!(
                                "UPDATE_DATA: the bootloader keeps asking for offset {next}"
                            ));
                        }
                    }
                    offset = next;
                }
                status::FLASH => {
                    return Err(format!("UPDATE_DATA at offset {offset}: flash write failed"))
                }
                status::NO_BEGIN => {
                    return Err(
                        "UPDATE_DATA: the bootloader lost the session (no UPDATE_BEGIN) — it may have reset; start the update again"
                            .into(),
                    )
                }
                status::BAD_LENGTH => {
                    return Err(format!(
                        "UPDATE_DATA at offset {offset}: the bootloader rejected a {n}-byte chunk"
                    ))
                }
                other => {
                    return Err(format!(
                        "UPDATE_DATA at offset {offset}: unexpected status {other}"
                    ))
                }
            }
            emit(offset, progress);
        }

        // -- END: the bootloader validates header + CRC from flash.
        let reply = self.simple(&[op::UPDATE_END], self.timeouts.end, "UPDATE_END")?;
        match reply[1] {
            status::OK => {}
            status::HEADER => {
                return Err(
                    "UPDATE_END: the bootloader rejected the image header (magic, vectors or length)"
                        .into(),
                )
            }
            status::CRC => {
                return Err(
                    "UPDATE_END: CRC mismatch — the flashed image differs from the file; run Install again"
                        .into(),
                )
            }
            status::NO_BEGIN => {
                return Err("UPDATE_END: no upload in progress (no UPDATE_BEGIN)".into())
            }
            other => return Err(format!("UPDATE_END: unexpected status {other}")),
        }
        emit(length, progress);
        Ok(report)
    }

    /// BOOT_RUN: the bootloader acks, then resets into the application.
    pub fn run(&mut self) -> Result<(), String> {
        let reply = self.simple(&[op::BOOT_RUN], self.timeouts.run, "BOOT_RUN")?;
        if reply[1] == status::OK {
            Ok(())
        } else {
            Err(format!("BOOT_RUN: unexpected status {}", reply[1]))
        }
    }

    /// ENTER_DFU: the bootloader acks, then resets into the ST ROM DFU
    /// (0483:df11). Only for reinstalling the bootloader itself.
    pub fn enter_rom_dfu(&mut self) -> Result<(), String> {
        let mut req = vec![op::ENTER_DFU];
        req.extend_from_slice(&ENTER_DFU_KEY);
        let reply = self.simple(&req, self.timeouts.run, "ENTER_DFU")?;
        if reply[1] == 1 {
            Ok(())
        } else {
            Err("ENTER_DFU: the bootloader did not acknowledge".into())
        }
    }
}

fn u32_at(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

/// Human wording for a local validation failure.
pub fn describe(error: AppError) -> &'static str {
    match error {
        AppError::TooShort => "file is shorter than a vector table plus header",
        AppError::Magic => "no application header at offset 0xC0",
        AppError::HeaderVersion => "unsupported application header version",
        AppError::Length => "header length is outside the application slot or the file",
        AppError::StackPointer => "initial stack pointer is not in RAM",
        AppError::ResetVector => "reset vector is not linked for the pad's application base",
        AppError::Crc => "CRC does not match the header",
        AppError::Variant => "built for the other board variant (proto vs production)",
    }
}

/// "valid" / "unstamped" / "none" as the bootloader reports it.
pub fn describe_app_valid(code: u8) -> &'static str {
    match code {
        app_valid::VALID => "valid",
        app_valid::UNSTAMPED => "unstamped",
        app_valid::NONE => "no valid firmware",
        _ => "unknown",
    }
}

pub fn version_string(v: [u8; 3]) -> String {
    format!("{}.{}.{}", v[0], v[1], v[2])
}

// ------------------------------------------------------------------ hidapi --

/// Every OpenMicro bootloader interface currently enumerated: `(path,
/// serial)`. A generic 1209:0002 is not enough — pid.codes ids are shared —
/// so the manufacturer and product strings must match too.
pub fn find_bootloaders(api: &HidApi) -> Vec<(CString, String)> {
    api.device_list()
        .filter(|info| {
            info.vendor_id() == BOOT_VID
                && info.product_id() == BOOT_PID
                && info.usage_page() == RAW_USAGE_PAGE
                && info.manufacturer_string() == Some(BOOT_MANUFACTURER)
                && info.product_string() == Some(BOOT_PRODUCT)
        })
        .map(|info| {
            (
                info.path().to_owned(),
                info.serial_number().unwrap_or("?").to_string(),
            )
        })
        .collect()
}

/// hidapi framing for the bootloader: a leading 0x00 report id on writes
/// (the interface defines none), 64-byte reads.
pub struct HidTransport {
    dev: HidDevice,
}

impl Transport for HidTransport {
    fn send(&mut self, report: &[u8]) -> Result<(), String> {
        let mut out = [0u8; BOOT_REPORT_LEN + 1];
        out[1..1 + report.len()].copy_from_slice(report);
        self.dev.write(&out).map_err(|e| e.to_string())?;
        Ok(())
    }

    fn recv(&mut self, timeout: Duration) -> Result<Option<Vec<u8>>, String> {
        let mut buf = [0u8; BOOT_REPORT_LEN];
        // hidapi takes milliseconds; a sub-millisecond remainder becomes a
        // non-blocking poll, which the caller's deadline loop then ends.
        let ms = timeout.as_millis().min(i32::MAX as u128) as i32;
        let n = self
            .dev
            .read_timeout(&mut buf, ms)
            .map_err(|e| e.to_string())?;
        Ok((n > 0).then(|| buf[..n].to_vec()))
    }
}

/// An open bootloader interface.
pub struct BootDevice {
    uploader: Uploader<HidTransport>,
}

impl BootDevice {
    pub fn open_path(api: &HidApi, path: &CStr) -> Result<Self, String> {
        let dev = api.open_path(path).map_err(|e| e.to_string())?;
        Ok(Self {
            uploader: Uploader::new(HidTransport { dev }),
        })
    }

    pub fn info(&mut self) -> Result<BootInfoReply, String> {
        self.uploader.info()
    }

    pub fn upload(
        &mut self,
        app_image: &[u8],
        progress: &mut dyn FnMut(u8),
    ) -> Result<UploadReport, String> {
        self.uploader.upload(app_image, progress)
    }

    pub fn run(&mut self) -> Result<(), String> {
        self.uploader.run()
    }

    pub fn enter_rom_dfu(&mut self) -> Result<(), String> {
        self.uploader.enter_rom_dfu()
    }
}

/// The bootloader's own protocol state machine (boot/src/update.rs), compiled
/// for the host: it is pure logic over a `Programmer` trait, so the tests
/// below can drive the real `Uploader` against the real `Updater` and catch
/// any drift between the two crates. The scripted fake in `tests` stays for
/// fault injection the real thing cannot be asked for.
#[cfg(test)]
#[allow(dead_code)]
#[path = "../../boot/src/update.rs"]
mod real_update;

#[cfg(test)]
mod tests {
    use super::*;
    use openmicro_layout::{
        app_header_bytes, stamp_app, APP_BASE, APP_SIZE, FLAG_PROTO, HEADER_OFFSET, PAGE_SIZE,
        RAM_END, RESET_OFFSET, VARIANT_PROD, VARIANT_PROTO,
    };
    use std::collections::VecDeque;

    /// One-shot misbehaviours the fake can be scripted with.
    #[derive(Clone, Debug)]
    enum Fault {
        /// Program the chunk at `offset` but never answer (a lost reply).
        DropDataReply { offset: u32 },
        /// Queue `reply` ahead of the real answer for the chunk at `offset`.
        StaleBefore { offset: u32, reply: Vec<u8> },
        /// At the chunk for `offset`, roll the write pointer back to `next`
        /// and answer status 3 (as if the previous chunk had been lost).
        OutOfOrderBackTo { offset: u32, next: u32 },
        /// Stop answering anything from the chunk at `offset` on.
        DieAt { offset: u32 },
    }

    /// A scripted bootloader: the same state machine boot/src/update.rs
    /// implements, with a RAM slot, plus fault injection.
    struct FakeBootloader {
        slot: Vec<u8>,
        begun: Option<(u32, u32)>,
        expected: u32,
        inbox: VecDeque<Vec<u8>>,
        faults: Vec<Fault>,
        requests: Vec<Vec<u8>>,
        end_status: Option<u8>,
        begin_status: Option<u8>,
        protocol: u8,
        dead: bool,
    }

    impl FakeBootloader {
        fn new() -> Self {
            Self {
                slot: vec![0xFF; APP_SIZE as usize],
                begun: None,
                expected: 0,
                inbox: VecDeque::new(),
                faults: Vec::new(),
                requests: Vec::new(),
                end_status: None,
                begin_status: None,
                protocol: 1,
                dead: false,
            }
        }

        fn take_fault(&mut self, pick: impl Fn(&Fault) -> bool) -> Option<Fault> {
            let index = self.faults.iter().position(pick)?;
            Some(self.faults.remove(index))
        }

        fn reply(&mut self, bytes: Vec<u8>) {
            if !self.dead {
                self.inbox.push_back(bytes);
            }
        }

        fn info_reply(&self) -> Vec<u8> {
            let (valid, version) = match validate_app(&self.slot, APP_BASE, APP_SIZE) {
                Ok(v) => (v.app_valid_code(), v.header().version),
                Err(_) => (app_valid::NONE, [0u8; 16]),
            };
            let mut out = [0u8; 64];
            BootInfoReply {
                status: 0,
                protocol: self.protocol,
                boot_version: [1, 0, 0],
                app_base: APP_BASE,
                app_size: APP_SIZE,
                app_valid: valid,
                app_version: version,
                page: Some(PAGE_SIZE as u16),
                max_chunk: Some(MAX_CHUNK as u8),
            }
            .encode(&mut out);
            out.to_vec()
        }

        fn requests_with(&self, opcode: u8) -> Vec<&Vec<u8>> {
            self.requests.iter().filter(|r| r[0] == opcode).collect()
        }
    }

    impl Transport for FakeBootloader {
        fn send(&mut self, report: &[u8]) -> Result<(), String> {
            assert!(report.len() <= BOOT_REPORT_LEN, "oversized report");
            self.requests.push(report.to_vec());
            match report[0] {
                op::BOOT_INFO => {
                    let r = self.info_reply();
                    self.reply(r);
                }
                op::UPDATE_BEGIN => {
                    let length = u32_at(report, 1);
                    let crc = u32_at(report, 5);
                    let st = self.begin_status.unwrap_or(
                        if (RESET_OFFSET..=APP_SIZE).contains(&length) && length % 4 == 0 {
                            status::OK
                        } else {
                            status::BAD_LENGTH
                        },
                    );
                    if st == status::OK {
                        self.slot.fill(0xFF);
                        self.begun = Some((length, crc));
                        self.expected = 0;
                    }
                    self.reply(vec![op::UPDATE_BEGIN, st]);
                }
                op::UPDATE_DATA => {
                    let offset = u32_at(report, 1);
                    let n = report[5] as usize;
                    let data = &report[6..6 + n];
                    if let Some(Fault::DieAt { .. }) =
                        self.take_fault(|f| matches!(f, Fault::DieAt { offset: o } if *o == offset))
                    {
                        self.dead = true;
                    }
                    if let Some(Fault::StaleBefore { reply, .. }) = self.take_fault(
                        |f| matches!(f, Fault::StaleBefore { offset: o, .. } if *o == offset),
                    ) {
                        self.reply(reply);
                    }
                    if let Some(Fault::OutOfOrderBackTo { next, .. }) = self.take_fault(
                        |f| matches!(f, Fault::OutOfOrderBackTo { offset: o, .. } if *o == offset),
                    ) {
                        self.expected = next;
                        self.reply(vec![op::UPDATE_DATA, status::OUT_OF_ORDER, 0, 0, 0, 0]);
                        let last = self.inbox.back_mut().unwrap();
                        last[2..6].copy_from_slice(&next.to_le_bytes());
                        return Ok(());
                    }
                    let Some((length, _)) = self.begun else {
                        self.reply(vec![op::UPDATE_DATA, status::NO_BEGIN, 0, 0, 0, 0]);
                        return Ok(());
                    };
                    let st = if offset != self.expected {
                        status::OUT_OF_ORDER
                    } else if n == 0 || n > MAX_CHUNK || (n % 4 != 0 && offset + n as u32 != length)
                    {
                        status::BAD_LENGTH
                    } else {
                        let o = offset as usize;
                        self.slot[o..o + n].copy_from_slice(data);
                        self.expected += n as u32;
                        status::OK
                    };
                    let drop = self
                        .take_fault(
                            |f| matches!(f, Fault::DropDataReply { offset: o } if *o == offset),
                        )
                        .is_some();
                    if !drop {
                        let mut r = vec![op::UPDATE_DATA, st];
                        r.extend_from_slice(&self.expected.to_le_bytes());
                        self.reply(r);
                    }
                }
                op::UPDATE_END => {
                    let st = match (self.begun, self.end_status) {
                        (_, Some(st)) => st,
                        (None, _) => status::NO_BEGIN,
                        (Some((_, crc)), None) => {
                            match validate_app(&self.slot, APP_BASE, APP_SIZE) {
                                Ok(Validity::Stamped(h)) if h.crc == crc => status::OK,
                                Ok(_) => status::HEADER,
                                Err(AppError::Crc) => status::CRC,
                                Err(_) => status::HEADER,
                            }
                        }
                    };
                    self.reply(vec![op::UPDATE_END, st]);
                }
                op::BOOT_RUN => self.reply(vec![op::BOOT_RUN, status::OK]),
                op::ENTER_DFU => {
                    let ok = report.len() >= 5 && report[1..5] == ENTER_DFU_KEY;
                    self.reply(vec![op::ENTER_DFU, ok as u8]);
                }
                other => self.reply(vec![other, status::UNKNOWN]),
            }
            Ok(())
        }

        fn recv(&mut self, _timeout: Duration) -> Result<Option<Vec<u8>>, String> {
            // Nothing queued means the (fake) deadline has passed.
            Ok(self.inbox.pop_front())
        }
    }

    /// A stamped application image of `len` bytes linked for `app_base`.
    fn stamped_image(app_base: u32, len: usize, version: &str) -> Vec<u8> {
        let mut img = unstamped_image(app_base, len, version);
        let padded = (img.len() + 3) & !3;
        img.resize(padded, 0xFF);
        stamp_app(&mut img, padded).unwrap();
        img
    }

    fn unstamped_image(app_base: u32, len: usize, version: &str) -> Vec<u8> {
        let mut img = vec![0u8; len];
        img[0..4].copy_from_slice(&(RAM_END - 16).to_le_bytes());
        img[4..8].copy_from_slice(&((app_base + RESET_OFFSET) | 1).to_le_bytes());
        img[HEADER_OFFSET as usize..RESET_OFFSET as usize]
            .copy_from_slice(&app_header_bytes(version, 0));
        for (i, b) in img[RESET_OFFSET as usize..].iter_mut().enumerate() {
            *b = (i * 13 + 5) as u8;
        }
        img
    }

    fn upload_with(
        fake: FakeBootloader,
        image: &[u8],
    ) -> (
        Result<UploadReport, String>,
        Uploader<FakeBootloader>,
        Vec<u8>,
    ) {
        let mut uploader = Uploader::new(fake);
        let mut progress = Vec::new();
        let result = uploader.upload(image, &mut |p| progress.push(p));
        (result, uploader, progress)
    }

    #[test]
    fn happy_path_programs_the_slot_in_order() {
        let image = stamped_image(APP_BASE, 1001, "0.10.0");
        let (result, mut uploader, progress) = upload_with(FakeBootloader::new(), &image);
        let report = result.unwrap();
        let fake = &uploader.transport;
        assert_eq!(report.length, 1004);
        assert_eq!(report.chunks, 1004u32.div_ceil(MAX_CHUNK as u32));
        assert_eq!((report.resumed, report.resyncs), (0, 0));
        assert_eq!(&fake.slot[..1004], &image[..]);
        // BEGIN carries the stamped length and crc.
        let begin = fake.requests_with(op::UPDATE_BEGIN);
        assert_eq!(begin.len(), 1);
        assert_eq!(u32_at(begin[0], 1), 1004);
        assert_eq!(
            u32_at(begin[0], 5),
            u32_at(&image[HEADER_OFFSET as usize + 8..], 0)
        );
        // Chunks are contiguous, word-multiples except the last, ≤ MAX_CHUNK.
        let mut offset = 0;
        for req in fake.requests_with(op::UPDATE_DATA) {
            assert_eq!(u32_at(req, 1), offset);
            let n = req[5] as usize;
            assert!(n <= MAX_CHUNK && n > 0);
            assert!(n % 4 == 0 || offset + n as u32 == 1004);
            offset += n as u32;
        }
        assert_eq!(offset, 1004);
        assert_eq!(fake.requests_with(op::UPDATE_END).len(), 1);
        // Progress is monotonic and finishes at 100.
        assert!(progress.windows(2).all(|w| w[0] < w[1]));
        assert_eq!(progress.first(), Some(&0));
        assert_eq!(progress.last(), Some(&100));
        // The fake now reports a valid app with the image's version.
        let info = uploader.info().unwrap();
        assert_eq!(info.app_valid, app_valid::VALID);
        assert_eq!(info.app_version_str(), "0.10.0");
    }

    #[test]
    fn stale_data_reply_is_discarded_not_taken_as_confirmation() {
        let image = stamped_image(APP_BASE, 600, "0.10.0");
        let mut fake = FakeBootloader::new();
        // A leftover status-0 reply claiming a different write pointer.
        let mut stale = vec![op::UPDATE_DATA, status::OK];
        stale.extend_from_slice(&600u32.to_le_bytes());
        fake.faults.push(Fault::StaleBefore {
            offset: 56,
            reply: stale,
        });
        let (result, uploader, _) = upload_with(fake, &image);
        assert!(result.is_ok(), "{result:?}");
        let fake = &uploader.transport;
        assert_eq!(&fake.slot[..600], &image[..]);
        // The chunk at 56 was sent exactly once: the stale reply was skipped
        // and the real one, right behind it, accepted.
        let at_56 = fake
            .requests_with(op::UPDATE_DATA)
            .iter()
            .filter(|r| u32_at(r, 1) == 56)
            .count();
        assert_eq!(at_56, 1);
    }

    #[test]
    fn lost_reply_resyncs_with_boot_info_and_resumes_from_status_3() {
        let image = stamped_image(APP_BASE, 700, "0.10.0");
        let mut fake = FakeBootloader::new();
        fake.faults.push(Fault::DropDataReply { offset: 112 });
        let (result, uploader, progress) = upload_with(fake, &image);
        let report = result.unwrap();
        let fake = &uploader.transport;
        assert_eq!(&fake.slot[..700], &image[..]);
        assert_eq!(report.resyncs, 1);
        assert_eq!(report.resumed, 1);
        // BOOT_INFO: once at the start, once for the re-sync.
        assert_eq!(fake.requests_with(op::BOOT_INFO).len(), 2);
        // The chunk at 112 went out twice (original + resend); the resend was
        // answered with status 3 / next = 168, and nothing was sent twice
        // after that.
        let data = fake.requests_with(op::UPDATE_DATA);
        assert_eq!(data.iter().filter(|r| u32_at(r, 1) == 112).count(), 2);
        assert_eq!(data.iter().filter(|r| u32_at(r, 1) == 168).count(), 1);
        assert!(progress.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn out_of_order_pointer_behind_us_rewinds_the_upload() {
        let image = stamped_image(APP_BASE, 500, "0.10.0");
        let mut fake = FakeBootloader::new();
        fake.faults.push(Fault::OutOfOrderBackTo {
            offset: 168,
            next: 112,
        });
        let (result, uploader, _) = upload_with(fake, &image);
        assert!(result.is_ok(), "{result:?}");
        let fake = &uploader.transport;
        assert_eq!(&fake.slot[..500], &image[..]);
        let data = fake.requests_with(op::UPDATE_DATA);
        // 112 and 168 were each sent twice: once before the rewind, once after.
        assert_eq!(data.iter().filter(|r| u32_at(r, 1) == 112).count(), 2);
        assert_eq!(data.iter().filter(|r| u32_at(r, 1) == 168).count(), 2);
    }

    #[test]
    fn a_dead_bootloader_fails_after_bounded_resyncs() {
        let image = stamped_image(APP_BASE, 500, "0.10.0");
        let mut fake = FakeBootloader::new();
        fake.faults.push(Fault::DieAt { offset: 56 });
        let (result, uploader, _) = upload_with(fake, &image);
        let err = result.unwrap_err();
        assert!(err.contains("offset 56"), "{err}");
        assert!(err.contains("re-sync failed"), "{err}");
        // The uploader gave up: one INFO up front, one failed re-sync, and
        // no runaway resend loop.
        let fake = &uploader.transport;
        assert_eq!(fake.requests_with(op::BOOT_INFO).len(), 2);
        assert!(fake.requests_with(op::UPDATE_DATA).len() <= 3);
    }

    #[test]
    fn end_and_begin_statuses_map_to_messages() {
        let image = stamped_image(APP_BASE, 300, "0.10.0");
        for (st, needle) in [
            (status::CRC, "CRC"),
            (status::HEADER, "header"),
            (status::NO_BEGIN, "UPDATE_BEGIN"),
            (0x77, "unexpected status 119"),
        ] {
            let mut fake = FakeBootloader::new();
            fake.end_status = Some(st);
            let (result, _, _) = upload_with(fake, &image);
            let err = result.unwrap_err();
            assert!(err.starts_with("UPDATE_END"), "{err}");
            assert!(err.contains(needle), "{err}");
        }
        for (st, needle) in [
            (status::BAD_LENGTH, "length"),
            (status::FLASH, "erase"),
            (0x42, "unexpected status 66"),
        ] {
            let mut fake = FakeBootloader::new();
            fake.begin_status = Some(st);
            let (result, uploader, _) = upload_with(fake, &image);
            let err = result.unwrap_err();
            assert!(err.starts_with("UPDATE_BEGIN"), "{err}");
            assert!(err.contains(needle), "{err}");
            assert!(uploader.transport.requests_with(op::UPDATE_DATA).is_empty());
        }
    }

    #[test]
    fn local_validation_refuses_bad_images_before_touching_flash() {
        // Unstamped (debugger-style) image.
        let image = unstamped_image(APP_BASE, 300, "0.10.0");
        let (result, uploader, _) = upload_with(FakeBootloader::new(), &image);
        assert!(result.unwrap_err().contains("not stamped"));
        assert!(uploader
            .transport
            .requests_with(op::UPDATE_BEGIN)
            .is_empty());
        // Linked for the wrong base.
        let image = stamped_image(APP_BASE + 0x2000, 300, "0.10.0");
        let (result, uploader, _) = upload_with(FakeBootloader::new(), &image);
        assert!(result.unwrap_err().contains("reset vector"));
        assert!(uploader
            .transport
            .requests_with(op::UPDATE_BEGIN)
            .is_empty());
        // A legacy 0x08000000 image (no header).
        let mut legacy = vec![0u8; 400];
        legacy[0..4].copy_from_slice(&0x2000_4000u32.to_le_bytes());
        legacy[4..8].copy_from_slice(&0x0800_00C1u32.to_le_bytes());
        let (result, _, _) = upload_with(FakeBootloader::new(), &legacy);
        assert!(result.unwrap_err().contains("no application header"));
    }

    #[test]
    fn info_requires_status_zero_and_protocol_one() {
        let mut fake = FakeBootloader::new();
        fake.protocol = 2;
        let mut uploader = Uploader::new(fake);
        assert!(uploader.info().unwrap_err().contains("protocol 2"));

        let mut uploader = Uploader::new(FakeBootloader::new());
        let info = uploader.info().unwrap();
        assert_eq!(info.app_valid, app_valid::NONE);
        assert_eq!(info.max_chunk, Some(MAX_CHUNK as u8));
        assert_eq!(info.page, Some(PAGE_SIZE as u16));

        // Silence is an error, not a panic.
        let mut dead = FakeBootloader::new();
        dead.dead = true;
        let mut uploader = Uploader::new(dead);
        assert!(uploader.info().unwrap_err().contains("no reply"));
    }

    #[test]
    fn run_and_rom_dfu_send_the_documented_requests() {
        let mut uploader = Uploader::new(FakeBootloader::new());
        uploader.run().unwrap();
        uploader.enter_rom_dfu().unwrap();
        let fake = &uploader.transport;
        assert_eq!(fake.requests[0], vec![op::BOOT_RUN]);
        assert_eq!(fake.requests[1], b"\x02DFU!".to_vec());
    }

    #[test]
    fn stale_reports_for_other_opcodes_are_skipped_within_the_deadline() {
        let mut fake = FakeBootloader::new();
        // Two leftovers ahead of anything we ask for.
        fake.inbox.push_back(vec![op::UPDATE_DATA, 0, 0, 0, 0, 0]);
        fake.inbox.push_back(vec![op::UPDATE_END, 0]);
        let mut uploader = Uploader::new(fake);
        uploader.run().unwrap();
        assert!(uploader.transport.inbox.is_empty());
    }

    // ---- interop with the real bootloader state machine --------------------

    use super::real_update::{Action, Programmer, Updater};

    /// The application slot as RAM with flash semantics: erase fills 0xFF,
    /// a write may only land on erased words (flash cannot set a bit back
    /// to 1), and both must stay inside the slot.
    struct RamSlot {
        slot: Vec<u8>,
        erases: Vec<(u32, u32)>,
    }

    impl Programmer for RamSlot {
        fn erase(&mut self, from: u32, to: u32) -> Result<(), ()> {
            if from % PAGE_SIZE != 0 || to % PAGE_SIZE != 0 || from >= to || to > APP_SIZE {
                return Err(());
            }
            self.slot[from as usize..to as usize].fill(0xFF);
            self.erases.push((from, to));
            Ok(())
        }

        fn write(&mut self, offset: u32, data: &[u8]) -> Result<(), ()> {
            let end = offset as usize + data.len();
            if data.is_empty() || data.len() % 4 != 0 || offset % 4 != 0 || end > self.slot.len() {
                return Err(());
            }
            if self.slot[offset as usize..end].iter().any(|&b| b != 0xFF) {
                return Err(());
            }
            self.slot[offset as usize..end].copy_from_slice(data);
            Ok(())
        }

        fn slot(&self) -> &[u8] {
            &self.slot
        }
    }

    /// A transport that hands every request to the real `Updater` and
    /// queues its 64-byte reply — except the UPDATE_DATA replies listed in
    /// `swallow` (by chunk offset, each once), which the "bus" loses.
    struct RealBootloader {
        updater: Updater<RamSlot>,
        inbox: VecDeque<Vec<u8>>,
        swallow: Vec<u32>,
        requests: Vec<Vec<u8>>,
        actions: Vec<Action>,
    }

    impl RealBootloader {
        fn new() -> Self {
            Self::with_variant(VARIANT_PROD)
        }

        fn with_variant(variant: u16) -> Self {
            let slot = RamSlot {
                slot: vec![0xFF; APP_SIZE as usize],
                erases: Vec::new(),
            };
            Self {
                updater: Updater::new(slot, "1.0.0", variant),
                inbox: VecDeque::new(),
                swallow: Vec::new(),
                requests: Vec::new(),
                actions: Vec::new(),
            }
        }

        fn slot(&self) -> &RamSlot {
            self.updater.programmer()
        }

        fn count(&self, opcode: u8, offset: Option<u32>) -> usize {
            self.requests
                .iter()
                .filter(|r| r[0] == opcode && offset.is_none_or(|o| u32_at(r, 1) == o))
                .count()
        }
    }

    impl Transport for RealBootloader {
        fn send(&mut self, report: &[u8]) -> Result<(), String> {
            assert!(report.len() <= BOOT_REPORT_LEN, "oversized report");
            self.requests.push(report.to_vec());
            let mut rep = [0u8; BOOT_REPORT_LEN];
            let action = self.updater.handle(report, &mut rep);
            self.actions.push(action);
            if report[0] == op::UPDATE_DATA {
                let offset = u32_at(report, 1);
                if let Some(i) = self.swallow.iter().position(|&o| o == offset) {
                    self.swallow.remove(i);
                    return Ok(());
                }
            }
            // hidapi delivers the whole 64-byte report, zero-padded.
            self.inbox.push_back(rep.to_vec());
            Ok(())
        }

        fn recv(&mut self, _timeout: Duration) -> Result<Option<Vec<u8>>, String> {
            Ok(self.inbox.pop_front())
        }
    }

    #[test]
    fn real_bootloader_takes_a_multi_chunk_upload_and_reports_it_valid() {
        // 5004 bytes: 89 full chunks and a 20-byte tail, three flash pages.
        let image = stamped_image(APP_BASE, 5001, "0.10.0");
        let mut uploader = Uploader::new(RealBootloader::new());
        let before = uploader.info().unwrap();
        assert_eq!(before.app_valid, app_valid::NONE);
        assert_eq!(before.boot_version, [1, 0, 0]);
        assert_eq!(before.page, Some(PAGE_SIZE as u16));
        assert_eq!(before.max_chunk, Some(MAX_CHUNK as u8));

        let mut progress = Vec::new();
        let report = uploader.upload(&image, &mut |p| progress.push(p)).unwrap();
        assert_eq!(report.length, 5004);
        assert_eq!(report.chunks, 90);
        assert_eq!((report.resumed, report.resyncs), (0, 0));
        assert_eq!(progress.first(), Some(&0));
        assert_eq!(progress.last(), Some(&100));

        let dev = &uploader.transport;
        assert_eq!(&dev.slot().slot[..5004], &image[..]);
        assert!(dev.slot().slot[5004..].iter().all(|&b| b == 0xFF));
        assert_eq!(dev.slot().erases, vec![(0, 3 * PAGE_SIZE)]);
        assert_eq!(dev.count(op::UPDATE_BEGIN, None), 1);
        assert_eq!(dev.count(op::UPDATE_DATA, None), 90);
        assert_eq!(dev.count(op::UPDATE_END, None), 1);
        assert!(dev.actions.iter().all(|a| matches!(a, Action::Reply(_))));

        let after = uploader.info().unwrap();
        assert_eq!(after.app_valid, app_valid::VALID);
        assert_eq!(after.app_version_str(), "0.10.0");

        // BOOT_RUN and ENTER_DFU are acked and turn into resets.
        uploader.run().unwrap();
        uploader.enter_rom_dfu().unwrap();
        let actions = &uploader.transport.actions;
        assert_eq!(actions[actions.len() - 2], Action::ResetToApp);
        assert_eq!(actions[actions.len() - 1], Action::ResetToDfu);
    }

    #[test]
    fn real_bootloader_resumes_after_swallowed_data_replies() {
        // 3000 bytes: 53 full chunks, the last (32 bytes) at offset 2968.
        let image = stamped_image(APP_BASE, 3000, "0.10.0");
        let mut dev = RealBootloader::new();
        dev.swallow = vec![112, 2968];
        let mut uploader = Uploader::new(dev);
        let mut progress = Vec::new();
        let report = uploader.upload(&image, &mut |p| progress.push(p)).unwrap();
        // Each lost reply costs one BOOT_INFO re-sync; the resend is
        // answered OUT_OF_ORDER with the pointer one chunk ahead (the real
        // bootloader did program it), and the upload resumes there. Nothing
        // is programmed twice: the RAM slot refuses a non-erased word.
        assert_eq!(report.resyncs, 2);
        assert_eq!(report.resumed, 2);
        assert_eq!(report.chunks, 54 - 2);
        assert!(progress.windows(2).all(|w| w[0] < w[1]));
        let dev = &uploader.transport;
        assert_eq!(&dev.slot().slot[..3000], &image[..]);
        assert_eq!(dev.count(op::UPDATE_DATA, Some(112)), 2);
        assert_eq!(dev.count(op::UPDATE_DATA, Some(168)), 1);
        assert_eq!(dev.count(op::UPDATE_DATA, Some(2968)), 2);
        // One BOOT_INFO up front, one per re-sync.
        assert_eq!(dev.count(op::BOOT_INFO, None), 3);
        assert!(dev.swallow.is_empty());
        let info = uploader.info().unwrap();
        assert_eq!(info.app_valid, app_valid::VALID);
        assert_eq!(info.app_version_str(), "0.10.0");
    }

    #[test]
    fn real_bootloader_refuses_an_application_built_for_the_other_pin_map() {
        // A proto build (FLAG_PROTO in the header) is refused by a prod
        // bootloader at UPDATE_END, which the uploader reports as a header
        // rejection; the same image is fine on a proto bootloader.
        let mut image = unstamped_image(APP_BASE, 700, "0.10.0");
        image[HEADER_OFFSET as usize + 14..HEADER_OFFSET as usize + 16]
            .copy_from_slice(&FLAG_PROTO.to_le_bytes());
        let padded = (image.len() + 3) & !3;
        image.resize(padded, 0xFF);
        stamp_app(&mut image, padded).unwrap();

        let mut uploader = Uploader::new(RealBootloader::new());
        let err = uploader.upload(&image, &mut |_| {}).unwrap_err();
        assert!(
            err.contains("UPDATE_END") && err.contains("header"),
            "{err}"
        );

        let mut uploader = Uploader::new(RealBootloader::with_variant(VARIANT_PROTO));
        uploader.upload(&image, &mut |_| {}).unwrap();
        assert_eq!(uploader.info().unwrap().app_valid, app_valid::VALID);
    }

    #[test]
    fn real_bootloader_rejects_what_the_uploader_would_never_send() {
        // The local checks already refuse these; the bootloader agrees.
        let mut dev = RealBootloader::new();
        let mut rep = [0u8; BOOT_REPORT_LEN];
        // DATA before BEGIN.
        dev.updater
            .handle(&[op::UPDATE_DATA, 0, 0, 0, 0, 4, 1, 2, 3, 4], &mut rep);
        assert_eq!(rep[1], status::NO_BEGIN);
        // A length the slot cannot hold.
        let mut begin = vec![op::UPDATE_BEGIN];
        begin.extend_from_slice(&(APP_SIZE + 4).to_le_bytes());
        begin.extend_from_slice(&[0; 4]);
        dev.updater.handle(&begin, &mut rep);
        assert_eq!(rep[1], status::BAD_LENGTH);
        assert!(dev.slot().erases.is_empty());
        // END with nothing begun.
        dev.updater.handle(&[op::UPDATE_END], &mut rep);
        assert_eq!(rep[1], status::NO_BEGIN);
    }

    /// Hardware: the exact primitives the worker uses to notice a pad in
    /// bootloader mode. Run with a pad parked in its bootloader:
    /// `cargo test --lib -- --ignored hw_bootloader --nocapture`.
    #[test]
    #[ignore]
    fn hw_bootloader_is_found_and_answers_boot_info() {
        let api = hidapi::HidApi::new().expect("hidapi");
        let found = find_bootloaders(&api);
        eprintln!(
            "bootloader pads: {:?}",
            found.iter().map(|(_, serial)| serial).collect::<Vec<_>>()
        );
        assert!(!found.is_empty(), "no pad in bootloader mode on the bus");
        let (path, serial) = &found[0];
        let mut dev = BootDevice::open_path(&api, path).expect("open the bootloader");
        let info = dev.info().expect("BOOT_INFO");
        eprintln!("serial {serial}: {info:?}");
        assert_eq!(info.status, 0);
        assert_eq!(info.protocol, 1);
    }
}
