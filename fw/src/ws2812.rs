//! Bit-banged WS2812/SK6812 driver (GRB, 800 kHz) for the two LED chains.
//!
//! The SK6812MINI-E timing tolerance (±150 ns) makes a cycle-counted bit-bang
//! at 48 MHz perfectly adequate; each chain refresh (< 1 ms for 29 LEDs total)
//! runs inside a critical section so the waveform is never stretched by an
//! interrupt. USB survives this: the FS peripheral buffers a full frame in
//! hardware and 1 ms of deferred IRQ handling is within its tolerance — the
//! same trade every bit-banged RGB keyboard makes.

use cortex_m::asm;
use embassy_stm32::gpio::Output;

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
    /// Scale all channels by `num/64` — the brightness cap that keeps 29 LEDs
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

/// Cycle counts at 48 MHz. WS2812 nominal: T1H 0.7us / T1L 0.6us,
/// T0H 0.35us / T0L 0.8us; each asm::delay unit is ~1 cycle on Cortex-M0,
/// and the GPIO writes add ~6 cycles absorbed into the low period.
const T1H: u32 = 30;
const T1L: u32 = 22;
const T0H: u32 = 12;
const T0L: u32 = 34;

/// Shift one chain's worth of pixels out of `pin`. Caller provides the pixels
/// in chain order (LED 0 = first in the chain).
pub fn write(pin: &mut Output<'_>, pixels: &[Grb]) {
    critical_section::with(|_| {
        for px in pixels {
            for byte in [px.g, px.r, px.b] {
                let mut b = byte;
                for _ in 0..8 {
                    if b & 0x80 != 0 {
                        pin.set_high();
                        asm::delay(T1H);
                        pin.set_low();
                        asm::delay(T1L);
                    } else {
                        pin.set_high();
                        asm::delay(T0H);
                        pin.set_low();
                        asm::delay(T0L);
                    }
                    b <<= 1;
                }
            }
        }
    });
    // Latch: > 80 us low.
    asm::delay(48 * 100);
}
