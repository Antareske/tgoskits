# ppoll(2) — Linux manual page (man-pages 6.16, 2025-10-29)

## NAME
poll, ppoll - wait for some event on a file descriptor

## SYNOPSIS
```c
#include <poll.h>

int poll(struct pollfd *fds, nfds_t nfds, int timeout);

#define _GNU_SOURCE
#include <poll.h>

int ppoll(struct pollfd *fds, nfds_t nfds,
          const struct timespec *_Nullable tmo_p,
          const sigset_t *_Nullable sigmask);
```

## DESCRIPTION
`ppoll()` allows an application to safely wait until either a file descriptor
becomes ready or until a signal is caught.

If `sigmask` is NULL, no signal mask manipulation is performed (ppoll differs
from poll only in timeout precision).

## ERRORS
- `EFAULT`: fds points outside accessible address space
- `EINTR`: a signal occurred before any requested event
- `EINVAL`: nfds exceeds RLIMIT_NOFILE; or (ppoll) timeout value is negative
- `ENOMEM`: unable to allocate memory for kernel data structures

## C library/kernel differences (CRITICAL for sigsetsize)

> The raw `ppoll()` system call has a fifth argument, `size_t sigsetsize`,
> which specifies the size in bytes of the `sigmask` argument. The glibc
> `ppoll()` wrapper function specifies this argument as a fixed value
> (equal to `sizeof(kernel_sigset_t)`). See `sigprocmask(2)` for a
> discussion on the differences between the kernel and the libc notion
> of the sigset.

The kernel enforces `sigsetsize == sizeof(kernel_sigset_t) == _NSIG/8 == 8`
when sigmask is non-NULL. When sigmask is NULL, sigsetsize is not validated.

## HISTORY
- `poll()`: POSIX.1-2001, Linux 2.1.23
- `ppoll()`: POSIX.1-2024, Linux 2.6.16, glibc 2.4
