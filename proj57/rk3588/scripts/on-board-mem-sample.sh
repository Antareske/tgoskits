#!/bin/sh
# External memory-footprint sampler for the ACT RKNN inference run.
#
# Rationale: StarryOS may not expose the same /proc fields or monitoring tools
# as Linux. This script samples the inference process RSS while it runs and
# prints the peak, using only the most portable mechanisms available, trying
# several fallbacks in order:
#   1. /proc/<pid>/status  VmHWM / VmRSS  (Linux and many minimal kernels)
#   2. /proc/<pid>/statm   resident pages * page size
#   3. busybox/ps RSS column
#
# Usage: on-board-mem-sample.sh <run-script> [args...]
#   e.g. on-board-mem-sample.sh /act_infer_rk3588/run-review.sh left
set -eu

if [ "$#" -lt 1 ]; then
    echo "usage: $0 <command> [args...]" >&2
    exit 2
fi

page_kb=4  # default page size assumption (4 KiB); refined below if getconf works
if command -v getconf >/dev/null 2>&1; then
    psz="$(getconf PAGESIZE 2>/dev/null || echo 4096)"
    page_kb=$((psz / 1024))
    [ "${page_kb}" -gt 0 ] || page_kb=4
fi

"$@" &
pid=$!

peak_kb=0
read_rss_kb() {
    # 1. /proc/<pid>/status
    if [ -r "/proc/${pid}/status" ]; then
        v="$(awk '/^VmHWM:/{print $2; f=1} END{if(!f) print ""}' "/proc/${pid}/status" 2>/dev/null || true)"
        [ -z "${v}" ] && v="$(awk '/^VmRSS:/{print $2}' "/proc/${pid}/status" 2>/dev/null || true)"
        if [ -n "${v}" ]; then echo "${v}"; return; fi
    fi
    # 2. /proc/<pid>/statm  (field 2 = resident pages)
    if [ -r "/proc/${pid}/statm" ]; then
        pages="$(awk '{print $2}' "/proc/${pid}/statm" 2>/dev/null || true)"
        if [ -n "${pages}" ]; then echo "$((pages * page_kb))"; return; fi
    fi
    # 3. ps RSS column (KiB)
    if command -v ps >/dev/null 2>&1; then
        v="$(ps -o rss= -p "${pid}" 2>/dev/null | tr -d ' ' || true)"
        if [ -n "${v}" ]; then echo "${v}"; return; fi
    fi
    echo ""
}

while kill -0 "${pid}" 2>/dev/null; do
    cur="$(read_rss_kb)"
    if [ -n "${cur}" ] && [ "${cur}" -gt "${peak_kb}" ] 2>/dev/null; then
        peak_kb="${cur}"
    fi
    sleep 0.05 2>/dev/null || sleep 1
done

wait "${pid}"
rc=$?
echo "ACT_PEAK_RSS_KB=${peak_kb}"
exit "${rc}"
