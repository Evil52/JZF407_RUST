//! Reset reason tracking via CCM RAM marker + RCC CSR flags.
//! Marker word at 0x1000FFF0 (same address as C version).
//! Survives soft resets but cleared on power-on.

use core::sync::atomic::{AtomicU32, Ordering};

const MARKER_ADDR: u32 = 0x1000_FFF0;

const TAG_STACK_OVERFLOW: u32 = 0xDEAD_0001;
const TAG_MALLOC_FAIL: u32 = 0xDEAD_0002;
const TAG_REMOTE_REBOOT: u32 = 0xDEAD_0003;
const TAG_CLEAR: u32 = 0x0000_0000;

#[derive(Clone, Copy, PartialEq)]
pub enum ResetReason {
    PowerOn,
    NrstPin,
    Software,
    IwdgTimeout,
    BrownOut,
    StackOverflow,
    MallocFailed,
    RemoteReboot,
    Unknown,
}

impl ResetReason {
    pub fn as_str(self) -> &'static str {
        match self {
            ResetReason::PowerOn => "power_on",
            ResetReason::NrstPin => "nrst_pin",
            ResetReason::Software => "software",
            ResetReason::IwdgTimeout => "iwdg_timeout",
            ResetReason::BrownOut => "brown_out",
            ResetReason::StackOverflow => "stack_overflow",
            ResetReason::MallocFailed => "malloc_failed",
            ResetReason::RemoteReboot => "remote_reboot",
            ResetReason::Unknown => "unknown",
        }
    }
}

fn marker() -> &'static AtomicU32 {
    // Safety: forming a `&'static AtomicU32` from a fixed address is sound here because:
    //  * Reserved storage. The linker pins `_fault_marker = 0x1000FFF0` (see memory.x)
    //    inside a NOLOAD CCM section, so the compiler never places any other object
    //    there. Nothing else in the program ever takes a reference to this word, so the
    //    shared `&` cannot alias a `&mut` to the same location — no UB from aliasing.
    //  * Always mapped & aligned. CCM RAM (0x10000000..+64K) is always present on the
    //    F407 (CPU-only, no DMA), and 0x1000FFF0 is word-aligned, so the read/write the
    //    returned reference performs can never fault or be misaligned.
    //  * No invalid bit patterns. After power-on the word is uninitialised, but every
    //    32-bit value is a valid `AtomicU32`, so reading it is defined (we treat
    //    non-tag garbage as "no marker" in read_and_clear). NOLOAD means a soft reset
    //    preserves whatever we last stored, which is the whole point of the marker.
    //  * No leak. We hand out a borrow of statically-reserved memory; nothing is
    //    allocated, so there is nothing to leak.
    unsafe { &*(MARKER_ADDR as *const AtomicU32) }
}

/// Read RCC_CSR reset flags + CCM marker, then clear both.
pub fn read_and_clear() -> ResetReason {
    use embassy_stm32::pac::RCC;

    let csr = RCC.csr().read();

    let soft_marker = marker().load(Ordering::Relaxed);
    marker().store(TAG_CLEAR, Ordering::Relaxed);

    // Clear RCC CSR reset flags
    RCC.csr().modify(|w| w.set_rmvf(true));

    // CCM marker takes priority (written just before soft reset)
    if soft_marker == TAG_STACK_OVERFLOW {
        return ResetReason::StackOverflow;
    }
    if soft_marker == TAG_MALLOC_FAIL {
        return ResetReason::MallocFailed;
    }
    if soft_marker == TAG_REMOTE_REBOOT {
        return ResetReason::RemoteReboot;
    }

    if csr.porrstf() {
        return ResetReason::PowerOn;
    }
    if csr.borrstf() {
        return ResetReason::BrownOut;
    }
    if csr.wdgrstf() {
        return ResetReason::IwdgTimeout;
    }
    if csr.sftrstf() {
        return ResetReason::Software;
    }
    if csr.padrstf() {
        return ResetReason::NrstPin;
    }

    ResetReason::Unknown
}

pub fn mark_remote_reboot() {
    marker().store(TAG_REMOTE_REBOOT, Ordering::Relaxed);
}

/// Cleanly reset the MCU. `sys_reset()` alone does NOT reset peripherals, so
/// the ETH DMA keeps running across the reset and fires an interrupt before
/// cortex-m-rt installs handlers → DefaultHandler Exception. Disable interrupts
/// and reset the ETH MAC + DMA via RCC before resetting the core.
pub fn safe_reboot() -> ! {
    use embassy_stm32::pac::RCC;

    cortex_m::interrupt::disable();

    // Pulse ETH reset in AHB1RSTR to stop the DMA engine before the core resets.
    RCC.ahb1rstr().modify(|w| w.set_ethrst(true));
    RCC.ahb1rstr().modify(|w| w.set_ethrst(false));

    cortex_m::asm::dsb();
    cortex_m::peripheral::SCB::sys_reset();
}
