//! The contract between the OpenMicro v1 bootloader, the application firmware
//! and the host tools: where things live in flash and RAM, what the image
//! header and the bootloader info block look like, the RAM handoff words,
//! and the bootloader's HID protocol. Pure `no_std` logic with no
//! dependencies, so the same code validates images on the pad, on the host
//! and in tests.
//!
//! ```text
//! flash 0x08000000  bootloader        BOOT_SIZE  (info block at +0xC0)
//!       0x08006000  application       APP_SIZE   (header at +0xC0, Reset at +0xE0)
//!       0x0801B000  keymap.json slot  12 KiB     (fw/src/codex/files.rs)
//!       0x0801E000  smart_actions     6 KiB
//!       0x0801F800  config page       2 KiB      (fw/src/keymap.rs)
//! ram   0x20000000  application vector table copy (0xC0 bytes, SYSCFG remap)
//!       0x200000C0  .data/.bss/stack of whichever image runs
//!       0x20003FF0  handoff words (request, !request, reason, faults)
//! ```

#![cfg_attr(not(test), no_std)]

// ---- flash / RAM ----------------------------------------------------------

pub const FLASH_BASE: u32 = 0x0800_0000;
pub const FLASH_SIZE: u32 = 0x2_0000;
pub const PAGE_SIZE: u32 = 2048;

pub const BOOT_BASE: u32 = FLASH_BASE;
pub const BOOT_SIZE: u32 = 0x6000;
/// Where the application image starts; also its link address.
pub const APP_BASE: u32 = BOOT_BASE + BOOT_SIZE;
pub const APP_SIZE: u32 = 0x1_5000;
pub const APP_END: u32 = APP_BASE + APP_SIZE;
/// First byte the bootloader must never erase (Work Louder file slots and
/// the config page live above it).
pub const DATA_BASE: u32 = 0x0801_B000;

pub const RAM_BASE: u32 = 0x2000_0000;
pub const RAM_SIZE: u32 = 0x4000;
pub const RAM_END: u32 = RAM_BASE + RAM_SIZE;

/// 16 core + 32 device vectors on the F072.
pub const VECTORS_LEN: u32 = 0xC0;
/// Both the application header and the bootloader info block sit right after
/// the vector table.
pub const HEADER_OFFSET: u32 = VECTORS_LEN;
pub const HEADER_LEN: u32 = 32;
/// `_stext`: cortex-m-rt places `Reset` first in `.text`, so the reset vector
/// of a correctly linked image is exactly `base + RESET_OFFSET | 1`.
pub const RESET_OFFSET: u32 = HEADER_OFFSET + HEADER_LEN;

// ---- handoff words (survive a system reset, not a power cycle) ------------

pub const HANDOFF_REQUEST: u32 = 0x2000_3FF0;
pub const HANDOFF_REQUEST_INV: u32 = 0x2000_3FF4;
pub const HANDOFF_REASON: u32 = 0x2000_3FF8;
pub const HANDOFF_FAULTS: u32 = 0x2000_3FFC;

/// "BOOT": stay in the bootloader's update mode.
pub const REQ_BOOTLOADER: u32 = 0x424F_4F54;
/// The historical DFU magic: jump to the ST ROM DFU (0483:DF11).
pub const REQ_ROM_DFU: u32 = 0xB007_10AD;
/// "RUN!": skip the encoder-switch check, validate and start the application.
pub const REQ_RUN: u32 = 0x214E_5552;

pub const REASON_NORMAL: u32 = 1;
pub const REASON_AFTER_UPDATE: u32 = 2;
pub const REASON_SWITCH: u32 = 3;

/// The faults word is `FAULTS_TAG | count`; any other value counts as zero.
pub const FAULTS_TAG: u32 = 0x5A5A_0000;

pub const fn faults_count(word: u32) -> u32 {
    if word & 0xFFFF_0000 == FAULTS_TAG {
        word & 0xFF
    } else {
        0
    }
}

pub const fn faults_word(count: u32) -> u32 {
    FAULTS_TAG | (count & 0xFF)
}

// ---- magics ---------------------------------------------------------------

/// "OMKA" (bytes O, M, K, A little-endian) at APP_BASE + HEADER_OFFSET.
pub const APP_MAGIC: u32 = 0x414B_4D4F;
/// "OMKB" at BOOT_BASE + HEADER_OFFSET.
pub const BOOT_MAGIC: u32 = 0x424B_4D4F;
pub const HEADER_VERSION: u16 = 1;
pub const BOOT_PROTOCOL: u16 = 1;
pub const VARIANT_PROD: u16 = 0;
pub const VARIANT_PROTO: u16 = 1;
/// Application header `flags` bit: built with the `proto` pin map. The
/// bootloader refuses an upload whose flag disagrees with its own variant.
pub const FLAG_PROTO: u16 = 1;

/// The header flags an application built for `variant` must carry.
pub const fn flags_for_variant(variant: u16) -> u16 {
    if variant == VARIANT_PROTO {
        FLAG_PROTO
    } else {
        0
    }
}

// ---- USB identity of the bootloader ---------------------------------------

pub const BOOT_VID: u16 = 0x1209;
pub const BOOT_PID: u16 = 0x0002;
pub const BOOT_MANUFACTURER: &str = "conol";
pub const BOOT_PRODUCT: &str = "OpenMicro Bootloader";
/// Vendor usage page / usage shared with the application's raw interface.
pub const RAW_USAGE_PAGE: u16 = 0xFF60;
pub const RAW_USAGE: u16 = 0x61;
pub const BOOT_REPORT_LEN: usize = 64;
pub const APP_REPORT_LEN: usize = 32;

// ---- protocol -------------------------------------------------------------

/// Opcodes. Replies echo the opcode in byte 0; byte 0 never has bit 7 set
/// (the application uses 0x80+ for unsolicited input events).
pub mod op {
    pub const VERSION: u8 = 0x01;
    pub const ENTER_DFU: u8 = 0x02;
    /// Application only: reboot into the bootloader's update mode.
    pub const ENTER_BOOT: u8 = 0x12;
    pub const BOOT_INFO: u8 = 0x20;
    pub const UPDATE_BEGIN: u8 = 0x21;
    pub const UPDATE_DATA: u8 = 0x22;
    pub const UPDATE_END: u8 = 0x23;
    pub const BOOT_RUN: u8 = 0x24;
}

pub const ENTER_DFU_KEY: [u8; 4] = *b"DFU!";
pub const ENTER_BOOT_KEY: [u8; 4] = *b"BOOT";

/// Status codes in byte 1 of a reply.
pub mod status {
    pub const OK: u8 = 0;
    pub const BAD_LENGTH: u8 = 1;
    pub const FLASH: u8 = 2;
    pub const OUT_OF_ORDER: u8 = 3;
    pub const HEADER: u8 = 4;
    pub const NO_BEGIN: u8 = 5;
    pub const CRC: u8 = 6;
    pub const UNKNOWN: u8 = 0xFF;
}

/// Payload bytes per UPDATE_DATA report: 64 - op - offset - len.
pub const MAX_CHUNK: usize = 56;

pub mod app_valid {
    pub const NONE: u8 = 0;
    pub const VALID: u8 = 1;
    /// Magic ok, length and crc zero: flashed from an ELF by a debugger.
    pub const UNSTAMPED: u8 = 2;
}

// ---- image header ---------------------------------------------------------

/// The 32-byte application header at APP_BASE + HEADER_OFFSET.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct AppHeader {
    /// Image bytes from APP_BASE, multiple of 4. 0 in an unstamped image.
    pub length: u32,
    /// CRC-32 (ISO-HDLC) over the image with this field as zero.
    pub crc: u32,
    pub header_version: u16,
    pub flags: u16,
    /// NUL-padded ASCII version.
    pub version: [u8; 16],
}

impl AppHeader {
    pub fn parse(bytes: &[u8]) -> Option<AppHeader> {
        if bytes.len() < HEADER_LEN as usize || u32_at(bytes, 0) != APP_MAGIC {
            return None;
        }
        let mut version = [0u8; 16];
        version.copy_from_slice(&bytes[16..32]);
        Some(AppHeader {
            length: u32_at(bytes, 4),
            crc: u32_at(bytes, 8),
            header_version: u16_at(bytes, 12),
            flags: u16_at(bytes, 14),
            version,
        })
    }

    pub fn encode(&self) -> [u8; 32] {
        let mut b = [0u8; 32];
        b[0..4].copy_from_slice(&APP_MAGIC.to_le_bytes());
        b[4..8].copy_from_slice(&self.length.to_le_bytes());
        b[8..12].copy_from_slice(&self.crc.to_le_bytes());
        b[12..14].copy_from_slice(&self.header_version.to_le_bytes());
        b[14..16].copy_from_slice(&self.flags.to_le_bytes());
        b[16..32].copy_from_slice(&self.version);
        b
    }

    pub fn is_unstamped(&self) -> bool {
        self.length == 0 && self.crc == 0
    }

    pub fn version_str(&self) -> &str {
        cstr(&self.version)
    }
}

/// The 32-byte bootloader info block at BOOT_BASE + HEADER_OFFSET.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct BootInfo {
    pub protocol: u16,
    /// VARIANT_PROD / VARIANT_PROTO.
    pub variant: u16,
    pub app_base: u32,
    pub app_size: u32,
    pub version: [u8; 16],
}

impl BootInfo {
    pub fn parse(bytes: &[u8]) -> Option<BootInfo> {
        if bytes.len() < HEADER_LEN as usize || u32_at(bytes, 0) != BOOT_MAGIC {
            return None;
        }
        let mut version = [0u8; 16];
        version.copy_from_slice(&bytes[16..32]);
        Some(BootInfo {
            protocol: u16_at(bytes, 4),
            variant: u16_at(bytes, 6),
            app_base: u32_at(bytes, 8),
            app_size: u32_at(bytes, 12),
            version,
        })
    }

    pub fn encode(&self) -> [u8; 32] {
        let mut b = [0u8; 32];
        b[0..4].copy_from_slice(&BOOT_MAGIC.to_le_bytes());
        b[4..6].copy_from_slice(&self.protocol.to_le_bytes());
        b[6..8].copy_from_slice(&self.variant.to_le_bytes());
        b[8..12].copy_from_slice(&self.app_base.to_le_bytes());
        b[12..16].copy_from_slice(&self.app_size.to_le_bytes());
        b[16..32].copy_from_slice(&self.version);
        b
    }

    pub fn version_str(&self) -> &str {
        cstr(&self.version)
    }
}

/// Bytes of an application header as linked into the ELF (unstamped): the
/// build patches length and crc into the .bin afterwards. `flags` is
/// `FLAG_PROTO` for a `proto` build, else 0.
pub const fn app_header_bytes(version: &str, flags: u16) -> [u8; 32] {
    let mut b = [0u8; 32];
    let m = APP_MAGIC.to_le_bytes();
    let hv = HEADER_VERSION.to_le_bytes();
    let fl = flags.to_le_bytes();
    b[0] = m[0];
    b[1] = m[1];
    b[2] = m[2];
    b[3] = m[3];
    b[12] = hv[0];
    b[13] = hv[1];
    b[14] = fl[0];
    b[15] = fl[1];
    let v = version_bytes(version);
    let mut i = 0;
    while i < 16 {
        b[16 + i] = v[i];
        i += 1;
    }
    b
}

/// Bytes of the bootloader info block as linked into the bootloader.
pub const fn boot_info_bytes(version: &str, variant: u16) -> [u8; 32] {
    let mut b = [0u8; 32];
    let m = BOOT_MAGIC.to_le_bytes();
    let p = BOOT_PROTOCOL.to_le_bytes();
    let va = variant.to_le_bytes();
    let ab = APP_BASE.to_le_bytes();
    let asz = APP_SIZE.to_le_bytes();
    b[0] = m[0];
    b[1] = m[1];
    b[2] = m[2];
    b[3] = m[3];
    b[4] = p[0];
    b[5] = p[1];
    b[6] = va[0];
    b[7] = va[1];
    let mut i = 0;
    while i < 4 {
        b[8 + i] = ab[i];
        b[12 + i] = asz[i];
        i += 1;
    }
    let v = version_bytes(version);
    let mut i = 0;
    while i < 16 {
        b[16 + i] = v[i];
        i += 1;
    }
    b
}

/// "1.2.3" → [1, 2, 3] (prerelease suffixes ignored, missing parts 0).
pub const fn version_triple(s: &str) -> [u8; 3] {
    version_triple_bytes(s.as_bytes())
}

/// [`version_triple`] over the raw bytes of a NUL-padded version field, so
/// firmware can parse a header/info-block version without UTF-8 validation
/// (which costs ~300 bytes of flash it has no other use for).
pub const fn version_triple_bytes(b: &[u8]) -> [u8; 3] {
    let mut out = [0u8; 3];
    let mut part = 0;
    let mut i = 0;
    while i < b.len() && part < 3 {
        let c = b[i];
        if c >= b'0' && c <= b'9' {
            out[part] = out[part].saturating_mul(10).saturating_add(c - b'0');
        } else if c == b'.' {
            part += 1;
        } else {
            break;
        }
        i += 1;
    }
    out
}

/// A version string as a NUL-padded 16-byte field (truncated if longer).
pub const fn version_bytes(s: &str) -> [u8; 16] {
    let b = s.as_bytes();
    let mut out = [0u8; 16];
    let mut i = 0;
    while i < 16 && i < b.len() {
        out[i] = b[i];
        i += 1;
    }
    out
}

// ---- BOOT_INFO reply --------------------------------------------------------

/// The 31-byte BOOT_INFO reply both images send; the bootloader appends
/// `page` and `max_chunk` (34 bytes) on its 64-byte interface.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct BootInfoReply {
    pub status: u8,
    pub protocol: u8,
    pub boot_version: [u8; 3],
    pub app_base: u32,
    pub app_size: u32,
    pub app_valid: u8,
    pub app_version: [u8; 16],
    pub page: Option<u16>,
    pub max_chunk: Option<u8>,
}

pub const BOOT_INFO_REPLY_LEN: usize = 31;
pub const BOOT_INFO_REPLY_EXT_LEN: usize = 34;

impl BootInfoReply {
    /// Writes the reply into `out` (needs 31 bytes; the extended fields are
    /// written when `page`/`max_chunk` are set and 34 bytes are available).
    /// Returns the number of bytes written.
    pub fn encode(&self, out: &mut [u8]) -> usize {
        out[0] = op::BOOT_INFO;
        out[1] = self.status;
        out[2] = self.protocol;
        out[3..6].copy_from_slice(&self.boot_version);
        out[6..10].copy_from_slice(&self.app_base.to_le_bytes());
        out[10..14].copy_from_slice(&self.app_size.to_le_bytes());
        out[14] = self.app_valid;
        out[15..31].copy_from_slice(&self.app_version);
        if let (Some(page), Some(chunk), true) = (self.page, self.max_chunk, out.len() >= 34) {
            out[31..33].copy_from_slice(&page.to_le_bytes());
            out[33] = chunk;
            BOOT_INFO_REPLY_EXT_LEN
        } else {
            BOOT_INFO_REPLY_LEN
        }
    }

    pub fn parse(rep: &[u8]) -> Option<BootInfoReply> {
        if rep.len() < BOOT_INFO_REPLY_LEN || rep[0] != op::BOOT_INFO {
            return None;
        }
        let mut boot_version = [0u8; 3];
        boot_version.copy_from_slice(&rep[3..6]);
        let mut app_version = [0u8; 16];
        app_version.copy_from_slice(&rep[15..31]);
        let ext = rep.len() >= BOOT_INFO_REPLY_EXT_LEN;
        Some(BootInfoReply {
            status: rep[1],
            protocol: rep[2],
            boot_version,
            app_base: u32_at(rep, 6),
            app_size: u32_at(rep, 10),
            app_valid: rep[14],
            app_version,
            page: if ext { Some(u16_at(rep, 31)) } else { None },
            max_chunk: if ext { Some(rep[33]) } else { None },
        })
    }

    pub fn app_version_str(&self) -> &str {
        cstr(&self.app_version)
    }
}

// ---- CRC-32 (ISO-HDLC: zlib.crc32, crc32fast) ------------------------------

const CRC_TABLE: [u32; 256] = {
    let mut t = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            k += 1;
        }
        t[i] = c;
        i += 1;
    }
    t
};

/// Streaming CRC-32.
#[derive(Clone, Copy, Debug)]
pub struct Crc32 {
    state: u32,
}

impl Default for Crc32 {
    fn default() -> Self {
        Self::new()
    }
}

impl Crc32 {
    pub const fn new() -> Self {
        Self { state: 0xFFFF_FFFF }
    }

    pub fn update(&mut self, data: &[u8]) {
        let mut c = self.state;
        for &b in data {
            c = CRC_TABLE[((c ^ b as u32) & 0xFF) as usize] ^ (c >> 8);
        }
        self.state = c;
    }

    pub fn finish(self) -> u32 {
        !self.state
    }
}

pub fn crc32(data: &[u8]) -> u32 {
    let mut c = Crc32::new();
    c.update(data);
    c.finish()
}

/// CRC of an application image with the header's crc field taken as zero.
pub fn image_crc(image: &[u8]) -> u32 {
    let crc_at = (HEADER_OFFSET + 8) as usize;
    let mut c = Crc32::new();
    c.update(&image[..crc_at]);
    c.update(&[0, 0, 0, 0]);
    c.update(&image[crc_at + 4..]);
    c.finish()
}

// ---- validation -------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppError {
    /// Fewer bytes than a vector table plus header.
    TooShort,
    Magic,
    HeaderVersion,
    /// Length outside RESET_OFFSET..=app_size, not a multiple of 4, or
    /// beyond the bytes given.
    Length,
    /// Initial stack pointer outside RAM or not 8-byte aligned.
    StackPointer,
    /// Reset vector is not `app_base + RESET_OFFSET | 1`.
    ResetVector,
    Crc,
    /// Built for the other board variant (`FLAG_PROTO` disagrees).
    Variant,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Validity {
    /// Stamped header, CRC verified.
    Stamped(AppHeader),
    /// Magic ok, length and crc zero (debugger-flashed): vectors checked only.
    Unstamped(AppHeader),
}

impl Validity {
    pub fn header(&self) -> &AppHeader {
        match self {
            Validity::Stamped(h) | Validity::Unstamped(h) => h,
        }
    }

    pub fn app_valid_code(&self) -> u8 {
        match self {
            Validity::Stamped(_) => app_valid::VALID,
            Validity::Unstamped(_) => app_valid::UNSTAMPED,
        }
    }
}

/// Validates an application image. `image` starts at `app_base` (it may be
/// the whole slot; bytes past the header's length are ignored).
pub fn validate_app(image: &[u8], app_base: u32, app_size: u32) -> Result<Validity, AppError> {
    if image.len() < RESET_OFFSET as usize {
        return Err(AppError::TooShort);
    }
    let header = AppHeader::parse(&image[HEADER_OFFSET as usize..]).ok_or(AppError::Magic)?;
    if header.header_version != HEADER_VERSION {
        return Err(AppError::HeaderVersion);
    }
    let sp = u32_at(image, 0);
    if sp < RAM_BASE || sp > RAM_END || sp % 8 != 0 {
        return Err(AppError::StackPointer);
    }
    if u32_at(image, 4) != (app_base + RESET_OFFSET) | 1 {
        return Err(AppError::ResetVector);
    }
    if header.is_unstamped() {
        return Ok(Validity::Unstamped(header));
    }
    let len = header.length;
    if len < RESET_OFFSET || len > app_size || len % 4 != 0 || len as usize > image.len() {
        return Err(AppError::Length);
    }
    if image_crc(&image[..len as usize]) != header.crc {
        return Err(AppError::Crc);
    }
    Ok(Validity::Stamped(header))
}

/// `validate_app`, plus the board-variant rule: the header's `FLAG_PROTO`
/// must match `flags_for_variant(variant)`. The bootloader uses this for
/// every decision (boot, BOOT_INFO, UPDATE_END) so an image built for the
/// other pin map is never started, not even when flashed by a debugger.
pub fn validate_app_for(
    image: &[u8],
    app_base: u32,
    app_size: u32,
    variant: u16,
) -> Result<Validity, AppError> {
    let v = validate_app(image, app_base, app_size)?;
    if v.header().flags & FLAG_PROTO != flags_for_variant(variant) {
        return Err(AppError::Variant);
    }
    Ok(v)
}

/// Stamps an in-memory application image: pads to a multiple of 4 with
/// 0xFF (returns the new length) and writes length and crc into the header.
/// Fails if the header magic is missing.
pub fn stamp_app(image: &mut [u8], padded_len: usize) -> Result<AppHeader, AppError> {
    if padded_len < RESET_OFFSET as usize || padded_len % 4 != 0 || padded_len > image.len() {
        return Err(AppError::Length);
    }
    let h = AppHeader::parse(&image[HEADER_OFFSET as usize..]).ok_or(AppError::Magic)?;
    let off = HEADER_OFFSET as usize;
    image[off + 4..off + 8].copy_from_slice(&(padded_len as u32).to_le_bytes());
    image[off + 8..off + 12].copy_from_slice(&[0, 0, 0, 0]);
    let crc = image_crc(&image[..padded_len]);
    image[off + 8..off + 12].copy_from_slice(&crc.to_le_bytes());
    Ok(AppHeader {
        length: padded_len as u32,
        crc,
        ..h
    })
}

// ---- helpers ----------------------------------------------------------------

pub fn u32_at(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

pub fn u16_at(b: &[u8], i: usize) -> u16 {
    u16::from_le_bytes([b[i], b[i + 1]])
}

fn cstr(b: &[u8]) -> &str {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    core::str::from_utf8(&b[..end]).unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(app_base: u32, len: usize, version: &str) -> Vec<u8> {
        let mut img = vec![0u8; len];
        img[0..4].copy_from_slice(&(RAM_END - 16).to_le_bytes());
        img[4..8].copy_from_slice(&((app_base + RESET_OFFSET) | 1).to_le_bytes());
        img[HEADER_OFFSET as usize..RESET_OFFSET as usize].copy_from_slice(&app_header_bytes(version, 0));
        for (i, b) in img[RESET_OFFSET as usize..].iter_mut().enumerate() {
            *b = (i * 7 + 3) as u8;
        }
        img
    }

    #[test]
    fn crc_vectors() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0);
        let mut c = Crc32::new();
        c.update(b"1234");
        c.update(b"56789");
        assert_eq!(c.finish(), 0xCBF4_3926);
    }

    #[test]
    fn layout_constants_are_consistent() {
        assert_eq!(APP_BASE, 0x0800_6000);
        assert_eq!(APP_END, DATA_BASE);
        assert_eq!(BOOT_SIZE % PAGE_SIZE, 0);
        assert_eq!(APP_SIZE % PAGE_SIZE, 0);
        assert_eq!(RESET_OFFSET, 0xE0);
        assert_eq!(HANDOFF_FAULTS + 4, RAM_END);
        assert_eq!(&APP_MAGIC.to_le_bytes(), b"OMKA");
        assert_eq!(&BOOT_MAGIC.to_le_bytes(), b"OMKB");
        assert_eq!(&REQ_BOOTLOADER.to_be_bytes(), b"BOOT");
        assert_eq!(&REQ_RUN.to_be_bytes(), b"!NUR");
    }

    #[test]
    fn headers_round_trip() {
        let h = AppHeader {
            length: 1234,
            crc: 0xDEAD_BEEF,
            header_version: 1,
            flags: 0,
            version: version_bytes("0.10.0"),
        };
        assert_eq!(AppHeader::parse(&h.encode()), Some(h));
        assert_eq!(h.version_str(), "0.10.0");
        let unstamped = AppHeader::parse(&app_header_bytes("0.10.0", 0)).unwrap();
        assert!(unstamped.is_unstamped());
        assert_eq!(unstamped.header_version, HEADER_VERSION);
        assert_eq!(unstamped.flags, 0);
        let proto = AppHeader::parse(&app_header_bytes("0.10.0", FLAG_PROTO)).unwrap();
        assert_eq!(proto.flags, FLAG_PROTO);
        assert_eq!(flags_for_variant(VARIANT_PROTO), FLAG_PROTO);
        assert_eq!(flags_for_variant(VARIANT_PROD), 0);
        let b = BootInfo::parse(&boot_info_bytes("1.0.0", VARIANT_PROTO)).unwrap();
        assert_eq!(b.app_base, APP_BASE);
        assert_eq!(b.app_size, APP_SIZE);
        assert_eq!(b.protocol, BOOT_PROTOCOL);
        assert_eq!(b.variant, VARIANT_PROTO);
        assert_eq!(b.version_str(), "1.0.0");
        assert_eq!(BootInfo::parse(&b.encode()), Some(b));
        assert!(AppHeader::parse(&[0u8; 32]).is_none());
        assert!(AppHeader::parse(&[0u8; 8]).is_none());
    }

    #[test]
    fn version_helpers() {
        assert_eq!(version_triple("1.2.3"), [1, 2, 3]);
        assert_eq!(version_triple("0.10.0-rc.1"), [0, 10, 0]);
        assert_eq!(version_triple("7"), [7, 0, 0]);
        assert_eq!(version_triple("boot"), [0, 0, 0]);
        assert_eq!(version_triple_bytes(b"1.0.0\0\0\0\0"), [1, 0, 0]);
        assert_eq!(version_triple_bytes(&version_bytes("0.10.0")), [0, 10, 0]);
        assert_eq!(version_triple_bytes(&[0u8; 16]), [0, 0, 0]);
        assert_eq!(version_triple_bytes(&[0xFFu8; 16]), [0, 0, 0]);
        assert_eq!(&version_bytes("0.10.0")[..7], b"0.10.0\0");
    }

    #[test]
    fn boot_info_reply_round_trip() {
        let r = BootInfoReply {
            status: 0,
            protocol: 1,
            boot_version: [1, 0, 0],
            app_base: APP_BASE,
            app_size: APP_SIZE,
            app_valid: app_valid::VALID,
            app_version: version_bytes("0.10.0"),
            page: Some(2048),
            max_chunk: Some(MAX_CHUNK as u8),
        };
        let mut short = [0u8; 32];
        assert_eq!(r.encode(&mut short), BOOT_INFO_REPLY_LEN);
        let p = BootInfoReply::parse(&short[..BOOT_INFO_REPLY_LEN]).unwrap();
        assert_eq!(p.page, None);
        assert_eq!(p.app_base, APP_BASE);
        assert_eq!(p.app_version_str(), "0.10.0");
        let mut long = [0u8; 64];
        assert_eq!(r.encode(&mut long), BOOT_INFO_REPLY_EXT_LEN);
        assert_eq!(BootInfoReply::parse(&long), Some(r));
        assert!(BootInfoReply::parse(&long[..30]).is_none());
    }

    #[test]
    fn stamp_then_validate() {
        let mut img = image(APP_BASE, 1000, "0.10.0");
        let padded = (img.len() + 3) & !3;
        img.resize(padded, 0xFF);
        let h = stamp_app(&mut img, padded).unwrap();
        assert_eq!(h.length, padded as u32);
        match validate_app(&img, APP_BASE, APP_SIZE).unwrap() {
            Validity::Stamped(got) => assert_eq!(got, h),
            other => panic!("{other:?}"),
        }
        // Extra slot bytes after the image are ignored.
        let mut slot = img.clone();
        slot.resize(APP_SIZE as usize, 0xFF);
        assert!(matches!(validate_app(&slot, APP_BASE, APP_SIZE), Ok(Validity::Stamped(_))));
        // A flipped byte fails the CRC.
        let mut bad = img.clone();
        bad[500] ^= 1;
        assert_eq!(validate_app(&bad, APP_BASE, APP_SIZE), Err(AppError::Crc));
        // A flipped vector-table byte fails too (the CRC covers it).
        let mut bad = img.clone();
        bad[0x40] ^= 1;
        assert_eq!(validate_app(&bad, APP_BASE, APP_SIZE), Err(AppError::Crc));
    }

    #[test]
    fn unstamped_boots_on_vectors_only() {
        let img = image(APP_BASE, 1000, "0.10.0");
        assert!(matches!(validate_app(&img, APP_BASE, APP_SIZE), Ok(Validity::Unstamped(_))));
        assert_eq!(
            validate_app(&img, APP_BASE, APP_SIZE).unwrap().app_valid_code(),
            app_valid::UNSTAMPED
        );
    }

    #[test]
    fn rejects_bad_images() {
        assert_eq!(validate_app(&[0xFF; 0x80], APP_BASE, APP_SIZE), Err(AppError::TooShort));
        assert_eq!(validate_app(&[0xFF; 0x200], APP_BASE, APP_SIZE), Err(AppError::Magic));
        // Linked for another base.
        let img = image(APP_BASE + 0x1000, 1000, "0.10.0");
        assert_eq!(validate_app(&img, APP_BASE, APP_SIZE), Err(AppError::ResetVector));
        // Bad stack pointer.
        let mut img = image(APP_BASE, 1000, "0.10.0");
        img[0..4].copy_from_slice(&0x2000_4004u32.to_le_bytes());
        assert_eq!(validate_app(&img, APP_BASE, APP_SIZE), Err(AppError::StackPointer));
        // Stamped length beyond the bytes given / the slot.
        let mut img = image(APP_BASE, 1000, "0.10.0");
        let off = HEADER_OFFSET as usize;
        img[off + 4..off + 8].copy_from_slice(&2000u32.to_le_bytes());
        img[off + 8..off + 12].copy_from_slice(&1u32.to_le_bytes());
        assert_eq!(validate_app(&img, APP_BASE, APP_SIZE), Err(AppError::Length));
        img[off + 4..off + 8].copy_from_slice(&(APP_SIZE + 4).to_le_bytes());
        assert_eq!(validate_app(&img, APP_BASE, APP_SIZE), Err(AppError::Length));
        img[off + 4..off + 8].copy_from_slice(&998u32.to_le_bytes());
        assert_eq!(validate_app(&img, APP_BASE, APP_SIZE), Err(AppError::Length));
        // Wrong header version.
        let mut img = image(APP_BASE, 1000, "0.10.0");
        img[off + 12] = 2;
        assert_eq!(validate_app(&img, APP_BASE, APP_SIZE), Err(AppError::HeaderVersion));
        // Stamping needs the magic.
        let mut junk = vec![0u8; 0x200];
        assert_eq!(stamp_app(&mut junk, 0x200), Err(AppError::Magic));
    }

    #[test]
    fn variant_rule_rejects_the_other_pin_map() {
        let mut img = image(APP_BASE, 1000, "0.10.0");
        let off = HEADER_OFFSET as usize;
        img[off..off + 32].copy_from_slice(&app_header_bytes("0.10.0", FLAG_PROTO));
        // Unstamped proto image: boots on a proto bootloader only.
        assert!(matches!(
            validate_app_for(&img, APP_BASE, APP_SIZE, VARIANT_PROTO),
            Ok(Validity::Unstamped(_))
        ));
        assert_eq!(
            validate_app_for(&img, APP_BASE, APP_SIZE, VARIANT_PROD),
            Err(AppError::Variant)
        );
        // Stamped prod image: the other way round.
        let mut prod = image(APP_BASE, 1000, "0.10.0");
        let padded = (prod.len() + 3) & !3;
        prod.resize(padded, 0xFF);
        stamp_app(&mut prod, padded).unwrap();
        assert!(matches!(
            validate_app_for(&prod, APP_BASE, APP_SIZE, VARIANT_PROD),
            Ok(Validity::Stamped(_))
        ));
        assert_eq!(
            validate_app_for(&prod, APP_BASE, APP_SIZE, VARIANT_PROTO),
            Err(AppError::Variant)
        );
    }

    #[test]
    fn faults_word_round_trip() {
        assert_eq!(faults_count(0), 0);
        assert_eq!(faults_count(0xFFFF_FFFF), 0);
        assert_eq!(faults_count(faults_word(2)), 2);
        assert_eq!(faults_count(faults_word(300)), 300 & 0xFF);
    }
}
