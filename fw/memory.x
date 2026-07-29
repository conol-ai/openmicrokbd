MEMORY
{
    /* STM32F072CB has 128 KiB. The last 2 KiB page belongs exclusively to
       keymap/config persistence at 0x0801F800. */
    FLASH : ORIGIN = 0x08000000, LENGTH = 126K
    RAM   : ORIGIN = 0x20000000, LENGTH = 16K
}
