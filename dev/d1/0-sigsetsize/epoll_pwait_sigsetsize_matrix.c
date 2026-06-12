#define _GNU_SOURCE

#include <errno.h>
#include <signal.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <sys/epoll.h>
#include <sys/syscall.h>
#include <unistd.h>

static long raw_epoll_pwait(int epfd, struct epoll_event *events, int maxevents,
                            int timeout, const sigset_t *sigmask, size_t sigsetsize)
{
    return syscall(SYS_epoll_pwait, epfd, events, maxevents, timeout, sigmask, sigsetsize);
}

int main(void)
{
    static const size_t sizes[] = {0, 1, 7, 8, 9, 16, 17, 32, sizeof(sigset_t)};

    int epfd = epoll_create1(0);
    if (epfd < 0) {
        perror("epoll_create1");
        return 1;
    }

    struct epoll_event events[1];
    memset(events, 0, sizeof(events));

    sigset_t mask;
    sigemptyset(&mask);
    sigaddset(&mask, SIGUSR1);

    printf("epoll_pwait raw syscall sigsetsize matrix\n");
    printf("sizeof(sigset_t)=%zu\n", sizeof(sigset_t));
    printf("columns: size | ret_nonnull errno_nonnull | ret_null errno_null\n");

    for (size_t i = 0; i < sizeof(sizes) / sizeof(sizes[0]); i++) {
        size_t sz = sizes[i];

        errno = 0;
        long r_nonnull = raw_epoll_pwait(epfd, events, 1, 0, &mask, sz);
        int e_nonnull = errno;

        errno = 0;
        long r_null = raw_epoll_pwait(epfd, events, 1, 0, NULL, sz);
        int e_null = errno;

        printf("%4zu | %11ld %13d (%s) | %8ld %9d (%s)\n",
               sz,
               r_nonnull,
               e_nonnull,
               (e_nonnull == 0 ? "OK" : strerror(e_nonnull)),
               r_null,
               e_null,
               (e_null == 0 ? "OK" : strerror(e_null)));
    }

    close(epfd);
    return 0;
}
