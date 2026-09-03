MEMORY
{
    /* STM32F072CB has 128 KiB. The top 20 KiB stay out of the image: the
       Work Louder file slots (keymap.json 0x0801B000, smart_actions.json
       0x0801E000 — src/codex/files.rs) and the keymap/config page at
       0x0801F800. */
    FLASH : ORIGIN = 0x08000000, LENGTH = 108K
    RAM   : ORIGIN = 0x20000000, LENGTH = 16K
}
