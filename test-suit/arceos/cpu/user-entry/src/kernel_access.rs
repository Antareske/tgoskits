//! Nofault kernel copy: an inaccessible range reports a fault instead of
//! entering the page-fault path.
//!
//! What leaves a range inaccessible, and what has to hold while it is probed,
//! differs per architecture, so each one supplies both.

use ax_cpu::kernel_access::{KernelAccessError, copy_from_kernel_nofault};

/// Contents the destination holds when a copy must not touch it.
const KEPT: u8 = 0xa5;

/// An address that no translation can reach, and the window that keeps it that
/// way.
#[cfg(target_arch = "aarch64")]
mod unreachable {
    use super::super::empty_user_table::EmptyUserTable;

    /// The empty user table leaves the whole user range unmapped, so address
    /// zero meets an invalid descriptor.
    pub(super) const ADDRESS: *mut u8 = core::ptr::null_mut();

    /// Holds the empty table for as long as the probe runs.
    pub(super) fn window() -> EmptyUserTable {
        EmptyUserTable::install()
    }
}

/// An address that no translation can reach, and the window that keeps it that
/// way.
#[cfg(target_arch = "riscv64")]
mod unreachable {
    /// Supervisor translation has no user-range register to empty here: the
    /// kernel probes with the very table it is running on, so the address
    /// itself has to be one that no table can translate. Bits 63:57 of this
    /// address are not the sign extension of bit 56, which makes it
    /// non-canonical under Sv39, Sv48 and Sv57 alike; the hardware reports a
    /// page fault for a non-canonical address whatever the current table maps.
    pub(super) const ADDRESS: *mut u8 = 0xfe00_0000_0000_0000 as *mut u8;

    /// The window a caller holds while probing. This address needs nothing
    /// installed to stay unreachable, so the window holds nothing.
    pub(super) struct Window;

    pub(super) fn window() -> Window {
        Window
    }
}

pub fn run() {
    // A mapped source and destination copy whole.
    let source = [1u8, 2, 3, 4, 5, 6, 7, 8];
    let mut destination = [0u8; 8];
    // SAFETY: both ranges are this frame's own arrays.
    let copied = unsafe {
        copy_from_kernel_nofault(destination.as_mut_ptr(), source.as_ptr(), source.len())
    };
    assert!(copied.is_ok());
    assert_eq!(destination, source);

    // Inaccessible ranges report the fault from both sides.
    let _window = unreachable::window();
    let address = unreachable::ADDRESS;

    // A source that faults before the first byte is read leaves the destination
    // untouched.
    let mut kept = [KEPT; 8];
    // SAFETY: the destination is this frame's own array; the source is the
    // unreachable address this case means to probe.
    let faulted = unsafe { copy_from_kernel_nofault(kept.as_mut_ptr(), address, kept.len()) };
    assert_eq!(faulted, Err(KernelAccessError::Fault));
    assert_eq!(kept, [KEPT; 8]);

    // The store side reports the same way, with the source readable.
    let source = [7u8; 8];
    // SAFETY: the source is this frame's own array; the destination is the
    // unreachable address this case means to probe.
    let faulted = unsafe { copy_from_kernel_nofault(address, source.as_ptr(), source.len()) };
    assert_eq!(faulted, Err(KernelAccessError::Fault));

    std::println!("CPU_KERNEL_ACCESS_OK");
}
