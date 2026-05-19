MEMORY
{
    /* STM32F407VETx: 512K Flash, 192K RAM (128K SRAM1 + 64K SRAM2 + 64K CCM) */
    FLASH  : ORIGIN = 0x08000000, LENGTH = 512K
    RAM    : ORIGIN = 0x20000000, LENGTH = 128K
    CCMRAM : ORIGIN = 0x10000000, LENGTH = 64K
}

/* panic-persist region: last 256 bytes of CCMRAM, preserved across soft resets */
_panic_dump_start = ORIGIN(CCMRAM) + LENGTH(CCMRAM) - 256;
_panic_dump_end   = ORIGIN(CCMRAM) + LENGTH(CCMRAM);

/* fault_marker word at 0x1000FFF0 (same address as C version) */
_fault_marker = 0x1000FFF0;

SECTIONS
{
    /* Keep CCM uninitialised so fault_marker survives soft reset */
    .ccmram (NOLOAD) :
    {
        . = ALIGN(4);
        *(.ccmram .ccmram.*);
        . = ALIGN(4);
    } > CCMRAM
}
INSERT BEFORE .bss;
