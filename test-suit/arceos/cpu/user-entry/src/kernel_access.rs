//! Nofault kernel copy: an inaccessible range reports a fault instead of
//! entering the page-fault path.

use ax_cpu::kernel_access::{KernelAccessError, copy_from_kernel_nofault};

use super::empty_user_table::EmptyUserTable;

/// Contents the destination holds when a copy must not touch it.
const KEPT: u8 = 0xa5;

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

    // Inaccessible ranges report the fault from both sides. The empty user
    // table leaves the whole user range unmapped, so a probe of address zero
    // cannot reach a translation.
    let _table = EmptyUserTable::install();

    // A source that faults before the first byte is read leaves the destination
    // untouched.
    let mut kept = [KEPT; 8];
    // SAFETY: the destination is this frame's own array; the source is the
    // unmapped address this case means to probe.
    let faulted =
        unsafe { copy_from_kernel_nofault(kept.as_mut_ptr(), core::ptr::null(), kept.len()) };
    assert_eq!(faulted, Err(KernelAccessError::Fault));
    assert_eq!(kept, [KEPT; 8]);

    // The store side reports the same way, with the source readable.
    let source = [7u8; 8];
    // SAFETY: the source is this frame's own array; the destination is the
    // unmapped address this case means to probe.
    let faulted =
        unsafe { copy_from_kernel_nofault(core::ptr::null_mut(), source.as_ptr(), source.len()) };
    assert_eq!(faulted, Err(KernelAccessError::Fault));

    std::println!("CPU_KERNEL_ACCESS_OK");
}
