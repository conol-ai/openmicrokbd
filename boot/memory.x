MEMORY
{
    /* The first 12 pages (24 KiB) of the 128 KiB flash belong to the
       bootloader; the application slot starts at 0x08006000 and the Work
       Louder file slots / config page keep the top 20 KiB (layout/src/lib.rs).
       Updates never erase below 0x08006000. */
    FLASH : ORIGIN = 0x08000000, LENGTH = 0x6000

    /* 0x20000000..0x200000C0 holds the application's relocated vector table
       (written last, just before the jump, so the bootloader's own statics
       must not live there) and 0x20003FF0..0x20004000 the four handoff
       words that survive a system reset. Neither belongs to this image.
       Stack top = 0x20003FF0. */
    RAM   : ORIGIN = 0x200000C0, LENGTH = 0x3F30
}

/* The 32-byte info block sits right behind the 0xC0-byte vector table and
   .text (with cortex-m-rt's Reset first) starts after it, so the reset
   vector of a correctly linked bootloader is exactly 0x080000E1. */
_stext = ORIGIN(FLASH) + 0xE0;

SECTIONS
{
    .boot_info ORIGIN(FLASH) + 0xC0 :
    {
        KEEP(*(.boot_info .boot_info.*));
    } > FLASH
} INSERT AFTER .vector_table;
