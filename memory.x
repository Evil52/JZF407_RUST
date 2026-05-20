MEMORY
{
    /* STM32F407VETx: 512K Flash, 192K RAM total = 128K SRAM (112K SRAM1 + 16K SRAM2) + 64K CCM.
       CCM is core-coupled (CPU-only, no DMA) — we use its top word as a reset-reason marker. */
    FLASH  : ORIGIN = 0x08000000, LENGTH = 512K
    RAM    : ORIGIN = 0x20000000, LENGTH = 128K
    CCMRAM : ORIGIN = 0x10000000, LENGTH = 64K
}

/* fault_marker word at 0x1000FFF0 (same address as C version): top 16 bytes of CCM */
_fault_marker = 0x1000FFF0;

/* panic-persist region: 240 bytes ending just below the fault_marker so a panic
   dump can never clobber the reset-reason marker (they used to overlap). */
_panic_dump_start = ORIGIN(CCMRAM) + LENGTH(CCMRAM) - 256;
_panic_dump_end   = _fault_marker;

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
