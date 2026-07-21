//! Minimal DfuSe (ST extension of USB DFU 1.1, AN3156) client for the
//! STM32F072 ROM bootloader — just enough to erase, program, and leave.
//!
//! The ROM bootloader enumerates as 0483:df11 with one DFU interface whose
//! alt 0 is internal flash ("@Internal Flash /0x08000000/64*002Kg" on the
//! F072: 64 pages x 2 KiB). DfuSe commands ride on DFU_DNLOAD wBlockNum 0:
//! 0x41+addr = erase page, 0x21+addr = set address pointer; data blocks
//! start at wBlockNum 2 with target = addr_ptr + (n-2)*transfer_size.

use rusb::{Device, DeviceHandle, GlobalContext};
use std::time::Duration;

const DFU_VID: u16 = 0x0483;
const DFU_PID: u16 = 0xdf11;

const FLASH_BASE: u32 = 0x0800_0000;
const PAGE_SIZE: usize = 2048;

// DFU class requests.
const DFU_DNLOAD: u8 = 1;
const DFU_GETSTATUS: u8 = 3;
const DFU_CLRSTATUS: u8 = 4;

// DFU states (of interest).
const STATE_DFU_DNBUSY: u8 = 4;
const STATE_DFU_ERROR: u8 = 10;

const TIMEOUT: Duration = Duration::from_secs(3);

pub fn find_bootloader() -> Option<Device<GlobalContext>> {
    rusb::devices().ok()?.iter().find(|d| {
        d.device_descriptor()
            .map(|desc| desc.vendor_id() == DFU_VID && desc.product_id() == DFU_PID)
            .unwrap_or(false)
    })
}

struct Dfu {
    handle: DeviceHandle<GlobalContext>,
    transfer_size: usize,
}

impl Dfu {
    fn open(device: Device<GlobalContext>) -> Result<Self, String> {
        let transfer_size = read_transfer_size(&device).unwrap_or(2048);
        let handle = device.open().map_err(|e| format!("open: {e}"))?;
        // Linux may have a kernel driver attached; macOS/Windows return
        // NotSupported, which is fine.
        let _ = handle.set_auto_detach_kernel_driver(true);
        handle
            .claim_interface(0)
            .map_err(|e| format!("claim interface: {e}"))?;
        Ok(Dfu {
            handle,
            transfer_size,
        })
    }

    /// (bStatus, bwPollTimeout ms, bState)
    fn get_status(&self) -> Result<(u8, u64, u8), String> {
        let mut buf = [0u8; 6];
        let n = self
            .handle
            .read_control(0xA1, DFU_GETSTATUS, 0, 0, &mut buf, TIMEOUT)
            .map_err(|e| format!("GETSTATUS: {e}"))?;
        if n < 6 {
            return Err("short GETSTATUS reply".into());
        }
        let poll_ms = u64::from(buf[1]) | u64::from(buf[2]) << 8 | u64::from(buf[3]) << 16;
        Ok((buf[0], poll_ms, buf[4]))
    }

    fn clear_status(&self) -> Result<(), String> {
        self.handle
            .write_control(0x21, DFU_CLRSTATUS, 0, 0, &[], TIMEOUT)
            .map_err(|e| format!("CLRSTATUS: {e}"))?;
        Ok(())
    }

    fn dnload(&self, block: u16, data: &[u8]) -> Result<(), String> {
        self.handle
            .write_control(0x21, DFU_DNLOAD, block, 0, data, TIMEOUT)
            .map_err(|e| format!("DNLOAD: {e}"))?;
        Ok(())
    }

    /// DNLOAD then poll GETSTATUS through dfuDNBUSY until the device settles.
    fn dnload_sync(&self, block: u16, data: &[u8], what: &str) -> Result<(), String> {
        self.dnload(block, data)?;
        loop {
            let (status, poll_ms, state) = self.get_status()?;
            if state == STATE_DFU_ERROR || status != 0 {
                let _ = self.clear_status();
                return Err(format!(
                    "{what}: DFU error (status {status}, state {state})"
                ));
            }
            if state != STATE_DFU_DNBUSY {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(poll_ms.max(1)));
        }
    }

    fn erase_page(&self, addr: u32) -> Result<(), String> {
        let a = addr.to_le_bytes();
        self.dnload_sync(0, &[0x41, a[0], a[1], a[2], a[3]], "erase")
    }

    fn set_address(&self, addr: u32) -> Result<(), String> {
        let a = addr.to_le_bytes();
        self.dnload_sync(0, &[0x21, a[0], a[1], a[2], a[3]], "set address")
    }

    /// Zero-length DNLOAD -> manifest: the bootloader jumps to the address
    /// pointer. The device drops off the bus mid-handshake, so errors after
    /// the request itself are expected and ignored.
    fn leave(&self) -> Result<(), String> {
        self.dnload(0, &[])?;
        let _ = self.get_status();
        Ok(())
    }
}

/// wTransferSize from the DFU functional descriptor (type 0x21), found in the
/// interface's extra descriptor bytes.
fn read_transfer_size(device: &Device<GlobalContext>) -> Option<usize> {
    let config = device.config_descriptor(0).ok()?;
    for interface in config.interfaces() {
        for desc in interface.descriptors() {
            let extra = desc.extra();
            let mut i = 0;
            while i + 1 < extra.len() {
                let (len, dtype) = (extra[i] as usize, extra[i + 1]);
                if dtype == 0x21 && len >= 9 && i + 7 < extra.len() {
                    return Some(u16::from_le_bytes([extra[i + 5], extra[i + 6]]) as usize);
                }
                i += len.max(2);
            }
        }
    }
    None
}

/// Erase + program `image` at 0x08000000 and jump into it.
/// `progress(phase, fraction)` is called throughout; fraction spans the whole
/// operation (erase = first 30%, program = the rest).
pub fn flash(
    device: Device<GlobalContext>,
    image: &[u8],
    mut progress: impl FnMut(&str, f64),
) -> Result<(), String> {
    let dfu = Dfu::open(device)?;

    // A previous failed attempt can leave the state machine in dfuERROR.
    if let Ok((status, _, state)) = dfu.get_status() {
        if state == STATE_DFU_ERROR || status != 0 {
            dfu.clear_status()?;
        }
    }

    let pages = image.len().div_ceil(PAGE_SIZE);
    for p in 0..pages {
        progress(
            &format!("Erasing page {}/{pages}…", p + 1),
            0.30 * (p as f64 / pages as f64),
        );
        dfu.erase_page(FLASH_BASE + (p * PAGE_SIZE) as u32)?;
    }

    let xfer = dfu.transfer_size;
    let blocks = image.len().div_ceil(xfer);
    dfu.set_address(FLASH_BASE)?;
    for (i, chunk) in image.chunks(xfer).enumerate() {
        progress(
            &format!("Programming block {}/{blocks}…", i + 1),
            0.30 + 0.70 * (i as f64 / blocks as f64),
        );
        dfu.dnload_sync((i + 2) as u16, chunk, "program")?;
    }
    progress("Starting the new firmware…", 1.0);

    // Point the bootloader at the app and manifest out of DFU mode.
    dfu.set_address(FLASH_BASE)?;
    dfu.leave()
}
