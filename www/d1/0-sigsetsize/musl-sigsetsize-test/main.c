#define _GNU_SOURCE

#include <errno.h>
#include <poll.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/epoll.h>
#include <sys/signalfd.h>
#include <time.h>
#include <unistd.h>

static int fail_count = 0;

static void report_call(const char *name, int ret) {
    if (ret >= 0) {
        printf("[PASS] %s ret=%d\n", name, ret);
        return;
    }

    if (errno == EINVAL) {
        printf("[FAIL] %s ret=-1 errno=EINVAL (unexpected for libc wrapper)\n", name);
        fail_count++;
        return;
    }

    printf("[PASS] %s ret=-1 errno=%d (%s)\n", name, errno, strerror(errno));
}

static void test_epoll_pwait(void) {
    int epfd = epoll_create1(0);
    if (epfd < 0) {
        perror("epoll_create1");
        exit(1);
    }

    sigset_t mask;
    sigemptyset(&mask);
    sigaddset(&mask, SIGUSR1);

    struct epoll_event events[1];

    errno = 0;
    report_call("epoll_pwait(sigmask!=NULL)",
                epoll_pwait(epfd, events, 1, 0, &mask));

    close(epfd);
}

static void test_pselect_ppoll(void) {
    sigset_t mask;
    sigemptyset(&mask);
    sigaddset(&mask, SIGUSR1);

    struct timespec ts = {.tv_sec = 0, .tv_nsec = 0};

    fd_set rfds;
    FD_ZERO(&rfds);

    errno = 0;
    report_call("pselect(sigmask!=NULL)",
                pselect(0, &rfds, NULL, NULL, &ts, &mask));

    errno = 0;
    report_call("ppoll(sigmask!=NULL)",
                ppoll(NULL, 0, &ts, &mask));
}

static void test_signalfd4(void) {
    sigset_t mask;
    sigemptyset(&mask);
    sigaddset(&mask, SIGUSR1);

    errno = 0;
    int fd = signalfd(-1, &mask, SFD_CLOEXEC | SFD_NONBLOCK);
    if (fd >= 0) {
        printf("[PASS] signalfd(sigmask!=NULL) ret=%d\n", fd);
        close(fd);
        return;
    }

    if (errno == EINVAL) {
        printf("[FAIL] signalfd(sigmask!=NULL) ret=-1 errno=EINVAL (unexpected for libc wrapper)\n");
        fail_count++;
        return;
    }

    printf("[PASS] signalfd(sigmask!=NULL) ret=-1 errno=%d (%s)\n", errno, strerror(errno));
}

int main(void) {
    printf("=== libc wrapper sigsetsize behavior test ===\n");
    printf("Goal: replay raw-syscall failing nodes with libc APIs only.\n");
    printf("Note: epoll_pwait2 is excluded because musl has no wrapper.\n");

    test_epoll_pwait();
    test_pselect_ppoll();
    test_signalfd4();

    if (fail_count > 0) {
        printf("RESULT: FAIL (%d libc calls returned EINVAL)\n", fail_count);
        return 1;
    }

    printf("RESULT: PASS (no unexpected EINVAL from libc wrappers)\n");
    return 0;
}
