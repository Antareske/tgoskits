# epoll_wait(2) — Linux manual page (man-pages 6.16, 2025-09-21)

## NAME
epoll_wait, epoll_pwait, epoll_pwait2 - wait for an I/O event on an epoll file descriptor

## SYNOPSIS
```c
#include <sys/epoll.h>

int epoll_wait(int epfd, struct epoll_event events[n], int n, int timeout);

int epoll_pwait(int epfd, struct epoll_event events[n], int n,
                int timeout,
                const sigset_t *_Nullable sigmask);

int epoll_pwait2(int epfd, struct epoll_event events[n], int n,
                 const struct timespec *_Nullable timeout,
                 const sigset_t *_Nullable sigmask);
```

## DESCRIPTION

`epoll_wait()` waits for events on the epoll instance referred to by `epfd`.

### epoll_pwait()
Like `pselect(2)`, allows an application to safely wait until either a file
descriptor becomes ready or until a signal is caught.

The `sigmask` argument may be specified as NULL, in which case `epoll_pwait()`
is equivalent to `epoll_wait()`.

### epoll_pwait2()
Equivalent to `epoll_pwait()` except the timeout argument is `struct timespec`
(nanosecond resolution). If `timeout` is NULL, `epoll_pwait2()` can block
indefinitely.

## RETURN VALUE
On success, returns the number of file descriptors ready, or zero on timeout.
On failure, returns -1 and sets errno.

## ERRORS
- `EBADF`: epfd is not a valid file descriptor
- `EFAULT`: events memory not accessible with write permissions
- `EINTR`: interrupted by a signal handler
- `EINVAL`: epfd is not an epoll fd, or n <= 0

## C library/kernel differences (CRITICAL for sigsetsize)

> The raw `epoll_pwait()` and `epoll_pwait2()` system calls have a sixth
> argument, `size_t sigsetsize`, which specifies the size in bytes of the
> `sigmask` argument. The glibc `epoll_pwait()` wrapper function specifies
> this argument as a fixed value (equal to `sizeof(sigset_t)`).

**Note**: The man page says glibc passes `sizeof(sigset_t)` (= 128 on x86_64),
but the Linux kernel enforces `sigsetsize == _NSIG/8 == 8`. This is NOT a
contradiction: glibc's `sizeof(sigset_t)` in the wrapper context refers to
`sizeof(kernel_sigset_t)` = 8, not the userspace `sigset_t` = 128.
The glibc wrapper passes the kernel-side size, not the userspace type size.

## HISTORY
- `epoll_wait()`: Linux 2.6, glibc 2.3.2
- `epoll_pwait()`: Linux 2.6.19, glibc 2.6
- `epoll_pwait2()`: Linux 5.11
