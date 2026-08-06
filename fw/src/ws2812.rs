//! Bit-banged WS2812/SK6812 driver (GRB, 800 kHz) for the two LED chains.
//!
//! The SK6812MINI-E timing tolerance (±150 ns) makes a cycle-counted bit-bang
//! at 48 MHz perfectly adequate; each chain refresh (< 1 ms for 21 LEDs total)
//! runs inside a critical section so the waveform is never stretched by an
//! interrupt. USB survives this: the FS peripheral buffers a full frame in
//! hardware and 1 ms of deferred IRQ handling is within its tolerance — the
//! same trade every bit-banged RGB keyboard makes.

use cortex_m::asm;
use embassy_stm32::gpio::Output;
use embassy_stm32::pac;
#[cfg(not(feature = "proto"))]
use embassy_stm32::gpio::OutputOpenDrain;

/// Both chains run plain 3.3 V push-pull on every revision: hardware-proven
/// (2026-08-06) on the fitted SK6812 reel once the bit timing was fixed —
/// the LEDs' real VIH is below 3.3 V despite the 0.7*VDD datasheet figure.
pub type LedPin<'d> = Output<'d>;


#[cfg(feature = "proto")]
pub use write as write_key_alias;
#[cfg(feature = "proto")]
pub fn write_key(pin: &mut KeyLedPin<'_>, pixels: &[Grb]) {
    write(pin, pixels)
}

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
    /// Scale all channels by `num/64` — the brightness cap that keeps 21 LEDs
    /// inside the 500 mA VBUS budget (see the board's #[high_current] note).
    pub fn scaled(self, num: u8) -> Self {
        let s = |c: u8| ((c as u16 * num as u16) / 64) as u8;
        Grb {
            g: s(self.g),
            r: s(self.r),
            b: s(self.b),
        }
    }
}

/// Delay-unit counts, calibrated against TIM2 measurements of the emitted
/// waveform (2026-08-05). Two facts from measurement: cortex_m::asm::delay
/// is ~2 cycles/unit here, and embassy HAL edge calls cost 26-43 cycles —
/// which made a valid 300 ns T0H physically impossible through the HAL. The
/// writer therefore uses RAW single-store BSRR writes (~2 cycles), and these
/// units target T1H 600 / T1L 600 / T0H 300 / T0L 900 ns at 48 MHz.
const T1H: u32 = 8;
const T1L: u32 = 8;
const T0H: u32 = 1;
const T0L: u32 = 15;

/// Shift pixels out via exact-cycle high pulses. Only the HIGH phase of a
/// WS2812 bit is timing-critical (lows may stretch to just under the 80 us
/// latch limit), so each high is a str/nops/str inline-asm sequence with
/// single-cycle precision — measured floors made every abstraction-based
/// approach ~30 cycles per phase, hopelessly long for the 14-cycle T0H.
/// The inter-bit Rust loop overhead lands harmlessly in the low phase.
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
