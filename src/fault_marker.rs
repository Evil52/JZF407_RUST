//! Reset reason tracking via CCM RAM marker word at 0x1000FFF0 + RCC CSR flags.
//! The marker survives soft resets but is garbage after power-on.

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
    // Safety: the linker pins `_fault_marker = 0x1000FFF0` (memory.x) in a NOLOAD
    // CCM section, so no other object lives there and no `&mut` can alias this
    // shared reference. CCM is always mapped on the F407 and the address is
    // word-aligned. Every 32-bit pattern is a valid AtomicU32, so reading the
    // uninitialised post-power-on value is defined (non-tag garbage = no marker).
    unsafe { &*(MARKER_ADDR as *const AtomicU32) }
}

/// Read RCC_CSR reset flags + CCM marker, then clear both.
pub fn read_and_clear() -> ResetReason {
    use embassy_stm32::pac::RCC;

    let csr = RCC.csr().read();

    let soft_marker = marker().load(Ordering::Relaxed);
    marker().store(TAG_CLEAR, Ordering::Relaxed);

    RCC.csr().modify(|w| w.set_rmvf(true));

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

/// Always reboot via this, never bare `sys_reset()`: sys_reset does NOT reset
/// peripherals, so the ETH DMA keeps running across the reset and fires an
/// interrupt before cortex-m-rt installs handlers → DefaultHandler exception.
pub fn safe_reboot() -> ! {
    use embassy_stm32::pac::RCC;

    cortex_m::interrupt::disable();

    RCC.ahb1rstr().modify(|w| w.set_ethrst(true));
    RCC.ahb1rstr().modify(|w| w.set_ethrst(false));

    cortex_m::asm::dsb();
    cortex_m::peripheral::SCB::sys_reset();
}
