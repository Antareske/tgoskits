/*
 * Checks the net:queue_poll_round tracepoint end to end:
 *
 *   1. the event is discoverable and its format declares the documented
 *      fields;
 *   2. enabling it makes real queue rounds readable from the trace buffer,
 *      with an internally consistent record;
 *   3. disabling it stops new records while the same traffic continues.
 *
 * The event carries no network semantics beyond the queue runtime's own
 * facts, so the traffic below only has to produce poll rounds, not traffic
 * patterns.
 */

#define _GNU_SOURCE
#include <arpa/inet.h>
#include <netinet/in.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <unistd.h>

#define EVENT_DIR "/sys/kernel/debug/tracing/events/net/queue_poll_round"
#define TRACE_FILE "/sys/kernel/debug/tracing/trace"
/* A rendered record reads `queue_poll_round(discovery_order=0 ... outcome=0)`. */
#define MARKER "queue_poll_round("
#define MAX_TRACE_BYTES (256 * 1024)

/* The QEMU command line this suite runs under uses user-mode networking, whose
 * gateway answers ARP and IP, so a datagram sent to it always leaves the
 * interface.  Loopback traffic would not: it is injected into the protocol
 * receive path without touching a physical queue. */
#define GATEWAY_ADDR "10.0.2.2"
#define GATEWAY_PORT 9
#define TRAFFIC_FRAMES 16
#define TRAFFIC_ATTEMPTS 8
#define TRAFFIC_RETRY_US 20000
#define RECORD_WAIT_ATTEMPTS 20
#define RECORD_WAIT_US 50000
/* The same budget the record wait above is allowed, so the negative check
 * cannot pass merely by reading the buffer before the rounds finished. */
#define DISABLED_WAIT_US (RECORD_WAIT_ATTEMPTS * RECORD_WAIT_US)

static int failures;

static void fail(const char *message)
{
    printf("NET_EVENTS_FAIL: %s\n", message);
    failures++;
}

static long read_file(const char *path, char *buffer, size_t capacity)
{
    FILE *file = fopen(path, "r");
    if (file == NULL) {
        return -1;
    }
    size_t total = 0;
    size_t read;
    while (total + 1 < capacity &&
           (read = fread(buffer + total, 1, capacity - 1 - total, file)) > 0) {
        total += read;
    }
    fclose(file);
    buffer[total] = '\0';
    return (long)total;
}

static int write_file(const char *path, const char *text)
{
    FILE *file = fopen(path, "w");
    if (file == NULL) {
        return -1;
    }
    int result = fputs(text, file) >= 0 ? 0 : -1;
    if (fclose(file) != 0) {
        result = -1;
    }
    return result;
}

static int count_markers(const char *text)
{
    int count = 0;
    const char *cursor = text;
    while ((cursor = strstr(cursor, MARKER)) != NULL) {
        count++;
        cursor += sizeof(MARKER) - 1;
    }
    return count;
}

/* Sends datagrams off the box, which drives real transmit and receive rounds. */
static int send_traffic(void)
{
    int fd = socket(AF_INET, SOCK_DGRAM, 0);
    if (fd < 0) {
        return -1;
    }
    struct sockaddr_in peer;
    memset(&peer, 0, sizeof(peer));
    peer.sin_family = AF_INET;
    peer.sin_port = htons(GATEWAY_PORT);
    if (inet_pton(AF_INET, GATEWAY_ADDR, &peer.sin_addr) != 1) {
        close(fd);
        return -1;
    }

    unsigned char payload[32];
    memset(payload, 0x5a, sizeof(payload));
    int sent = 0;
    /* The first datagram can be refused while the gateway neighbor entry is
     * being resolved; the retries let that resolution complete. */
    for (int attempt = 0; attempt < TRAFFIC_ATTEMPTS && sent == 0; attempt++) {
        for (int index = 0; index < TRAFFIC_FRAMES; index++) {
            payload[0] = (unsigned char)index;
            ssize_t result = sendto(fd, payload, sizeof(payload), 0, (struct sockaddr *)&peer,
                                    sizeof(peer));
            if (result == (ssize_t)sizeof(payload)) {
                sent++;
            }
        }
        if (sent == 0) {
            usleep(TRAFFIC_RETRY_US);
        }
    }
    close(fd);
    return sent;
}

/* Reads the trace buffer, waiting briefly for records to be published. */
static long read_trace_waiting(char *buffer, size_t capacity, int *markers)
{
    long length = -1;
    for (int attempt = 0; attempt < RECORD_WAIT_ATTEMPTS; attempt++) {
        length = read_file(TRACE_FILE, buffer, capacity);
        if (length < 0) {
            return -1;
        }
        *markers = count_markers(buffer);
        if (*markers > 0) {
            break;
        }
        usleep(RECORD_WAIT_US);
    }
    return length;
}

/* Parses the first record and checks its internal consistency. */
static void check_record(const char *trace)
{
    const char *marker = strstr(trace, MARKER);
    if (marker == NULL) {
        fail("trace buffer holds no queue_poll_round record");
        return;
    }
    unsigned discovery_order = 0;
    unsigned group_id = 0;
    unsigned owner_cpu = 0;
    unsigned budget = 0;
    unsigned work_units = 0;
    unsigned outcome = 0;
    int fields = sscanf(marker + sizeof(MARKER) - 1,
                        "discovery_order=%u group_id=%u owner_cpu=%u budget=%u work_units=%u "
                        "outcome=%u",
                        &discovery_order, &group_id, &owner_cpu, &budget, &work_units, &outcome);
    if (fields != 6) {
        fail("queue_poll_round record does not carry the documented fields");
        return;
    }
    if (outcome > 3) {
        fail("queue_poll_round record reports an unknown outcome code");
    }
    if (work_units > budget) {
        fail("queue_poll_round record reports more work than its budget");
    }
    long online_cpus = sysconf(_SC_NPROCESSORS_ONLN);
    if (online_cpus > 0 && owner_cpu >= (unsigned long)online_cpus) {
        fail("queue_poll_round record reports an owner CPU outside the online set");
    }
    printf("NET_EVENTS_RECORD discovery_order=%u group_id=%u owner_cpu=%u budget=%u work_units=%u "
           "outcome=%u\n",
           discovery_order, group_id, owner_cpu, budget, work_units, outcome);
}

int main(void)
{
    char buffer[MAX_TRACE_BYTES];
    char format[8192];

    long id_length = read_file(EVENT_DIR "/id", buffer, sizeof(buffer));
    if (id_length <= 0 || strtol(buffer, NULL, 10) < 0) {
        fail("net:queue_poll_round has no readable id");
    }
    if (read_file(EVENT_DIR "/format", format, sizeof(format)) <= 0) {
        fail("net:queue_poll_round has no readable format");
    } else {
        static const char *const fields[] = {
            "discovery_order", "group_id", "owner_cpu", "budget", "work_units", "outcome",
        };
        for (size_t index = 0; index < sizeof(fields) / sizeof(fields[0]); index++) {
            if (strstr(format, fields[index]) == NULL) {
                printf("NET_EVENTS_FAIL: format lacks field %s\n", fields[index]);
                failures++;
            }
        }
        if (strstr(format, "net:queue_poll_round") == NULL &&
            strstr(format, "queue_poll_round") == NULL) {
            fail("format does not describe the queue_poll_round event");
        }
    }

    if (write_file(TRACE_FILE, "\n") != 0) {
        fail("trace buffer could not be cleared");
    }
    if (write_file(EVENT_DIR "/enable", "1") != 0) {
        fail("net:queue_poll_round could not be enabled");
        printf("NET_EVENTS_FAILED\n");
        return 1;
    }

    int sent = send_traffic();
    if (sent <= 0) {
        fail("no traffic could be sent to drive queue rounds");
    }

    int markers = 0;
    long trace_length = read_trace_waiting(buffer, sizeof(buffer), &markers);
    if (trace_length < 0) {
        fail("trace buffer is not readable");
    } else if (markers == 0) {
        fail("enabled event recorded no queue poll round");
    } else {
        printf("NET_EVENTS records=%d\n", markers);
        check_record(buffer);
    }

    if (write_file(EVENT_DIR "/enable", "0") != 0) {
        fail("net:queue_poll_round could not be disabled");
    }
    if (write_file(TRACE_FILE, "\n") != 0) {
        fail("trace buffer could not be cleared after disabling");
    }
    sent = send_traffic();
    if (sent <= 0) {
        fail("no traffic could be sent while the event was disabled");
    }
    usleep(DISABLED_WAIT_US);
    trace_length = read_file(TRACE_FILE, buffer, sizeof(buffer));
    if (trace_length < 0) {
        fail("trace buffer is not readable after disabling");
    } else if (count_markers(buffer) != 0) {
        fail("a disabled event still recorded queue poll rounds");
    }

    fflush(stdout);
    if (failures != 0) {
        puts("NET_EVENTS_FAILED");
        return 1;
    }
    puts("NET_EVENTS_PASSED");
    return 0;
}
