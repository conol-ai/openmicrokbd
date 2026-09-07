//! OpenMicro v1 resident bootloader (STM32F072CB, 24 KiB at 0x08000000).
//!
//! The one job: the pad must never end up executing garbage. The
//! bootloader is the only thing that writes the application slot, checks
//! the application's header and CRC-32 before every start, and stays in a
//! driverless USB HID update mode whenever the application is missing,
//! invalid, asked for it, or the encoder switch is held at power-up.
//!
//! `main` is a plain `#[cortex_m_rt::entry]`: the boot decision runs first,
//! on the reset-default 8 MHz HSI, with no executor, clocks, USB or NVIC
//! state touched, so a valid application starts from reset defaults. Order:
//!
//! 1. hygiene: SysTick off, pending SysTick/PendSV cleared, every NVIC
//!    line disabled and unpended — and only then SYSCFG on and MEM_MODE =
//!    main flash (the ROM DFU `leave` path arrives with system memory
//!    mapped at 0), sysclk back on HSI, PRIMASK clear;
//! 2. read and clear the handoff request (layout: REQUEST / !REQUEST);
//! 3. REQ_ROM_DFU → jump into the ST ROM DFU (0483:DF11);
//! 4. REQ_BOOTLOADER → update mode; REQ_RUN → straight on to step 5;
//!    otherwise a fault counted on the previous boot → update mode, and
//!    sampling the encoder switch for 20 ms → held → update mode;
//! 5. validate the slot (`openmicro_layout::validate_app_for`: header, CRC and board variant);
//! 6. valid → copy its vector table to 0x20000000, remap SRAM to address 0
//!    (no VTOR on Cortex-M0), `bootload`; invalid → update mode.
//!
//! Update mode brings the clocks to 48 MHz, enumerates as 1209:0002
//! "OpenMicro Bootloader" with one 64-byte raw-HID interface and runs the
//! protocol in `update.rs` over the flash (`flashprog.rs`). It never times
//! out. Panics, HardFaults and every otherwise unbound exception escalate
//! through the FAULTS handoff word: reset once (back into update mode when
//! the fault happened there), then reset into ROM DFU, then halt
//! SWD-attachable. Only a running application or a talking update mode
//! clears the word.

#![no_std]
#![no_main]

mod flashprog;
mod handoff;
mod update;
mod ws2812;

use core::sync::atomic::{AtomicU32, Ordering};

use cortex_m::asm;
use cortex_m::peripheral::{NVIC, SCB, SYST};
use embassy_executor::Executor;
use embassy_futures::join::join;
use embassy_futures::select::{select, Either};
use embassy_stm32::flash::Flash;
use embassy_stm32::gpio::{Level, Output, Speed};
use embassy_stm32::pac;
use embassy_stm32::pac::gpio::vals::{Idr, Moder, Pupdr};
use embassy_stm32::pac::rcc::vals::Sw;
use embassy_stm32::pac::syscfg::vals::MemMode;
use embassy_stm32::rcc::{Hsi48Config, Sysclk};
use embassy_stm32::usb::Driver;
use embassy_stm32::{bind_interrupts, peripherals, usb, Config, Peripherals};
use embassy_time::{Duration, Instant, Timer};
use embassy_usb::class::hid::{HidReader, HidReaderWriter, HidWriter, State};
use embassy_usb::driver::Driver as UsbDriver;
use openmicro_layout::{
    boot_info_bytes, faults_count, faults_word, validate_app_for, APP_BASE, APP_SIZE,
    BOOT_MANUFACTURER, BOOT_PID, BOOT_PRODUCT, BOOT_REPORT_LEN, BOOT_VID, RAM_BASE,
    REASON_AFTER_UPDATE, REASON_NORMAL, REASON_SWITCH, REQ_BOOTLOADER, REQ_ROM_DFU, REQ_RUN,
    VARIANT_PROD, VARIANT_PROTO, VECTORS_LEN,
};
use static_cell::StaticCell;

use update::{outcome, Action, Outcome, Programmer, Updater};
use ws2812::Grb;

const BOOT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Recorded in the info block; build-firmware.sh asserts it matches the
/// application's.
const VARIANT: u16 = if cfg!(feature = "proto") {
    VARIANT_PROTO
} else {
    VARIANT_PROD
};

/// The info block at 0x080000C0 (memory.x pins the section right behind
/// the vector table). The application answers BOOT_INFO from it and the
/// host slices a combined image at `app_base`, so nothing else hard-codes
/// the layout.
#[link_section = ".boot_info"]
#[used]
#[no_mangle]
pub static BOOT_INFO: [u8; 32] = boot_info_bytes(BOOT_VERSION, VARIANT);

/// F07x system-memory bootloader base (AN2606).
const SYSTEM_MEMORY: u32 = 0x1FFF_C800;

// ---- board (see the pin-map tables in fw/src/main.rs) ---------------------

#[cfg(not(feature = "proto"))]
mod board {
    use embassy_stm32::pac;
    /// ENC_SW: internal pull-up, the switch shorts it to GND.
    pub const SW_PORT: pac::gpio::Gpio = pac::GPIOB;
    pub const SW_PIN: usize = 15;
    pub fn sw_clock(on: bool) {
        pac::RCC.ahbenr().modify(|w| w.set_gpioben(on));
    }
    /// LED_KEY: the 13-pixel per-key chain.
    pub const KEY_PORT: pac::gpio::Gpio = pac::GPIOA;
    pub const KEY_PIN: usize = 8;
    /// LED_UG: the underglow ring.
    pub const UG_PORT: pac::gpio::Gpio = pac::GPIOB;
    pub const UG_PIN: usize = 14;
    pub const UG_LEN: usize = 8;
}

#[cfg(feature = "proto")]
mod board {
    use embassy_stm32::pac;
    pub const SW_PORT: pac::gpio::Gpio = pac::GPIOC;
    pub const SW_PIN: usize = 15;
    pub fn sw_clock(on: bool) {
        pac::RCC.ahbenr().modify(|w| w.set_gpiocen(on));
    }
    pub const KEY_PORT: pac::gpio::Gpio = pac::GPIOB;
    pub const KEY_PIN: usize = 4;
    pub const UG_PORT: pac::gpio::Gpio = pac::GPIOA;
    pub const UG_PIN: usize = 0;
    pub const UG_LEN: usize = 16;
}

const KEY_LEN: usize = 13;

// ---- boot decision ----------------------------------------------------------

#[cortex_m_rt::entry]
fn main() -> ! {
    hygiene();

    let request = handoff::take_request();
    match request {
        Some(REQ_ROM_DFU) => rom_dfu(),
        Some(REQ_BOOTLOADER) => update_mode(REASON_NORMAL),
        // BOOT_RUN just happened: the user is not holding the switch for us,
        // and update mode was demonstrably talking, so a stale fault count
        // is not a reason to go back.
        Some(REQ_RUN) => {}
        _ => {
            // The previous boot faulted and reset without asking for
            // anything. Either the application died in its init window (its
            // faults vector through our table until it re-remaps, and only
            // a running application clears the counter, fw/src/boot.rs) or
            // our own entry path did. Jumping again would just repeat it;
            // update mode lets the host replace the image instead. A power
            // cycle clears the word (SRAM) and tries the application again.
            if faults_count(handoff::faults()) >= 1 {
                update_mode(REASON_NORMAL);
            }
            if switch_held() {
                update_mode(REASON_SWITCH);
            }
        }
    }

    match validate_app_for(app_slot(), APP_BASE, APP_SIZE, VARIANT) {
        Ok(_) => {
            // Update mode leaves REASON_AFTER_UPDATE once an upload verified
            // and REASON_SWITCH when the switch brought it up; either is
            // carried through only if this boot really came from its
            // BOOT_RUN (a valid REQ_RUN), never from power-up garbage.
            let reason = match handoff::reason() {
                r @ (REASON_AFTER_UPDATE | REASON_SWITCH) if request == Some(REQ_RUN) => r,
                _ => REASON_NORMAL,
            };
            jump_to_app(reason)
        }
        Err(_) => update_mode(REASON_NORMAL),
    }
}

/// SCB ICSR write-one-to-clear bits for a pending SysTick / PendSV.
const ICSR_PENDSTCLR: u32 = 1 << 25;
const ICSR_PENDSVCLR: u32 = 1 << 27;

/// Bring the core back to the state a fresh reset would have left, whatever
/// path led here (the ROM DFU `leave` jumps with system memory at address
/// 0, its clocks and possibly SysTick running and PRIMASK set; a debugger
/// may have poked around). Nothing here touches PA13/PA14.
///
/// Order matters: every inherited exception source is silenced *before*
/// the vector table switches to ours, so nothing can be taken through it in
/// the few instructions between the remap and the NVIC writes (a SysTick
/// already pending in ICSR would otherwise fire right after the remap and
/// land in an exception handler with the core half-configured).
fn hygiene() {
    // SAFETY: plain register writes on the core's own SysTick/SCB/NVIC.
    unsafe {
        // SysTick stopped: CSR = 0 clears ENABLE and TICKINT.
        (*SYST::PTR).csr.write(0);
        // A tick or PendSV that was already pending survives the CSR
        // write; ICSR has clear-pending bits for both.
        (*SCB::PTR).icsr.write(ICSR_PENDSTCLR | ICSR_PENDSVCLR);
        // No interrupt enabled or pending (the F072 has 32 lines, all in
        // word 0).
        let nvic = &*NVIC::PTR;
        nvic.icer[0].write(u32::MAX);
        nvic.icpr[0].write(u32::MAX);
    }
    asm::dsb();
    asm::isb();

    // Vectors from main flash: our own table at 0x08000000.
    pac::RCC.apb2enr().modify(|w| w.set_syscfgen(true));
    let _ = pac::RCC.apb2enr().read();
    pac::SYSCFG
        .cfgr1()
        .modify(|w| w.set_mem_mode(MemMode::MAIN_FLASH));
    asm::dsb();
    asm::isb();

    // Sysclk on the 8 MHz HSI: the switch-sample busy loop is calibrated for
    // it and the application's init expects reset defaults. HSI is always
    // the reset source; this only acts after a ROM DFU `leave`.
    if pac::RCC.cfgr().read().sws() != Sw::HSI {
        pac::RCC.cr().modify(|w| w.set_hsion(true));
        while !pac::RCC.cr().read().hsirdy() {}
        pac::RCC.cfgr().modify(|w| w.set_sw(Sw::HSI));
        while pac::RCC.cfgr().read().sws() != Sw::HSI {}
    }
    asm::dsb();
    asm::isb();

    // The ROM DFU `leave` path may hand over with PRIMASK set. Update mode's
    // USB interrupt has to be able to fire and the application expects the
    // reset default; everything that could fire was disabled and unpended
    // above, so nothing happens until something is enabled on purpose.
    // SAFETY: no critical section is open on the entry path.
    unsafe { cortex_m::interrupt::enable() };
}

/// Divert into the ST ROM DFU exactly as fw/src/dfu.rs did: map system
/// memory at address 0 and bootstrap from its vector table. We are in reset
/// state apart from the SYSCFG clock, which the ROM expects.
fn rom_dfu() -> ! {
    pac::SYSCFG
        .cfgr1()
        .modify(|w| w.set_mem_mode(MemMode::SYSTEM_FLASH));
    asm::dsb();
    asm::isb();
    // SAFETY: fixed ROM addresses; `bootstrap` never returns.
    unsafe {
        let sp = (SYSTEM_MEMORY as *const u32).read_volatile();
        let rv = ((SYSTEM_MEMORY + 4) as *const u32).read_volatile();
        asm::bootstrap(sp as *const u32, rv as *const u32)
    }
}

/// `cortex_m::asm::delay` units for one millisecond at 8 MHz: the loop is
/// `subs; bne` over `1 + n/2` iterations, 4 cycles each on a Cortex-M0
/// (~2 cycles per unit, which fw/src/ws2812.rs confirmed with TIM2 at
/// 48 MHz), so 8000 cycles ≈ 4000 units.
const DELAY_1MS: u32 = 4000;

/// Encoder switch held at power-up? Pull-up on, settle 1 ms, then 20
/// samples 1 ms apart must all read low (active low, no external resistor).
/// The pin and its clock go back to reset state afterwards; a broken or
/// unpopulated switch reads high through the pull-up and boots the app.
fn switch_held() -> bool {
    board::sw_clock(true);
    let _ = pac::RCC.ahbenr().read();
    board::SW_PORT
        .pupdr()
        .modify(|w| w.set_pupdr(board::SW_PIN, Pupdr::PULL_UP));
    board::SW_PORT
        .moder()
        .modify(|w| w.set_moder(board::SW_PIN, Moder::INPUT));
    asm::delay(DELAY_1MS);
    let mut held = true;
    for _ in 0..20 {
        if board::SW_PORT.idr().read().idr(board::SW_PIN) == Idr::HIGH {
            held = false;
            break;
        }
        asm::delay(DELAY_1MS);
    }
    board::SW_PORT
        .pupdr()
        .modify(|w| w.set_pupdr(board::SW_PIN, Pupdr::FLOATING));
    board::SW_PORT
        .moder()
        .modify(|w| w.set_moder(board::SW_PIN, Moder::INPUT));
    board::sw_clock(false);
    held
}

/// The memory-mapped application slot.
fn app_slot() -> &'static [u8] {
    // SAFETY: readable flash, never handed out mutably.
    unsafe { core::slice::from_raw_parts(APP_BASE as *const u8, APP_SIZE as usize) }
}

/// Start the (already validated) application. Runs only from the entry
/// path, before anything else used RAM below 0xC0 or set up peripherals.
/// FAULTS is deliberately left alone: the application clears it once it is
/// running (fw/src/boot.rs), so a fault in its init window still counts and
/// the next boot lands in update mode instead of repeating the jump.
#[inline(never)]
fn jump_to_app(reason: u32) -> ! {
    handoff::set_reason(reason);

    // Cortex-M0 has no VTOR: copy the app's table to the start of SRAM and
    // map SRAM at address 0. Both memory.x files keep 0x20000000..0xC0 free,
    // so this clobbers nothing.
    let src = APP_BASE as *const u32;
    let dst = RAM_BASE as *mut u32;
    for i in 0..(VECTORS_LEN / 4) as usize {
        // SAFETY: both ranges are fixed, word-aligned and unaliased.
        unsafe { dst.add(i).write_volatile(src.add(i).read_volatile()) };
    }
    pac::SYSCFG.cfgr1().modify(|w| w.set_mem_mode(MemMode::SRAM));
    asm::dsb();
    asm::isb();

    // SAFETY: last words — SysTick off, PRIMASK clear, then MSP/PC from the
    // app's table; `bootload` never returns.
    unsafe {
        (*SYST::PTR).csr.write(0);
        cortex_m::interrupt::enable();
        asm::bootload(APP_BASE as *const u32)
    }
}

// ---- update mode --------------------------------------------------------------

bind_interrupts!(struct Irqs {
    USB => usb::InterruptHandler<peripherals::USB>;
});

/// Clocks up (same configuration as the application: HSI48 trimmed by CRS
/// from USB SOF, 48 MHz core), then the executor with the single task.
fn update_mode(reason: u32) -> ! {
    handoff::set_reason(reason);
    // From here on a fault retries update mode, not the application.
    PHASE.store(PHASE_UPDATE_MODE, Ordering::SeqCst);

    let mut config = Config::default();
    config.rcc.hsi48 = Some(Hsi48Config { sync_from_usb: true });
    config.rcc.sys = Sysclk::HSI48;
    let p = embassy_stm32::init(config);

    static EXECUTOR: StaticCell<Executor> = StaticCell::new();
    EXECUTOR
        .init(Executor::new())
        .run(|spawner| spawner.must_spawn(update_task(p)))
}

/// "a.b.c" -> USB bcdDevice (a in the high byte, b/c a nibble each); same
/// formula as the application so tools can read the version from the
/// device descriptor.
const fn version_bcd(s: &str) -> u16 {
    let b = s.as_bytes();
    let mut parts = [0u16; 3];
    let mut pi = 0;
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'.' {
            pi += 1;
        } else if b[i].is_ascii_digit() && pi < 3 {
            parts[pi] = parts[pi] * 10 + (b[i] - b'0') as u16;
        }
        i += 1;
    }
    ((parts[0] & 0xFF) << 8) | ((parts[1] & 0xF) << 4) | (parts[2] & 0xF)
}

/// Vendor "raw HID" interface like the application's (usage page 0xFF60),
/// but with 64-byte IN and OUT reports, no report IDs.
#[rustfmt::skip]
const RAW_HID_DESC: &[u8] = &[
    0x06, 0x60, 0xFF, // Usage Page (Vendor 0xFF60)
    0x09, 0x61,       // Usage (0x61)
    0xA1, 0x01,       // Collection (Application)
    0x09, 0x62,       //   Usage (0x62)
    0x15, 0x00,       //   Logical Minimum (0)
    0x26, 0xFF, 0x00, //   Logical Maximum (255)
    0x75, 0x08,       //   Report Size (8)
    0x95, 0x40,       //   Report Count (64)
    0x81, 0x02,       //   Input (Data, Var, Abs)
    0x09, 0x63,       //   Usage (0x63)
    0x15, 0x00,       //   Logical Minimum (0)
    0x26, 0xFF, 0x00, //   Logical Maximum (255)
    0x75, 0x08,       //   Report Size (8)
    0x95, 0x40,       //   Report Count (64)
    0x91, 0x02,       //   Output (Data, Var, Abs)
    0xC0,             // End Collection
];

#[embassy_executor::task]
async fn update_task(p: Peripherals) -> ! {
    // LEDs first, so the pad shows something the moment update mode starts:
    // the ring goes dark (it would otherwise keep the app's last frame), the
    // keys start breathing from the protocol loop. The Output handles only
    // hold the pin configuration; write_raw drives BSRR directly.
    #[cfg(not(feature = "proto"))]
    let (_led_key, _led_ug) = (
        Output::new(p.PA8, Level::Low, Speed::VeryHigh),
        Output::new(p.PB14, Level::Low, Speed::VeryHigh),
    );
    #[cfg(feature = "proto")]
    let (_led_key, _led_ug) = (
        Output::new(p.PB4, Level::Low, Speed::VeryHigh),
        Output::new(p.PA0, Level::Low, Speed::VeryHigh),
    );
    ws2812::write_raw(board::UG_PORT, board::UG_PIN, &[Grb::default(); board::UG_LEN]);

    let mut updater = Updater::new(
        flashprog::FlashProgrammer::new(Flash::new_blocking(p.FLASH)),
        BOOT_VERSION,
        VARIANT,
    );

    // ---- USB: one raw-HID interface under the bootloader's own PID ----
    let driver = Driver::new(p.USB, Irqs, p.PA12, p.PA11);
    let mut usb_config = embassy_usb::Config::new(BOOT_VID, BOOT_PID);
    usb_config.manufacturer = Some(BOOT_MANUFACTURER);
    usb_config.product = Some(BOOT_PRODUCT);
    // The MCU's factory 96-bit UID: the host matches it against the pad it
    // sent ENTER_BOOT to.
    usb_config.serial_number = Some(embassy_stm32::uid::uid_hex());
    usb_config.device_release = version_bcd(BOOT_VERSION);

    static CONFIG_DESC: StaticCell<[u8; 128]> = StaticCell::new();
    static BOS_DESC: StaticCell<[u8; 32]> = StaticCell::new();
    static CONTROL_BUF: StaticCell<[u8; 64]> = StaticCell::new();
    static HID_STATE: StaticCell<State> = StaticCell::new();

    let mut builder = embassy_usb::Builder::new(
        driver,
        usb_config,
        CONFIG_DESC.init([0; 128]),
        BOS_DESC.init([0; 32]),
        &mut [],
        CONTROL_BUF.init([0; 64]),
    );
    let hid = HidReaderWriter::<_, BOOT_REPORT_LEN, BOOT_REPORT_LEN>::new(
        &mut builder,
        HID_STATE.init(State::new()),
        embassy_usb::class::hid::Config {
            report_descriptor: RAW_HID_DESC,
            request_handler: None,
            poll_ms: 1,
            max_packet_size: BOOT_REPORT_LEN as u16,
        },
    );
    let mut usb = builder.build();
    let (reader, writer) = hid.split();

    join(usb.run(), protocol_loop(reader, writer, &mut updater))
        .await
        .0
}

/// LED frame period; also the cadence of the idle loop.
const FRAME_MS: u64 = 30;

/// Reports in, replies out, LED cue in between. Never returns: update mode
/// has no timeout, only BOOT_RUN / ENTER_DFU / a power cycle leave it.
async fn protocol_loop<'d, D: UsbDriver<'d>, P: Programmer>(
    mut reader: HidReader<'d, D, BOOT_REPORT_LEN>,
    mut writer: HidWriter<'d, D, BOOT_REPORT_LEN>,
    updater: &mut Updater<P>,
) -> ! {
    let mut buf = [0u8; BOOT_REPORT_LEN];
    let mut rep = [0u8; BOOT_REPORT_LEN];
    let mut cue = Cue::new();
    let mut next_frame = Instant::now();
    loop {
        // One frame per FRAME_MS, not one per report: a burst of DATA
        // reports must not spend its time in the LED critical section.
        let now = Instant::now();
        if now >= next_frame {
            ws2812::write_raw(board::KEY_PORT, board::KEY_PIN, &cue.frame());
            next_frame = now + Duration::from_millis(FRAME_MS);
        }
        match select(reader.read(&mut buf), Timer::at(next_frame)).await {
            Either::Second(()) => {}
            // `read` fails immediately while the endpoint is disabled (before
            // the host configures us): back off instead of spinning.
            Either::First(Err(_)) => Timer::after_millis(100).await,
            Either::First(Ok(n)) => {
                let action = updater.handle(&buf[..n], &mut rep);
                if updater.take_info_answered() {
                    // Enumerated and talking: the fault escalation starts over.
                    handoff::set_faults(0);
                }
                match outcome(&rep) {
                    Outcome::UpdateDone => {
                        cue.success();
                        // Read back by the boot path after BOOT_RUN.
                        handoff::set_reason(REASON_AFTER_UPDATE);
                    }
                    Outcome::Error => cue.error(),
                    Outcome::Quiet => {}
                }
                // Always the full 64-byte report: the Windows HID class
                // driver discards input reports shorter than the descriptor
                // says; the unused tail is zero.
                let _ = writer.write(&rep).await;
                match action {
                    Action::Reply(_) => {}
                    Action::ResetToDfu => reset_with(REQ_ROM_DFU).await,
                    Action::ResetToApp => reset_with(REQ_RUN).await,
                }
            }
        }
    }
}

/// Let the ack reach the host before dropping off the bus, then arm the
/// request and reset; the boot decision does the rest from reset state.
async fn reset_with(request: u32) -> ! {
    Timer::after_millis(50).await;
    handoff::write_request(request);
    SCB::sys_reset()
}

// ---- LED cue -------------------------------------------------------------------

/// Brightest channel value used, out of 255: 13 keys at this level draw a
/// few tens of mA, far inside the ~500 mA VBUS budget for both chains.
const LEVEL_MAX: u32 = 40;
/// Half a breathing period in frames (2 s period at 30 ms).
const BREATH_HALF: u32 = 33;
/// Solid green after a verified upload: 1 s.
const SUCCESS_FRAMES: u32 = 33;
/// Three red blinks: 6 phases of 150 ms.
const ERROR_FRAMES: u32 = 30;

#[derive(Clone, Copy, PartialEq, Eq)]
enum CueMode {
    Idle,
    Success,
    Error,
}

struct Cue {
    mode: CueMode,
    tick: u32,
}

impl Cue {
    fn new() -> Self {
        Cue {
            mode: CueMode::Idle,
            tick: 0,
        }
    }

    fn success(&mut self) {
        self.mode = CueMode::Success;
        self.tick = 0;
    }

    fn error(&mut self) {
        self.mode = CueMode::Error;
        self.tick = 0;
    }

    /// The next frame for all 13 keys.
    fn frame(&mut self) -> [Grb; KEY_LEN] {
        let over = match self.mode {
            CueMode::Idle => false,
            CueMode::Success => self.tick >= SUCCESS_FRAMES,
            CueMode::Error => self.tick >= ERROR_FRAMES,
        };
        if over {
            self.mode = CueMode::Idle;
            self.tick = 0;
        }
        let px = match self.mode {
            CueMode::Idle => {
                // Amber triangle wave, 0 → LEVEL_MAX → 0 over 2 s.
                let t = self.tick % (2 * BREATH_HALF);
                let up = if t < BREATH_HALF { t } else { 2 * BREATH_HALF - t };
                let level = LEVEL_MAX * up / BREATH_HALF;
                Grb::rgb(level as u8, (level * 3 / 8) as u8, 0)
            }
            CueMode::Success => Grb::rgb(0, LEVEL_MAX as u8, 0),
            CueMode::Error => {
                if (self.tick / 5) % 2 == 0 {
                    Grb::rgb(LEVEL_MAX as u8, 0, 0)
                } else {
                    Grb::default()
                }
            }
        };
        self.tick = self.tick.wrapping_add(1);
        [px; KEY_LEN]
    }
}

// ---- faults ----------------------------------------------------------------------

/// Where the bootloader is: 0 on the entry path, `PHASE_UPDATE_MODE` from
/// the moment `update_mode` starts. `escalate` reads it so a fault in
/// update mode retries *update mode*: a plain reset would find a valid
/// application, boot it, and quietly leave a pad that can never be updated
/// over HID, with the counter never reaching the ROM DFU rung.
///
/// A fault taken through our table while the *application* runs (its init
/// window, before it re-remaps) executes this code on the application's
/// RAM, where this address holds whatever the application put there. The
/// magic value makes that read "not update mode" — the plain-reset rung,
/// after which `main` sees the count and goes to update mode anyway.
static PHASE: AtomicU32 = AtomicU32::new(0);
/// "UPMD".
const PHASE_UPDATE_MODE: u32 = 0x5550_4D44;

/// Never a reset loop: count the fault in the FAULTS handoff word and
/// escalate. 1 → reset, back into update mode if that is where it happened
/// (a transient); 2 → reset into the ROM DFU (recoverable with the combined
/// image, no driverless path left); 3+ → halt with the keys red if the
/// clocks happen to be up, spinning on `nop` (never WFI) so an SWD probe
/// can attach through J2.
fn escalate() -> ! {
    let count = faults_count(handoff::faults()) + 1;
    handoff::set_faults(faults_word(count));
    match count {
        1 => {
            if PHASE.load(Ordering::SeqCst) == PHASE_UPDATE_MODE {
                handoff::write_request(REQ_BOOTLOADER);
            }
            SCB::sys_reset()
        }
        2 => {
            handoff::write_request(REQ_ROM_DFU);
            SCB::sys_reset()
        }
        _ => {
            let clocks_up = pac::RCC.cfgr().read().sws() == Sw::HSI48;
            let pin_ready = board::KEY_PORT.moder().read().moder(board::KEY_PIN) == Moder::OUTPUT;
            if clocks_up && pin_ready {
                ws2812::write_raw(
                    board::KEY_PORT,
                    board::KEY_PIN,
                    &[Grb::rgb(LEVEL_MAX as u8, 0, 0); KEY_LEN],
                );
            }
            loop {
                asm::nop();
            }
        }
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    escalate()
}

#[cortex_m_rt::exception]
unsafe fn HardFault(_frame: &cortex_m_rt::ExceptionFrame) -> ! {
    escalate()
}

/// Every exception and interrupt nothing else binds (SysTick, NMI, a
/// spurious IRQ line, ...). cortex-m-rt's own default is `loop {}`, which
/// would park the core dark with no reset and no escalation; here it takes
/// the same ladder as a panic or HardFault.
#[cortex_m_rt::exception]
unsafe fn DefaultHandler(_irqn: i16) -> ! {
    escalate()
}
