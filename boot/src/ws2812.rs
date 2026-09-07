//! Bit-banged WS2812/SK6812 driver (GRB, 800 kHz) — a trimmed copy of
//! fw/src/ws2812.rs with the same cycle counts (keep the two in sync).
//!
//! The SK6812MINI-E timing tolerance (±150 ns) makes a cycle-counted
//! bit-bang at 48 MHz perfectly adequate; each chain refresh runs inside a
//! critical section so the waveform is never stretched by an interrupt. USB
//! survives this: the FS peripheral buffers a full frame in hardware and
//! < 1 ms of deferred IRQ handling is within its tolerance. The bootloader
//! only calls this from update mode (48 MHz core, pin already configured
//! push-pull VeryHigh) and, best effort, from the fault handler.

use cortex_m::asm;
use embassy_stm32::pac;

/// One 8-bit-per-channel colour, kept in GRB wire order.
#[derive(Clone, Copy, Default)]
pub struct Grb {
    pub g: u8,
    pub r: u8,
    pub b: u8,
}

impl Grb {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Grb { g, r, b }
    }
}

/// Shift pixels out via exact-cycle high pulses. Only the HIGH phase of a
/// WS2812 bit is timing-critical (lows may stretch to just under the 80 us
/// latch limit), so each high is a str/nops/str inline-asm sequence with
/// single-cycle precision: raw BSRR stores cost ~2 cycles where HAL edge
/// calls measured 26-43, hopelessly long for the 14-cycle T0H. The units
/// were calibrated against TIM2 captures of the emitted waveform
/// (2026-08-05) and target T1H 600 / T0H 300 ns at 48 MHz.
pub fn write_raw(port: pac::gpio::Gpio, pin: usize, pixels: &[Grb]) {
    let bsrr = port.bsrr().as_ptr() as *mut u32;
    let set: u32 = 1 << pin;
    let clr: u32 = 1 << (pin + 16);
    critical_section::with(|_| {
        for px in pixels {
            for byte in [px.g, px.r, px.b] {
                let mut b = byte;
                for _ in 0..8 {
                    if b & 0x80 != 0 {
                        // T1H ~ 29 cycles: 2 (store) + 26 nops + next store
                        unsafe {
                            core::arch::asm!(
                                "str {s}, [{a}]",
                                "nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop",
                                "nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop",
                                "str {c}, [{a}]",
                                a = in(reg) bsrr, s = in(reg) set, c = in(reg) clr,
                                options(nostack),
                            );
                        }
                    } else {
                        // T0H ~ 14 cycles: 2 (store) + 11 nops + next store
                        unsafe {
                            core::arch::asm!(
                                "str {s}, [{a}]",
                                "nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop",
                                "str {c}, [{a}]",
                                a = in(reg) bsrr, s = in(reg) set, c = in(reg) clr,
                                options(nostack),
                            );
                        }
                    }
                    // Low phase: loop overhead (~15 cycles) plus this pad —
                    // uncritical, anywhere between ~200 ns and 80 us works.
                    asm::delay(8);
                    b <<= 1;
                }
            }
        }
    });
    // Latch: > 80 us low.
    asm::delay(2400);
}
