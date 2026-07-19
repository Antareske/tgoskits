# signalfd(2) — Linux manual page (man-pages 6.16, 2025-09-21)

## NAME
signalfd - create a file descriptor for accepting signals

## SYNOPSIS
```c
#include <sys/signalfd.h>

int signalfd(int fd, const sigset_t *mask, int flags);
```

## DESCRIPTION
Creates a file descriptor that can be used to accept signals targeted at the
caller. The `mask` argument specifies the set of signals to accept.

If `fd` is -1, creates a new file descriptor. If `fd` is a valid existing
signalfd descriptor, replaces its signal mask with `mask`.

Flags (since Linux 2.6.27):
- `SFD_NONBLOCK`: set O_NONBLOCK on the fd
- `SFD_CLOEXEC`: set FD_CLOEXEC on the fd

## RETURN VALUE
On success, returns a signalfd file descriptor (new fd if fd=-1, or fd itself).
On error, returns -1 and sets errno.

## ERRORS
- `EBADF`: fd is not a valid file descriptor
- `EINVAL`: fd is not a valid signalfd file descriptor
- `EINVAL`: flags is invalid; or in Linux 2.6.26 or earlier, flags is nonzero
- `EMFILE`: per-process open fd limit reached
- `ENFILE`: system-wide open file limit reached
- `ENODEV`: could not mount anonymous inode device
- `ENOMEM`: insufficient memory

## C library/kernel differences (CRITICAL for sigsetsize)

> The underlying Linux system call requires an additional argument,
> `size_t sizemask`, which specifies the size of the `mask` argument.
> The glibc `signalfd()` wrapper function does not include this argument,
> since it provides the required value for the underlying system call.

The kernel syscall is `signalfd4(int fd, const sigset_t *mask, size_t sigsz, int flags)`.
The kernel enforces `sigsz == sizeof(kernel_sigset_t) == _NSIG/8 == 8`.
Any other value (including 0, 4, 7, 9, 16, 128) returns EINVAL.

## HISTORY
- `signalfd()`: Linux 2.6.22, glibc 2.8
- `signalfd4()`: Linux 2.6.27
