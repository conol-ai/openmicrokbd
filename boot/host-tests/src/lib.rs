//! Compiles the bootloader's protocol state machine (`boot/src/update.rs`,
//! pure `core` over the `Programmer` trait) for the host and drives it
//! against a RAM model of the application slot with real flash semantics.
//! Run with:
//!
//! ```text
//! cd boot/host-tests && cargo test --offline --target $(rustc -vV | sed -n 's/^host: //p')
//! ```

#[path = "../../src/update.rs"]
pub mod update;

#[cfg(test)]
mod tests {
    use super::update::{outcome, Action, Outcome, Programmer, Updater};
    use openmicro_layout::{
        app_header_bytes, app_valid, image_crc, op, stamp_app, status, BootInfoReply, APP_BASE,
        APP_SIZE, BOOT_INFO_REPLY_EXT_LEN, BOOT_REPORT_LEN, FLAG_PROTO, HEADER_OFFSET, MAX_CHUNK,
        PAGE_SIZE, RAM_END, RESET_OFFSET, VARIANT_PROD, VARIANT_PROTO,
    };

    /// RAM model of the slot with the STM32F0 flash rules: erase sets 0xFF a
    /// page at a time; programming happens per half-word and is refused when
    /// the target is not erased (RM0091 only excepts writing 0x0000), which
    /// is stricter than "no 0→1 bit" and catches double writes. Offsets are
    /// slot-relative, like the real programmer, and bounds are enforced the
    /// same way. `fail_erase` / `fail_write` simulate a flash error;
    /// `stuck_zero` a range of cells that erase fine but read back 0 after
    /// being programmed, whatever was written.
    struct Ram {
        slot: Vec<u8>,
        erases: usize,
        writes: usize,
        fail_erase: bool,
        fail_write: bool,
        stuck_zero: Option<std::ops::Range<usize>>,
    }

    impl Ram {
        fn new() -> Self {
            Ram {
                slot: vec![0xFF; APP_SIZE as usize],
                erases: 0,
                writes: 0,
                fail_erase: false,
                fail_write: false,
                stuck_zero: None,
            }
        }

        /// A slot that already holds `image` (as if flashed earlier).
        fn holding(image: &[u8]) -> Self {
            let mut r = Ram::new();
            r.slot[..image.len()].copy_from_slice(image);
            r
        }
    }

    impl Programmer for Ram {
        fn erase(&mut self, from: u32, to: u32) -> Result<(), ()> {
            assert_eq!(from % PAGE_SIZE, 0, "erase start not page aligned");
            assert_eq!(to % PAGE_SIZE, 0, "erase end not page aligned");
            assert!(from < to && to <= APP_SIZE, "erase range {from:#x}..{to:#x}");
            if self.fail_erase {
                return Err(());
            }
            self.erases += 1;
            self.slot[from as usize..to as usize].fill(0xFF);
            Ok(())
        }

        fn write(&mut self, offset: u32, data: &[u8]) -> Result<(), ()> {
            assert!(!data.is_empty(), "empty write");
            assert_eq!(offset % 4, 0, "write offset {offset:#x} not word aligned");
            assert_eq!(data.len() % 4, 0, "write length {} not a word multiple", data.len());
            assert!(offset as usize + data.len() <= APP_SIZE as usize, "write past the slot");
            if self.fail_write {
                return Err(());
            }
            self.writes += 1;
            for (i, hw) in data.chunks(2).enumerate() {
                let at = offset as usize + i * 2;
                let erased = self.slot[at] == 0xFF && self.slot[at + 1] == 0xFF;
                let zero = hw == [0, 0];
                assert!(erased || zero, "programming a non-erased half-word at {at:#x}");
                self.slot[at..at + 2].copy_from_slice(hw);
                if self.stuck_zero.as_ref().is_some_and(|r| r.contains(&at)) {
                    self.slot[at..at + 2].fill(0);
                }
            }
            Ok(())
        }

        fn slot(&self) -> &[u8] {
            &self.slot
        }
    }

    /// A synthetic but structurally real application image: stack pointer,
    /// exact reset vector, unstamped header with `flags`, deterministic body.
    fn image_with_flags(len: usize, version: &str, flags: u16) -> Vec<u8> {
        let mut img = vec![0u8; len];
        img[0..4].copy_from_slice(&(RAM_END - 16).to_le_bytes());
        img[4..8].copy_from_slice(&((APP_BASE + RESET_OFFSET) | 1).to_le_bytes());
        img[HEADER_OFFSET as usize..RESET_OFFSET as usize]
            .copy_from_slice(&app_header_bytes(version, flags));
        for (i, b) in img[RESET_OFFSET as usize..].iter_mut().enumerate() {
            *b = (i * 7 + 3) as u8;
        }
        img
    }

    /// A production-variant (flags 0) unstamped image.
    fn image(len: usize, version: &str) -> Vec<u8> {
        image_with_flags(len, version, 0)
    }

    /// `image_with_flags`, padded to a word multiple and stamped like
    /// fw-image.py would; returns the image and its header crc.
    fn stamped_with_flags(len: usize, version: &str, flags: u16) -> (Vec<u8>, u32) {
        let mut img = image_with_flags(len, version, flags);
        let padded = (img.len() + 3) & !3;
        img.resize(padded, 0xFF);
        let h = stamp_app(&mut img, padded).unwrap();
        assert_eq!(h.crc, image_crc(&img));
        assert_ne!(h.crc, 0, "fixture: a stamped crc of 0 would read as unstamped");
        (img, h.crc)
    }

    /// A stamped production-variant image.
    fn stamped(len: usize, version: &str) -> (Vec<u8>, u32) {
        stamped_with_flags(len, version, 0)
    }

    /// A crc that is not 0 for BEGINs whose upload never reaches END.
    const SOME_CRC: u32 = 0x1234_5678;

    /// Request/reply helpers around one `Updater`.
    struct Dev {
        u: Updater<Ram>,
        rep: [u8; BOOT_REPORT_LEN],
    }

    impl Dev {
        fn new(ram: Ram) -> Self {
            Dev::with_variant(ram, VARIANT_PROD)
        }

        fn with_variant(ram: Ram, variant: u16) -> Self {
            Dev {
                u: Updater::new(ram, "1.0.0", variant),
                rep: [0; BOOT_REPORT_LEN],
            }
        }

        fn send(&mut self, req: &[u8]) -> Action {
            let a = self.u.handle(req, &mut self.rep);
            assert_eq!(self.rep[0], req.first().copied().unwrap_or(0), "opcode echo");
            assert_eq!(self.rep[0] & 0x80, 0, "bit 7 must stay clear");
            a
        }

        fn begin(&mut self, length: u32, crc: u32) -> u8 {
            let mut req = vec![op::UPDATE_BEGIN];
            req.extend_from_slice(&length.to_le_bytes());
            req.extend_from_slice(&crc.to_le_bytes());
            assert_eq!(self.send(&req), Action::Reply(2));
            self.rep[1]
        }

        fn data(&mut self, offset: u32, chunk: &[u8]) -> (u8, u32) {
            let mut req = vec![op::UPDATE_DATA];
            req.extend_from_slice(&offset.to_le_bytes());
            req.push(chunk.len() as u8);
            req.extend_from_slice(chunk);
            assert_eq!(self.send(&req), Action::Reply(6));
            (self.rep[1], u32::from_le_bytes(self.rep[2..6].try_into().unwrap()))
        }

        fn end(&mut self) -> u8 {
            assert_eq!(self.send(&[op::UPDATE_END]), Action::Reply(2));
            self.rep[1]
        }

        fn info(&mut self) -> BootInfoReply {
            assert_eq!(self.send(&[op::BOOT_INFO]), Action::Reply(BOOT_INFO_REPLY_EXT_LEN));
            BootInfoReply::parse(&self.rep[..BOOT_INFO_REPLY_EXT_LEN]).unwrap()
        }

        /// BEGIN + all DATA chunks of `chunk` bytes; returns the DATA
        /// statuses so a test can check every step was OK.
        fn upload(&mut self, img: &[u8], crc: u32, chunk: usize) -> Vec<u8> {
            assert_eq!(self.begin(img.len() as u32, crc), status::OK);
            let mut statuses = Vec::new();
            for (i, c) in img.chunks(chunk).enumerate() {
                let (st, next) = self.data((i * chunk) as u32, c);
                statuses.push(st);
                if st == status::OK {
                    assert_eq!(next as usize, i * chunk + c.len());
                }
            }
            statuses
        }
    }

    #[test]
    fn version_reply_names_the_bootloader() {
        let mut d = Dev::new(Ram::new());
        assert_eq!(d.send(&[op::VERSION]), Action::Reply(2 + "boot 1.0.0".len()));
        let len = d.rep[1] as usize;
        assert_eq!(&d.rep[2..2 + len], b"boot 1.0.0");
        assert_eq!(outcome(&d.rep), Outcome::Quiet);
    }

    #[test]
    fn boot_info_reports_an_empty_slot() {
        let mut d = Dev::new(Ram::new());
        assert!(!d.u.take_info_answered());
        let info = d.info();
        assert!(d.u.take_info_answered());
        assert!(!d.u.take_info_answered(), "reported once");
        assert_eq!(info.status, status::OK);
        assert_eq!(info.protocol, 1);
        assert_eq!(info.boot_version, [1, 0, 0]);
        assert_eq!(info.app_base, APP_BASE);
        assert_eq!(info.app_size, APP_SIZE);
        assert_eq!(info.app_valid, app_valid::NONE);
        assert_eq!(info.app_version, [0; 16]);
        assert_eq!(info.page, Some(PAGE_SIZE as u16));
        assert_eq!(info.max_chunk, Some(MAX_CHUNK as u8));
    }

    #[test]
    fn boot_info_reports_a_stamped_and_an_unstamped_image() {
        let (img, _) = stamped(3000, "0.10.0");
        let mut d = Dev::new(Ram::holding(&img));
        let info = d.info();
        assert_eq!(info.app_valid, app_valid::VALID);
        assert_eq!(info.app_version_str(), "0.10.0");

        // What `cargo run` leaves behind: magic ok, length/crc zero.
        let mut d = Dev::new(Ram::holding(&image(3000, "0.10.0-dev")));
        let info = d.info();
        assert_eq!(info.app_valid, app_valid::UNSTAMPED);
        assert_eq!(info.app_version_str(), "0.10.0-dev");
    }

    #[test]
    fn happy_path_multi_chunk_with_a_short_last_chunk() {
        // Three and a half pages: 127 full chunks of 56 and a final 52-byte one.
        let (img, crc) = stamped(127 * MAX_CHUNK + 52, "0.10.0");
        assert_eq!(img.len() % MAX_CHUNK, 52);
        assert_eq!(img.len() % 4, 0);
        let mut d = Dev::new(Ram::new());
        let statuses = d.upload(&img, crc, MAX_CHUNK);
        assert!(statuses.iter().all(|&s| s == status::OK), "{statuses:?}");
        assert_eq!(d.u.programmer().erases, 1);
        assert_eq!(d.end(), status::OK);
        assert_eq!(outcome(&d.rep), Outcome::UpdateDone);
        assert_eq!(&d.u.programmer().slot()[..img.len()], &img[..]);
        // The rest of the erased pages stayed 0xFF; nothing beyond them was touched.
        let pages = (img.len() as u32).div_ceil(PAGE_SIZE) * PAGE_SIZE;
        assert!(d.u.programmer().slot()[img.len()..pages as usize].iter().all(|&b| b == 0xFF));

        let info = d.info();
        assert_eq!(info.app_valid, app_valid::VALID);
        assert_eq!(info.app_version_str(), "0.10.0");
        // END twice: the state was consumed.
        assert_eq!(d.end(), status::NO_BEGIN);
    }

    #[test]
    fn largest_image_fills_the_slot_exactly() {
        let (img, crc) = stamped(APP_SIZE as usize, "0.10.0");
        let mut d = Dev::new(Ram::new());
        let statuses = d.upload(&img, crc, MAX_CHUNK);
        assert!(statuses.iter().all(|&s| s == status::OK));
        assert_eq!(d.end(), status::OK);
        assert_eq!(d.info().app_valid, app_valid::VALID);
    }

    #[test]
    fn out_of_order_chunk_reports_the_resume_offset() {
        let (img, crc) = stamped(1000, "0.10.0");
        let mut d = Dev::new(Ram::new());
        assert_eq!(d.begin(img.len() as u32, crc), status::OK);
        assert_eq!(d.data(0, &img[..56]), (status::OK, 56));
        // A lost reply makes the host resend the same chunk.
        assert_eq!(d.data(0, &img[..56]), (status::OUT_OF_ORDER, 56));
        assert_eq!(outcome(&d.rep), Outcome::Error);
        // Skipping ahead is refused the same way.
        assert_eq!(d.data(112, &img[112..168]), (status::OUT_OF_ORDER, 56));
        // Resuming from the reported offset works and the image verifies.
        for (i, c) in img[56..].chunks(56).enumerate() {
            let off = 56 + i * 56;
            assert_eq!(d.data(off as u32, c), (status::OK, (off + c.len()) as u32));
        }
        assert_eq!(d.end(), status::OK);
        assert_eq!(d.u.programmer().writes, img.len().div_ceil(56));
    }

    #[test]
    fn data_before_begin_is_refused() {
        let mut d = Dev::new(Ram::new());
        assert_eq!(d.data(0, &[1, 2, 3, 4]), (status::NO_BEGIN, 0));
        assert_eq!(d.end(), status::NO_BEGIN);
        assert!(d.u.programmer().slot().iter().all(|&b| b == 0xFF));
        assert_eq!(d.u.programmer().writes, 0);
    }

    #[test]
    fn begin_rejects_bad_lengths_without_erasing() {
        let mut d = Dev::new(Ram::holding(&stamped(1000, "0.9.9").0));
        assert_eq!(d.begin(APP_SIZE + 4, 0), status::BAD_LENGTH);
        assert_eq!(d.begin(1001, 0), status::BAD_LENGTH);
        assert_eq!(d.begin(1002, 0), status::BAD_LENGTH);
        assert_eq!(d.begin(RESET_OFFSET - 4, 0), status::BAD_LENGTH);
        assert_eq!(d.begin(0, 0), status::BAD_LENGTH);
        assert_eq!(outcome(&d.rep), Outcome::Error);
        // Truncated request.
        assert_eq!(d.send(&[op::UPDATE_BEGIN, 1, 2, 3]), Action::Reply(2));
        assert_eq!(d.rep[1], status::BAD_LENGTH);
        assert_eq!(d.u.programmer().erases, 0);
        // The old image is untouched and still valid; no upload is open.
        assert_eq!(d.info().app_valid, app_valid::VALID);
        assert_eq!(d.data(0, &[0; 4]), (status::NO_BEGIN, 0));
        // The smallest and largest legal lengths are accepted.
        assert_eq!(d.begin(RESET_OFFSET, SOME_CRC), status::OK);
        assert_eq!(d.begin(APP_SIZE, SOME_CRC), status::OK);
    }

    #[test]
    fn begin_refuses_a_zero_crc_without_erasing() {
        let (old, _) = stamped(1000, "0.9.9");
        let mut d = Dev::new(Ram::holding(&old));
        // crc 0 is the unstamped signature: not an upload, not an erase.
        assert_eq!(d.begin(1000, 0), status::HEADER);
        assert_eq!(outcome(&d.rep), Outcome::Error);
        assert_eq!(d.begin(RESET_OFFSET, 0), status::HEADER);
        assert_eq!(d.begin(APP_SIZE, 0), status::HEADER);
        assert_eq!(d.u.programmer().erases, 0);
        assert_eq!(&d.u.programmer().slot()[..old.len()], &old[..]);
        assert_eq!(d.info().app_valid, app_valid::VALID);
        // No upload is open.
        assert_eq!(d.data(0, &old[..56]), (status::NO_BEGIN, 0));
        assert_eq!(d.end(), status::NO_BEGIN);
        // The length checks still come first.
        assert_eq!(d.begin(1001, 0), status::BAD_LENGTH);
    }

    #[test]
    fn begin_erases_exactly_the_pages_the_image_needs() {
        let (old, _) = stamped(4 * PAGE_SIZE as usize, "0.9.9");
        let mut d = Dev::new(Ram::holding(&old));
        // A 1-page image erases page 0 only; pages 1..4 keep the old bytes.
        assert_eq!(d.begin(PAGE_SIZE, SOME_CRC), status::OK);
        let slot = d.u.programmer().slot();
        assert!(slot[..PAGE_SIZE as usize].iter().all(|&b| b == 0xFF));
        assert_eq!(&slot[PAGE_SIZE as usize..old.len()], &old[PAGE_SIZE as usize..]);
        // Page 0 is gone, so the slot no longer validates.
        assert_eq!(d.info().app_valid, app_valid::NONE);
        // A length just over a page boundary erases two pages.
        assert_eq!(d.begin(PAGE_SIZE + 4, SOME_CRC), status::OK);
        assert!(d.u.programmer().slot()[..2 * PAGE_SIZE as usize].iter().all(|&b| b == 0xFF));
    }

    #[test]
    fn data_validates_the_chunk_shape() {
        let (img, crc) = stamped(1000, "0.10.0");
        let mut d = Dev::new(Ram::new());
        assert_eq!(d.begin(img.len() as u32, crc), status::OK);
        // Too long.
        assert_eq!(d.data(0, &img[..MAX_CHUNK + 4]).0, status::BAD_LENGTH);
        // Empty.
        assert_eq!(d.data(0, &[]).0, status::BAD_LENGTH);
        // Not a word multiple and not the final chunk.
        assert_eq!(d.data(0, &img[..6]).0, status::BAD_LENGTH);
        // Beyond the announced length.
        assert_eq!(d.data(0, &[0; 56]).0, status::OK);
        let mut req = vec![op::UPDATE_DATA];
        req.extend_from_slice(&56u32.to_le_bytes());
        req.push(8);
        req.extend_from_slice(&[0; 4]); // claims 8 bytes, carries 4
        assert_eq!(d.send(&req), Action::Reply(6));
        assert_eq!(d.rep[1], status::BAD_LENGTH);
        assert_eq!(d.data(56, &[0; 56]).0, status::OK);
        // Correct offset but the chunk would run past `length`.
        let mut d2 = Dev::new(Ram::new());
        assert_eq!(d2.begin(img.len() as u32, crc), status::OK);
        let full = img.len() / 56;
        for i in 0..full {
            assert_eq!(d2.data((i * 56) as u32, &img[i * 56..(i + 1) * 56]).0, status::OK);
        }
        let at = (full * 56) as u32;
        assert!(img.len() as u32 - at < 56, "fixture: the tail must be shorter than a chunk");
        assert_eq!(d2.data(at, &[0; 56]).0, status::BAD_LENGTH);
        // Errors keep the state: the right (short) tail still goes through
        // and the image verifies.
        assert_eq!(d2.data(at, &img[at as usize..]).0, status::OK);
        assert_eq!(d2.end(), status::OK);
    }

    #[test]
    fn crc_mismatch_is_reported_and_the_slot_stays_invalid() {
        let (mut img, crc) = stamped(2500, "0.10.0");
        img[700] ^= 0x01;
        let mut d = Dev::new(Ram::new());
        d.upload(&img, crc, MAX_CHUNK);
        assert_eq!(d.end(), status::CRC);
        assert_eq!(outcome(&d.rep), Outcome::Error);
        assert_eq!(d.info().app_valid, app_valid::NONE);
        // The bytes are what was sent; only the verdict differs.
        assert_eq!(&d.u.programmer().slot()[..img.len()], &img[..]);
    }

    /// The index of the 56-byte chunk that carries the header's length and
    /// crc words (slot offsets 0xC4..0xCC): 168..224.
    const HEADER_CHUNK: usize = (HEADER_OFFSET as usize + 4) / MAX_CHUNK;

    #[test]
    fn begin_crc_must_match_the_header() {
        let (img, crc) = stamped(1000, "0.10.0");
        let mut d = Dev::new(Ram::new());
        let statuses = d.upload(&img, crc ^ 0xDEAD_BEEF, MAX_CHUNK);
        // The image itself verifies, but it is not the one the host
        // announced: refused as soon as the header chunk shows up, before
        // it is programmed; every later chunk finds the upload closed.
        assert_eq!(&statuses[..HEADER_CHUNK], &[status::OK; HEADER_CHUNK]);
        assert_eq!(statuses[HEADER_CHUNK], status::HEADER);
        assert!(statuses[HEADER_CHUNK + 1..].iter().all(|&s| s == status::NO_BEGIN));
        assert_eq!(d.end(), status::NO_BEGIN);
        // The slot never got its magic, so it cannot look bootable.
        assert!(d.u.programmer().slot()[HEADER_OFFSET as usize..].iter().all(|&b| b == 0xFF));
        assert_eq!(d.info().app_valid, app_valid::NONE);
        // Uploading again with the right crc succeeds.
        let statuses = d.upload(&img, crc, MAX_CHUNK);
        assert!(statuses.iter().all(|&s| s == status::OK));
        assert_eq!(d.end(), status::OK);
    }

    #[test]
    fn data_refuses_a_header_that_disagrees_with_begin() {
        let (img, crc) = stamped(1000, "0.10.0");

        // Length announced in BEGIN differs from the header's.
        let mut d = Dev::new(Ram::new());
        assert_eq!(d.begin(img.len() as u32 + 8, crc), status::OK);
        for i in 0..HEADER_CHUNK {
            let (st, _) = d.data((i * MAX_CHUNK) as u32, &img[i * MAX_CHUNK..(i + 1) * MAX_CHUNK]);
            assert_eq!(st, status::OK);
        }
        let at = HEADER_CHUNK * MAX_CHUNK;
        assert_eq!(d.data(at as u32, &img[at..at + MAX_CHUNK]), (status::HEADER, 0));
        assert_eq!(outcome(&d.rep), Outcome::Error);
        let next = at + MAX_CHUNK;
        assert_eq!(d.data(next as u32, &img[next..next + MAX_CHUNK]).0, status::NO_BEGIN);
        assert_eq!(d.end(), status::NO_BEGIN);
        assert_eq!(d.info().app_valid, app_valid::NONE);
        let slot = d.u.programmer().slot();
        assert!(slot[HEADER_OFFSET as usize..].iter().all(|&b| b == 0xFF));

        // An unstamped image (length and crc both 0 in the header) under a
        // BEGIN that claims a crc: what `cargo run` produces, sent by a
        // hand-rolled client. Refused at the header chunk, never bootable.
        let raw = image(1000, "0.10.0-dev");
        let mut d = Dev::new(Ram::new());
        let statuses = d.upload(&raw, SOME_CRC, MAX_CHUNK);
        assert_eq!(statuses[HEADER_CHUNK], status::HEADER);
        assert!(statuses[HEADER_CHUNK + 1..].iter().all(|&s| s == status::NO_BEGIN));
        assert_eq!(d.end(), status::NO_BEGIN);
        assert_eq!(d.info().app_valid, app_valid::NONE);

        // Word-sized chunks put the length and crc words in different
        // chunks: the length word passes, the crc word is what trips.
        let mut d = Dev::new(Ram::new());
        assert_eq!(d.begin(img.len() as u32, crc ^ 1), status::OK);
        let crc_word = (HEADER_OFFSET + 8) as usize;
        for off in (0..crc_word).step_by(4) {
            assert_eq!(d.data(off as u32, &img[off..off + 4]), (status::OK, off as u32 + 4));
        }
        assert_eq!(d.data(crc_word as u32, &img[crc_word..crc_word + 4]), (status::HEADER, 0));
        // The magic and length words were programmed but the crc word was
        // not: still 0xFF there, so validation fails on the CRC and the
        // slot is not bootable.
        let slot = d.u.programmer().slot();
        assert_eq!(&slot[crc_word..crc_word + 4], &[0xFF; 4]);
        assert_eq!(d.info().app_valid, app_valid::NONE);

        // The same words at the right values pass in word-sized chunks too.
        let mut d = Dev::new(Ram::new());
        let statuses = d.upload(&img, crc, 4);
        assert!(statuses.iter().all(|&s| s == status::OK));
        assert_eq!(d.end(), status::OK);
    }

    #[test]
    fn end_refuses_a_slot_that_reads_back_unstamped() {
        // Cells that take the erase but not the program: the length and crc
        // words read back as zeros, i.e. the unstamped signature the boot
        // path would accept on the vectors alone. END must not.
        let (img, crc) = stamped(1000, "0.10.0");
        let mut ram = Ram::new();
        ram.stuck_zero = Some(HEADER_OFFSET as usize + 4..HEADER_OFFSET as usize + 12);
        let mut d = Dev::new(ram);
        let statuses = d.upload(&img, crc, MAX_CHUNK);
        assert!(statuses.iter().all(|&s| s == status::OK), "{statuses:?}");
        let slot = d.u.programmer().slot();
        assert_eq!(&slot[HEADER_OFFSET as usize + 4..HEADER_OFFSET as usize + 12], &[0; 8]);
        assert_eq!(d.end(), status::HEADER);
        assert_eq!(outcome(&d.rep), Outcome::Error);
        assert_eq!(d.end(), status::NO_BEGIN);
    }

    #[test]
    fn variant_flag_must_match_the_bootloader() {
        let (prod, prod_crc) = stamped(1000, "0.10.0");
        let (proto, proto_crc) = stamped_with_flags(1000, "0.10.0", FLAG_PROTO);
        assert_ne!(prod_crc, proto_crc);

        // A proto-flagged image on a production bootloader is refused at END.
        let mut d = Dev::with_variant(Ram::new(), VARIANT_PROD);
        let statuses = d.upload(&proto, proto_crc, MAX_CHUNK);
        assert!(statuses.iter().all(|&s| s == status::OK));
        assert_eq!(d.end(), status::HEADER);
        assert_eq!(outcome(&d.rep), Outcome::Error);
        assert_eq!(d.end(), status::NO_BEGIN);
        // The refused image is complete and CRC-valid in the slot, but the
        // variant rule is part of slot validation: BOOT_INFO (and the boot
        // path, which uses the same function) report no valid application.
        assert_eq!(d.info().app_valid, app_valid::NONE);
        // A matching one is accepted.
        d.upload(&prod, prod_crc, MAX_CHUNK);
        assert_eq!(d.end(), status::OK);
        assert_eq!(d.info().app_valid, app_valid::VALID);

        // And the other way round on a proto bootloader.
        let mut d = Dev::with_variant(Ram::new(), VARIANT_PROTO);
        d.upload(&prod, prod_crc, MAX_CHUNK);
        assert_eq!(d.end(), status::HEADER);
        d.upload(&proto, proto_crc, MAX_CHUNK);
        assert_eq!(d.end(), status::OK);
        assert_eq!(d.info().app_version_str(), "0.10.0");
    }

    #[test]
    fn garbage_upload_fails_the_header_check() {
        let junk: Vec<u8> = (0..1000u32).map(|i| (i * 31 + 7) as u8).collect();
        let mut d = Dev::new(Ram::new());
        // Junk has no header: the chunk where the length/crc words should
        // be is refused before the body is even complete.
        let statuses = d.upload(&junk, SOME_CRC, MAX_CHUNK);
        assert_eq!(statuses[HEADER_CHUNK], status::HEADER);
        assert_eq!(d.end(), status::NO_BEGIN);
        assert_eq!(d.info().app_valid, app_valid::NONE);
        // An image linked for another base carries a consistent stamp, so
        // it gets through DATA and fails validation at END: a header error.
        let mut moved = image(1000, "0.10.0");
        moved[4..8].copy_from_slice(&((APP_BASE + 0x1000 + RESET_OFFSET) | 1).to_le_bytes());
        let padded = moved.len();
        stamp_app(&mut moved, padded).unwrap();
        let crc = image_crc(&moved);
        d.upload(&moved, crc, MAX_CHUNK);
        assert_eq!(d.end(), status::HEADER);
    }

    #[test]
    fn begin_twice_restarts_the_upload() {
        let (img, crc) = stamped(3000, "0.10.0");
        let mut d = Dev::new(Ram::new());
        assert_eq!(d.begin(img.len() as u32, crc), status::OK);
        assert_eq!(d.data(0, &img[..56]).0, status::OK);
        assert_eq!(d.data(56, &img[56..112]).0, status::OK);
        // Start over: erased again, offset back to 0.
        assert_eq!(d.begin(img.len() as u32, crc), status::OK);
        assert_eq!(d.u.programmer().erases, 2);
        assert!(d.u.programmer().slot()[..112].iter().all(|&b| b == 0xFF));
        assert_eq!(d.data(56, &img[56..112]), (status::OUT_OF_ORDER, 0));
        let statuses = d.upload(&img, crc, 40);
        assert!(statuses.iter().all(|&s| s == status::OK));
        assert_eq!(d.end(), status::OK);
    }

    #[test]
    fn end_before_all_data_keeps_the_upload_open() {
        let (img, crc) = stamped(1000, "0.10.0");
        let mut d = Dev::new(Ram::new());
        assert_eq!(d.begin(img.len() as u32, crc), status::OK);
        assert_eq!(d.data(0, &img[..56]).0, status::OK);
        assert_eq!(d.end(), status::OUT_OF_ORDER);
        assert_eq!(outcome(&d.rep), Outcome::Error);
        // Still open at the same offset; finishing works.
        assert_eq!(d.data(0, &img[..56]), (status::OUT_OF_ORDER, 56));
        for (i, c) in img[56..].chunks(56).enumerate() {
            assert_eq!(d.data((56 + i * 56) as u32, c).0, status::OK);
        }
        assert_eq!(d.end(), status::OK);
    }

    #[test]
    fn flash_errors_are_reported_and_close_the_upload() {
        let (img, crc) = stamped(1000, "0.10.0");
        let mut ram = Ram::new();
        ram.fail_erase = true;
        let mut d = Dev::new(ram);
        assert_eq!(d.begin(img.len() as u32, crc), status::FLASH);
        assert_eq!(d.data(0, &img[..56]).0, status::NO_BEGIN);

        let mut ram = Ram::new();
        ram.fail_write = true;
        let mut d = Dev::new(ram);
        assert_eq!(d.begin(img.len() as u32, crc), status::OK);
        assert_eq!(d.data(0, &img[..56]).0, status::FLASH);
        assert_eq!(outcome(&d.rep), Outcome::Error);
        assert_eq!(d.data(56, &img[56..112]).0, status::NO_BEGIN);
        assert_eq!(d.end(), status::NO_BEGIN);
    }

    #[test]
    fn boot_run_resets_into_the_app() {
        let mut d = Dev::new(Ram::new());
        assert_eq!(d.send(&[op::BOOT_RUN]), Action::ResetToApp);
        assert_eq!(d.rep[1], status::OK);
        assert_eq!(outcome(&d.rep), Outcome::Quiet);
    }

    #[test]
    fn enter_dfu_needs_the_key() {
        let mut d = Dev::new(Ram::new());
        assert_eq!(d.send(&[op::ENTER_DFU, b'D', b'F', b'U', b'!']), Action::ResetToDfu);
        assert_eq!(d.rep[1], 1);
        assert_eq!(outcome(&d.rep), Outcome::Quiet);
        assert_eq!(d.send(&[op::ENTER_DFU, b'D', b'F', b'U', b'?']), Action::Reply(2));
        assert_eq!(d.rep[1], 0);
        assert_eq!(outcome(&d.rep), Outcome::Error);
        assert_eq!(d.send(&[op::ENTER_DFU]), Action::Reply(2));
        assert_eq!(d.rep[1], 0);
    }

    #[test]
    fn unknown_opcodes_and_empty_reports_get_a_refusal() {
        let mut d = Dev::new(Ram::new());
        assert_eq!(d.send(&[0x7E, 1, 2, 3]), Action::Reply(2));
        assert_eq!(d.rep[1], status::UNKNOWN);
        assert_eq!(outcome(&d.rep), Outcome::Error);
        // The application's own opcodes are not ours either.
        assert_eq!(d.send(&[0x03, 0]), Action::Reply(2));
        assert_eq!(d.rep[1], status::UNKNOWN);
        assert_eq!(d.send(&[]), Action::Reply(2));
        assert_eq!(d.rep[..2], [0, status::UNKNOWN]);
        assert_eq!(outcome(&[]), Outcome::Error);
        assert_eq!(outcome(&[op::VERSION]), Outcome::Error);
    }

    #[test]
    fn replies_are_zero_padded_reports() {
        let mut d = Dev::new(Ram::new());
        d.rep = [0xAA; BOOT_REPORT_LEN];
        assert_eq!(d.send(&[op::BOOT_RUN]), Action::ResetToApp);
        assert!(d.rep[2..].iter().all(|&b| b == 0));
    }
}
