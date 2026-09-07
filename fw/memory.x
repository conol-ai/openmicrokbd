/* OpenMicro v1 memory map — STM32F072CB: 128 KiB flash in 2 KiB pages,
   16 KiB SRAM. Since fw 0.10.0 the application is linked BEHIND a resident
   bootloader and only ever started by it. Every number below is mirrored
   in ../layout/src/lib.rs (openmicro-layout: BOOT_SIZE, APP_BASE, APP_SIZE,
   VECTORS_LEN, HEADER_OFFSET, HANDOFF_*) and in scripts/fw-image.py;
   change them together.

   flash 0x08000000..0x08006000  bootloader, 24 KiB / 12 pages (boot/);
                                 never erased by an update; its info block
                                 sits at +0xC0 (read by boot::boot_info)
         0x08006000..0x0801B000  application, 84 KiB / 42 pages — THIS IMAGE
                                   +0x00  vector table (16 core + 32 IRQ = 0xC0)
                                   +0xC0  32-byte header (.app_header, boot.rs)
                                   +0xE0  .text — cortex-m-rt puts Reset first,
                                          so the reset vector is 0x080060E1 and
                                          the bootloader checks exactly that
         0x0801B000..0x0801E000  keymap.json slot (Work Louder files,
                                 src/codex/files.rs)
         0x0801E000..0x0801F800  smart_actions.json slot
         0x0801F800..0x08020000  keymap/config page (src/keymap.rs CONFIG_OFFSET)

   ram   0x20000000..0x200000C0  copy of this image's vector table: the
                                 Cortex-M0 has no VTOR, so SYSCFG remaps SRAM
                                 to address 0 instead (boot.rs). Kept out of
                                 the RAM region so .data/.bss init and the
                                 stack never overwrite it.
         0x200000C0..0x20003FF0  .data / .bss / .uninit / stack of this image
                                 (_stack_start = 0x20003FF0)
         0x20003FF0..0x20004000  handoff words shared with the bootloader:
                                 request, !request, reason, faults — they
                                 survive SCB::sys_reset, so both images keep
                                 them outside their RAM regions */
MEMORY
{
    FLASH : ORIGIN = 0x08006000, LENGTH = 0x15000
    RAM   : ORIGIN = 0x200000C0, LENGTH = 0x3F30
}

/* cortex-m-rt pins .text at _stext, which defaults to the end of the vector
   table. Push it past the 32-byte application header so the header keeps
   its slot and Reset lands at APP_BASE + 0xE0 — the address the bootloader,
   the host app and fw-image.py all require of a valid image. */
_stext = ORIGIN(FLASH) + 0xE0;

/* The header itself: boot::APP_HEADER carries #[link_section = ".app_header"];
   KEEP so LTO cannot drop it (nothing in the code reads it). */
SECTIONS
{
    .app_header ORIGIN(FLASH) + 0xC0 :
    {
        KEEP(*(.app_header .app_header.*));
    } > FLASH
} INSERT AFTER .vector_table;
