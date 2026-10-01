/*
 * wifi-ctl — StarryOS (SG2002 aic8800) Wi-Fi mode control tool.
 *
 * Switches the wireless interface between Station (STA) and SoftAP (AP) at
 * runtime through the Linux wireless-extensions ioctls implemented by
 * StarryOS (`os/StarryOS/kernel/src/file/wext.rs`).
 *
 * The kernel side uses stage-then-commit semantics: SIOCSIWMODE/ESSID/
 * ENCODEEXT/FREQ only stage into a per-interface pending config, and
 * SIOCSIWCOMMIT atomically applies the whole transition (link-layer
 * teardown + switch + IP/DHCP role).
 *
 * Known limitation: WPA2 station mode (a non-empty passphrase) is rejected
 * by the driver until a kernel entropy source is wired to the wireless
 * commit path (the commit currently always submits `entropy: None`).
 * Open networks and access-point mode work today.
 *
 * Usage:
 *   wifi-ctl <ifname> sta <ssid>            # open network
 *   wifi-ctl <ifname> ap  <ssid> [channel]  # channel defaults to 6
 *
 * <ifname> is the network-stack interface name (ax_net side, e.g. "eth0"),
 * not the driver registration name ("wlan0"). Run `ip addr` to list them.
 *
 * Example:
 *   wifi-ctl eth0 ap SG2002 6
 *   wifi-ctl eth0 sta OpenNet
 */

#include <arpa/inet.h>
#include <errno.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>
#include <sys/socket.h>
#include <unistd.h>

/* Fixed values from <linux/wireless.h> (header may be absent in the musl
 * cross sysroot, and the kernel matches these exact constants). */
#define SIOCSIWCOMMIT 0x8B00
#define SIOCSIWFREQ 0x8B04
#define SIOCSIWMODE 0x8B06
#define SIOCSIWESSID 0x8B1A
#define SIOCSIWENCODEEXT 0x8B34

#define IW_MODE_INFRA 2 /* Managed / Station */
#define IW_MODE_MASTER 3 /* Master / Access Point */

#define IW_ESSID_MAX_SIZE 32
#define MAX_PASSPHRASE 63
#define MIN_PASSPHRASE 8 /* WPA2-PSK lower bound */

/* `struct iwreq` is 32 bytes on both 32/64-bit targets: a 16-byte ifrn_name
 * union followed by a 16-byte `union iwreq_data`. The kernel parses:
 *   - mode / freq:   first u32 of the data union
 *   - ssid / key:    iw_point { ptr, len(u16), flags(u16) } at the data union
 *   - commit:        name only
 */
struct iw_point {
    void *pointer;
    unsigned short length;
    unsigned short flags;
};

struct iwreq {
    char ifr_name[16];
    union {
        unsigned int mode;
        struct iw_point point;
        char data[16];
    } u;
};

/* Compile-time pin of the kernel ABI layout (wext.rs parses raw bytes at
 * these offsets; any struct change would silently break the protocol). */
_Static_assert(sizeof(struct iwreq) == 32, "iwreq must stay 32 bytes");
_Static_assert(sizeof(((struct iwreq *)0)->u.point.pointer) == 8,
               "iw_point pointer must stay 8 bytes on LP64");

/* Local-stage failure (already reported); ioctl failure keeps errno for
 * the caller's perror. */
#define R_IOCTL (-1)
#define R_USAGE (-2)

static int wifi_socket(void) {
    int fd = socket(AF_INET, SOCK_DGRAM, 0);
    if (fd < 0) {
        perror("socket");
        return -1;
    }
    return fd;
}

static int stage_mode(int fd, const struct iwreq *req, unsigned int mode) {
    struct iwreq r = *req;
    r.u.mode = mode;
    return ioctl(fd, SIOCSIWMODE, &r);
}

static int stage_ssid(int fd, const struct iwreq *req, const char *ssid) {
    size_t len = strlen(ssid);
    if (len == 0 || len > IW_ESSID_MAX_SIZE) {
        fprintf(stderr, "ssid length must be 1..%d\n", IW_ESSID_MAX_SIZE);
        return R_USAGE;
    }
    struct iwreq r = *req;
    r.u.point.pointer = (void *)ssid;
    r.u.point.length = (unsigned short)len;
    /* Linux WEXT semantics (and the dev kernel's wext.rs): flags != 0
     * means "set this SSID"; flags == 0 means "clear it". iwconfig sends
     * flags = 1 for SIOCSIWESSID; sending 0 stages no SSID and the commit
     * fails with EINVAL. */
    r.u.point.flags = 1;
    return ioctl(fd, SIOCSIWESSID, &r);
}

static int stage_passphrase(int fd, const struct iwreq *req, const char *pass) {
    size_t len = strlen(pass);
    if (len < MIN_PASSPHRASE || len > MAX_PASSPHRASE) {
        fprintf(stderr, "passphrase must be %d..%d bytes (WPA2-PSK)\n",
                MIN_PASSPHRASE, MAX_PASSPHRASE);
        return R_USAGE;
    }
    struct iwreq r = *req;
    r.u.point.pointer = (void *)pass;
    r.u.point.length = (unsigned short)len;
    r.u.point.flags = 0;
    return ioctl(fd, SIOCSIWENCODEEXT, &r);
}

static int stage_channel(int fd, const struct iwreq *req, unsigned int chan) {
    if (chan == 0 || chan > 14) {
        fprintf(stderr, "channel must be 1..14\n");
        return R_USAGE;
    }
    struct iwreq r = *req;
    r.u.mode = chan; /* first u32 of iwreq_data carries the channel */
    return ioctl(fd, SIOCSIWFREQ, &r);
}

/* Commits with a few EBUSY retries: the driver control queue can be full
 * briefly while a previous transaction settles. */
static int commit_with_retry(int fd, const struct iwreq *req) {
    for (int attempt = 0; attempt < 3; attempt++) {
        if (ioctl(fd, SIOCSIWCOMMIT, req) == 0)
            return 0;
        if (errno != EBUSY)
            return R_IOCTL;
        usleep(100 * 1000);
    }
    return R_IOCTL;
}

static int parse_channel(const char *arg, unsigned int *out) {
    char *end = NULL;
    errno = 0;
    unsigned long value = strtoul(arg, &end, 10);
    if (errno != 0 || end == arg || *end != '\0' || value == 0 || value > 14) {
        fprintf(stderr, "invalid channel '%s' (must be 1..14)\n", arg);
        return R_USAGE;
    }
    *out = (unsigned int)value;
    return 0;
}

static int run(const char *ifname, unsigned int mode, const char *ssid,
               const char *passphrase, unsigned int channel) {
    if (strlen(ifname) >= 16) {
        fprintf(stderr, "interface name too long\n");
        return R_USAGE;
    }

    int fd = wifi_socket();
    if (fd < 0)
        return R_IOCTL;

    struct iwreq req;
    memset(&req, 0, sizeof(req));
    strncpy(req.ifr_name, ifname, sizeof(req.ifr_name) - 1);

    int result;
    if ((result = stage_mode(fd, &req, mode)) < 0)
        goto out;
    if ((result = stage_ssid(fd, &req, ssid)) < 0)
        goto out;
    if (mode == IW_MODE_INFRA && passphrase[0] != '\0') {
        if ((result = stage_passphrase(fd, &req, passphrase)) < 0)
            goto out;
    }
    if (mode == IW_MODE_MASTER) {
        if ((result = stage_channel(fd, &req, channel)) < 0)
            goto out;
    }
    result = commit_with_retry(fd, &req);
    if (result == R_IOCTL && errno == ENODEV) {
        fprintf(stderr,
                "unknown interface '%s': run 'ip addr' to list interfaces\n",
                ifname);
    }

out:
    if (result < 0 && result == R_IOCTL)
        perror("ioctl");
    close(fd);
    if (result == 0)
        printf("%s: switched to %s ssid=%s\n", ifname,
               mode == IW_MODE_INFRA ? "station" : "access-point", ssid);
    return result;
}

int main(int argc, char **argv) {
    if (argc < 4) {
        fprintf(stderr,
                "usage:\n"
                "  %s <ifname> sta <ssid>            # open network\n"
                "  %s <ifname> ap  <ssid> [channel]  # channel defaults to 6\n",
                argv[0], argv[0]);
        return 2;
    }
    const char *ifname = argv[1];
    const char *subcmd = argv[2];
    const char *ssid = argv[3];

    if (strcmp(subcmd, "sta") == 0) {
        const char *pass = argc >= 5 ? argv[4] : "";
        if (pass[0] != '\0') {
            fprintf(stderr,
                    "WPA2 station mode is not available yet: the kernel "
                    "wireless commit path does not supply WPA entropy. Use an "
                    "open network, or wire a kernel entropy source first.\n");
            return 1;
        }
        return run(ifname, IW_MODE_INFRA, ssid, pass, 0);
    }
    if (strcmp(subcmd, "ap") == 0) {
        unsigned int channel = 6;
        if (argc >= 5) {
            int result = parse_channel(argv[4], &channel);
            if (result != 0)
                return 1;
        }
        return run(ifname, IW_MODE_MASTER, ssid, "", channel);
    }
    fprintf(stderr, "unknown subcommand '%s' (expected 'sta' or 'ap')\n", subcmd);
    return 2;
}
