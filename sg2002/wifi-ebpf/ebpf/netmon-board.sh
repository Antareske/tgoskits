#!/bin/sh
# Board-side run of the network observation line. Runs ON the board, from a
# file, so a mangled paste cannot corrupt it.
#
# Two ways to supply the load, both giving the same three numbers so the
# configurations stay comparable:
#
#   NETMON_IPERF_PEER=192.168.137.1 sh /usr/bin/netmon-board.sh
#     the board runs iperf3 against that peer and the numbers come from iperf3
#     itself. This is the way the board is normally driven.
#
#   NETMON_LOAD='wget -q -O - http://192.168.137.1:8000/blob' \
#     sh /usr/bin/netmon-board.sh
#     for anything that streams its payload to stdout.
#
# Four groups, because they answer different questions:
#
#   off       no monitor running; this is the throughput acceptance figure
#   observe   monitor attached, sample rate 0 (nothing carries a stamp)
#   sampled   monitor attached, sample rate 16
#   off2      the acceptance figure again, last
#
# The latency distributions from `observe` and `sampled` are for the second and
# third questions; only `off` answers throughput. `off2` exists because this
# board's throughput has been seen to fall over successive runs on its own: a
# drop between `off` and the monitored groups can only be read as the cost of
# observing if `off2` comes back to where `off` was.
#
# A group that fails is recorded and the remaining ones still run, and the
# collected output is printed either way: a board session is expensive to
# repeat, and one bad group must not cost the rest.

set -u

NETMON=${NETMON:-/usr/bin/netmon}
LOAD=${NETMON_LOAD:-}
IPERF_PEER=${NETMON_IPERF_PEER:-}
IPERF_SECONDS=${NETMON_IPERF_SECONDS:-20}
IPERF_WARMUP=${NETMON_IPERF_WARMUP:-3}
RATE=${NETMON_RATE:-16}
KNOB=/sys/kernel/debug/net_sample_rate
QUEUE=/sys/kernel/debug/net_queue
OUT=/tmp/netmon-board
# Below this the run is too short to compare against another configuration.
MIN_SECONDS=5

# Failures and progress go to stderr: the collected output is what goes to
# stdout, and a message in the middle of it would have to be filtered out.
fail() {
    echo "NETMON_BOARD_FAILED: $1" >&2
    exit 1
}

[ -n "$LOAD" ] || [ -n "$IPERF_PEER" ] || \
    fail "set NETMON_IPERF_PEER (or NETMON_LOAD for a stdout streamer)"
[ -x "$NETMON" ] || fail "$NETMON is not executable"
[ -r "$KNOB" ] || fail "$KNOB is missing; the observation surface is not in this image"
[ -w "$KNOB" ] || fail "$KNOB is not writable; are you root?"
if [ -n "$IPERF_PEER" ]; then
    command -v iperf3 >/dev/null 2>&1 || fail "iperf3 is not in this image"
fi

echo "netmon-board: monitor=$NETMON rate=$RATE" >&2
if [ -n "$IPERF_PEER" ]; then
    echo "netmon-board: peer=$IPERF_PEER iperf3 -t $IPERF_SECONDS -O $IPERF_WARMUP" >&2
else
    echo "netmon-board: load=$LOAD" >&2
fi

# The knob takes powers of two, and 0 means no frame carries a stamp. A value
# it refuses comes back as an error rather than as a silent default.
set_rate() {
    echo "$1" > "$KNOB" || fail "could not set rate $1"
    got=$(cat "$KNOB")
    [ "$got" = "$1" ] || fail "asked for rate $1, read back $got"
}

# One iperf3 run, with the client in the background.
#
# The client is known to wedge this board when it is pushed (their notes have
# -P4 and UDP hanging it), and there is no `timeout` in busybox, so the run is
# bounded here.
#
# What is waited for is the client's own `sender` summary, not the `receiver`
# line: that summary is printed before the two ends exchange results, and the
# exchange fails on this board often enough (`unable to receive results`) that
# waiting on it would throw away runs whose transfer had already finished. The
# rate it carries is what the client pushed; the peer's own account is reported
# alongside whenever the exchange does work out.
run_iperf() {
    out=$1
    : > "$out"
    iperf3 -c "$IPERF_PEER" -t "$IPERF_SECONDS" -O "$IPERF_WARMUP" > "$out" 2>&1 &
    iperf_pid=$!
    waited=0
    limit=$((IPERF_SECONDS + IPERF_WARMUP + 30))
    while ! grep -q 'sender$' "$out" 2>/dev/null; do
        if [ "$waited" -ge "$limit" ]; then
            kill "$iperf_pid" 2>/dev/null
            echo "netmon-board: iperf3 produced no summary in ${limit}s; last output:" >&2
            sed 's/^/  /' "$out" >&2
            return 1
        fi
        sleep 1
        waited=$((waited + 1))
    done
    # It may still be exchanging results; that part is not this run's to wait
    # for, and a client that never returns must not hold the console.
    grace=10
    while kill -0 "$iperf_pid" 2>/dev/null && [ "$grace" -gt 0 ]; do
        sleep 1
        grace=$((grace - 1))
    done
    kill -0 "$iperf_pid" 2>/dev/null && kill "$iperf_pid" 2>/dev/null
    wait "$iperf_pid" 2>/dev/null
    return 0
}

# The numbers out of iperf3's `sender` summary, in the order the caller wants
# them: bytes the board sent, the measured seconds, kbit/s (1 kbit = 1000
# bits), and the peer's kbit/s when the results exchange completed, so a lossy
# link shows up as a gap between the last two.
#
# The fields are read from the "sec" token onwards rather than by position: the
# leading `[  5]` splits into two fields, so counting from the left would read
# the wrong ones. `Mbits/sec` and `MBytes` are the default formatting; nothing
# here asks for another one.
parse_iperf() {
    awk '
        {
            if ($0 ~ /omitted/) next
            sender = ($0 ~ /sender$/)
            receiver = ($0 ~ /receiver$/)
            if (!sender && !receiver) next
            for (i = 1; i <= NF; i++) {
                if ($i != "sec") continue
                interval = $(i - 1)
                if (sender) { sb = $(i + 1); su = $(i + 2); sr = $(i + 3); sru = $(i + 4); ss = interval }
                else        { rr = $(i + 3); rru = $(i + 4) }
                break
            }
        }
        function mul(u) { return (u ~ /^K/) ? 1024 : (u ~ /^M/) ? 1048576 : \
                                (u ~ /^G/) ? 1073741824 : 1 }
        function rate(u) { return (u ~ /^K/) ? 1000 : (u ~ /^M/) ? 1000000 : \
                                  (u ~ /^G/) ? 1000000000 : 1 }
        function secs(iv, t) { split(iv, t, "-"); return int(t[2] + 0.5) }
        END {
            if (sb == "") { print "0 0 0 0"; exit }
            kbps = int(sr * rate(sru) / 1000)
            if (rr == "") { printf "%d %d %d 0\n", sb * mul(su), secs(ss), kbps; exit }
            printf "%d %d %d %d\n", sb * mul(su), secs(ss), kbps, rr * rate(rru) / 1000
        }
    ' "$1"
}

# One measured run. The numbers go to stdout as they are measured, so that a
# later group's failure cannot take them away.
failed=
measure() {
    label=$1
    echo "netmon-board: measuring $label" >&2
    bytes=0
    elapsed=0
    kbps=0
    peer_kbps=0
    if [ -n "$IPERF_PEER" ]; then
        # One retry: the board's iperf3 client has a history of wedging without
        # printing a summary, and a group lost that way leaves the whole round
        # without its control. The retry is announced, and a group that fails
        # twice is still recorded as failed rather than retried again.
        attempt=1
        while :; do
            if run_iperf "$OUT/$label.iperf3"; then
                break
            fi
            if [ "$attempt" -ge 2 ]; then
                break
            fi
            attempt=$((attempt + 1))
            echo "netmon-board: $label: iperf3 produced no summary; retrying once" >&2
        done
        if [ -s "$OUT/$label.iperf3" ] && grep -qE 'sender$' "$OUT/$label.iperf3"; then
            set -- $(parse_iperf "$OUT/$label.iperf3")
            bytes=$1
            elapsed=$2
            kbps=$3
            peer_kbps=$4
            grep -E 'sender$|receiver$' "$OUT/$label.iperf3" | sed 's/^/  /' >&2
        fi
    else
        start=$(date +%s)
        bytes=$($LOAD | wc -c)
        end=$(date +%s)
        elapsed=$((end - start))
        [ "$elapsed" -gt 0 ] || elapsed=1
        kbps=$((bytes / elapsed / 125))
    fi
    if [ "$bytes" -le 0 ]; then
        failed="$failed $label"
        echo "$label bytes=0 seconds=$elapsed kbit_per_s=0" | tee "$OUT/$label.txt"
        return 0
    fi
    {
        echo "$label bytes=$bytes seconds=$elapsed kbit_per_s=$kbps"
        [ "$peer_kbps" -gt 0 ] && echo "$label peer_kbit_per_s=$peer_kbps"
    } | tee "$OUT/$label.txt"
    [ "$elapsed" -ge "$MIN_SECONDS" ] || \
        echo "netmon-board: warning: $label ran ${elapsed}s, under ${MIN_SECONDS}s" >&2
    return 0
}

# A monitored run: netmon attached for the length of the load.
#
# The monitor's pid lives in a name of its own: the load runs in the background
# too, and a shared name once left a monitor attached into the next group,
# which reads as the observation path costing twice what it does.
#
# Stopping it is verified rather than assumed. INT is what it exits on by
# itself, but a signal can be ignored in ways the process cannot undo, and a
# monitor that outlives its group corrupts every later group's cost.
monitored() {
    label=$1
    rate=$2
    # The sampled group also prints the flow table: the board's load is one
    # iperf3 connection, so the table should hold exactly that flow and the
    # bytes it carried.
    flows=${3:-0}
    set_rate "$rate"
    echo "netmon-board: configuration $label rate=$rate" >&2
    $NETMON --interval 1 --flows "$flows" > "$OUT/$label.mon" 2>&1 &
    mon_pid=$!
    sleep 1
    measure "$label"
    kill -INT "$mon_pid" 2>/dev/null
    waited=0
    while kill -0 "$mon_pid" 2>/dev/null && [ "$waited" -lt 5 ]; do
        sleep 1
        waited=$((waited + 1))
    done
    if kill -0 "$mon_pid" 2>/dev/null; then
        echo "netmon-board: monitor for $label did not stop on INT; sending TERM" >&2
        kill "$mon_pid" 2>/dev/null
        sleep 1
        kill -KILL "$mon_pid" 2>/dev/null
    fi
    wait "$mon_pid" 2>/dev/null
}

rm -rf "$OUT"
mkdir -p "$OUT"

# 1. Throughput with nothing observing. This is the number that goes in a
#    comparison against an unmodified system.
echo "netmon-board: configuration off" >&2
measure off

# 2. The observation path with no stamping: what the reporting itself costs.
monitored observe 0

# 3. Stamping one frame in $RATE: the configuration the interval distributions
#    are meaningful in.
monitored sampled "$RATE" 8

# 4. The acceptance figure once more: see the header for why it is here.
echo "netmon-board: configuration off2 (control)" >&2
measure off2
set_rate 0

# The board's load is a single iperf3 connection, so the flow table should hold
# it with the bytes iperf3 moved. The table accumulates for the length of the
# monitor's run, so the last snapshot's rows are the whole run's; a floor well
# under the smallest throughput the board has shown still separates "the events
# reached the map and the map reached userspace" from an empty table.
#
# The total is summed over the flow rows of the last snapshot only: the monitor
# prints the table once per interval, and adding every snapshot together would
# count the same bytes as many times as there were intervals.
flow_bytes() {
    awk '/^flows=/{total = 0} /^flow /{
        for (i = 1; i <= NF; i++) {
            split($i, kv, "=")
            if (kv[1] == "tx_bytes" || kv[1] == "rx_bytes") { total += kv[2] }
        }
    } END { print total + 0 }' "$1"
}
# The table is printed once per interval and accumulates over the run, so the
# last snapshot's first row is the run's busiest flow; the first row in the file
# is the first snapshot's, taken while the load was still starting.
busiest_flow() {
    awk '/^flows=/{first = ""} /^flow /{if (first == "") first = $0} END {print first}' "$1"
}
echo "netmon-board: busiest flow: $(busiest_flow "$OUT/sampled.mon")"
if [ "$(flow_bytes "$OUT/sampled.mon")" -lt 65536 ]; then
    echo "netmon-board: the flow table holds no iperf3 traffic" >&2
    failed="${failed:+$failed,}flows"
fi

echo "--- throughput ---"
for label in off observe sampled off2; do
    [ -f "$OUT/$label.txt" ] && cat "$OUT/$label.txt"
done

echo "--- queue counters ---"
cat "$QUEUE"

echo "--- monitor, observe ---"
cat "$OUT/observe.mon" 2>/dev/null

echo "--- monitor, sampled ---"
cat "$OUT/sampled.mon" 2>/dev/null

if [ -n "$failed" ]; then
    echo "NETMON_BOARD_GROUPS_FAILED:$failed" >&2
fi
echo "NETMON_BOARD_END"
[ -z "$failed" ]
