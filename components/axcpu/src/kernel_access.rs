//! Nofault access to kernel-space byte ranges.

/// Failure returned by a nofault kernel copy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KernelAccessError {
    /// The source or destination range was not accessible without resolving a
    /// page fault.
    Fault,
}

unsafe extern "C" {
    fn __axcpu_copy_from_kernel_nofault(dst: *mut u8, src: *const u8, len: usize) -> usize;
}

/// Copies `len` bytes from `src` to `dst` without resolving faults.
///
/// Both the load and the store are registered in the nofault exception table,
/// so an inaccessible address on either side redirects control to a recovery
/// label instead of entering the OS page-fault path. The call therefore never
/// sleeps and never allocates, and may be used from interrupt context.
///
/// A byte range that spans a hole copies the bytes preceding the hole and then
/// reports [`KernelAccessError::Fault`]; the bytes after the hole keep their
/// previous contents.
///
/// # Safety
///
/// `src` and `dst` must each denote `len` bytes the caller is allowed to name.
/// The copy only avoids *faulting* on an inaccessible range; it does not make
/// a stale or wrong address correct data. A caller that resolves a fault must
/// not have left a lock held whose critical section must remain nofault.
pub unsafe fn copy_from_kernel_nofault(
    dst: *mut u8,
    src: *const u8,
    len: usize,
) -> Result<(), KernelAccessError> {
    // SAFETY: the caller guarantees the two ranges; whichever of them is not
    // mapped is recovered through the nofault exception table.
    let remaining = unsafe { __axcpu_copy_from_kernel_nofault(dst, src, len) };
    if remaining == 0 {
        Ok(())
    } else {
        Err(KernelAccessError::Fault)
    }
}
