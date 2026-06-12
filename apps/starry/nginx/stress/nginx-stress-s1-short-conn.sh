#!/bin/sh
set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
if [ -r /usr/bin/nginx-alpine-mirror.sh ]; then
    . /usr/bin/nginx-alpine-mirror.sh
elif [ -r "$SCRIPT_DIR/../nginx-alpine-mirror.sh" ]; then
    . "$SCRIPT_DIR/../nginx-alpine-mirror.sh"
fi

ID=S1
BASE=/tmp/nginx-stress-s1
CONF="$BASE/conf/short-conn.conf"
WWW="$BASE/www"
OUT="$BASE/out"
LOGDIR="$BASE/logs"
TIMEOUT_CMD=
WATCHDOG_PID=
MASTER_PID=
REQUEST_TIMEOUT_SEC=30
WATCHDOG_TIMEOUT_SEC=3600
POST_HEALTH_TIMEOUT_SEC=20
REQUEST_GAP_SEC=0.2
REQUEST_ATTEMPTS=3
REQUEST_RETRY_GAP_SEC=1

log() { printf 'NGINX_STRESS_%s_LOG: %s\n' "$ID" "$*"; }
pass() { printf 'NGINX_STRESS_%s_TEST_PASSED\n' "$ID"; }
fail() { printf 'NGINX_STRESS_%s_TEST_FAILED\n' "$ID"; log "$*"; exit 1; }

init_timeout_cmd() {
    if command -v timeout >/dev/null 2>&1; then TIMEOUT_CMD='timeout'; return; fi
    if busybox timeout 2>&1 | grep -qi 'usage'; then TIMEOUT_CMD='busybox timeout'; return; fi
    fail "timeout command not available"
}

run_with_timeout() { sec=$1; shift; $TIMEOUT_CMD "$sec" "$@"; }

cleanup_nginx() {
    nginx -s quit -c "$CONF" -p "$BASE/" >/dev/null 2>&1 || true
    sleep 1
    killall -q nginx 2>/dev/null || true
    sleep 1
    killall -q -9 nginx 2>/dev/null || true
}

cleanup_all() {
    cleanup_nginx
    if [ -n "$WATCHDOG_PID" ]; then kill "$WATCHDOG_PID" 2>/dev/null || true; fi
}

prepare_packages() {
    if command -v nginx >/dev/null 2>&1 && command -v curl >/dev/null 2>&1; then
        log "packages=present nginx_version=$(nginx -v 2>&1)"
        return 0
    fi
    if command -v nginx_apk_add_with_fallback >/dev/null 2>&1; then
        nginx_apk_add_with_fallback nginx curl busybox-extras || return 1
        log "packages=installed nginx_version=$(nginx -v 2>&1)"
        return 0
    fi
    return 1
}

prepare_tree() {
    rm -rf "$BASE"
    mkdir -p "$BASE/conf" "$WWW" "$OUT" "$LOGDIR"
    printf 'nginx stress s1 short connection\n' > "$WWW/small.txt"
    cat > "$CONF" <<'EOF'
daemon off;
master_process on;
worker_processes 1;
error_log /tmp/nginx-stress-s1/logs/error.log debug;
pid /tmp/nginx-stress-s1/nginx.pid;
events { worker_connections 256; }
http {
    include /etc/nginx/mime.types;
    access_log /tmp/nginx-stress-s1/logs/access.log;
    sendfile off;
    keepalive_timeout 0;
    server {
        listen 127.0.0.1:8080;
        root /tmp/nginx-stress-s1/www;
        location / { index index.html; }
    }
}
EOF
}

start_nginx() {
    nginx -t -c "$CONF" -p "$BASE/" || return 1
    nginx -c "$CONF" -p "$BASE/" > "$LOGDIR/nginx-stdout.log" 2>&1 &
    i=0
    while [ "$i" -lt 8 ]; do
        if run_with_timeout 2 curl -fsS -o "$OUT/health.body" http://127.0.0.1:8080/small.txt >/dev/null 2>&1; then
            grep -qx 'nginx stress s1 short connection' "$OUT/health.body" && return 0
        fi
        i=$((i + 1))
        sleep 1
    done
    return 1
}

run_worker() {
    worker_id=$1
    total=$2
    concurrency=$3
    idx=$worker_id
    while [ "$idx" -le "$total" ]; do
        attempt=1
        while [ "$attempt" -le "$REQUEST_ATTEMPTS" ]; do
            if run_with_timeout "$REQUEST_TIMEOUT_SEC" curl -fsS -H 'Connection: close' -o "$OUT/request.$idx.body" -w '%{http_code}' http://127.0.0.1:8080/small.txt > "$OUT/request.$idx.status" 2> "$OUT/request.$idx.err" && grep -qx '200' "$OUT/request.$idx.status" && grep -qx 'nginx stress s1 short connection' "$OUT/request.$idx.body"; then
                break
            fi
            attempt=$((attempt + 1))
            if [ "$attempt" -le "$REQUEST_ATTEMPTS" ]; then
                sleep "$REQUEST_RETRY_GAP_SEC"
            fi
        done
        sleep "$REQUEST_GAP_SEC"
        idx=$((idx + concurrency))
    done
}

run_short_connection_l1() {
    total=1000
    concurrency=8
    success=0
    failed=0
    fail_log="$OUT/failures.log"
    worker=1
    worker_pids=
    rm -f "$OUT"/request.*.status "$OUT"/request.*.body "$OUT"/request.*.err
    : > "$fail_log"

    while [ "$worker" -le "$concurrency" ]; do
        run_worker "$worker" "$total" "$concurrency" &
        worker_pids="$worker_pids $!"
        worker=$((worker + 1))
    done

    for pid in $worker_pids; do
        wait "$pid" || true
    done

    i=1
    while [ "$i" -le "$total" ]; do
        status_file="$OUT/request.$i.status"
        body_file="$OUT/request.$i.body"
        if [ -e "$status_file" ] && [ -e "$body_file" ] && grep -qx '200' "$status_file" && grep -qx 'nginx stress s1 short connection' "$body_file"; then
            success=$((success + 1))
        else
            failed=$((failed + 1))
            code=$(cat "$status_file" 2>/dev/null || printf 'missing')
            err=$(sed -n '1,2p' "$OUT/request.$i.err" 2>/dev/null | tr '\n' ' ')
            printf 'idx=%s code=%s err=%s\n' "$i" "$code" "$err" >> "$fail_log"
        fi
        i=$((i + 1))
    done

    log "level=L1 concurrency=$concurrency total=$total success=$success failed=$failed status_200=$success"
    if [ "$failed" -ne 0 ]; then
        log "fail_samples=$(sed -n '1,8p' "$fail_log" | tr '\n' ';')"
    fi
    [ "$success" -eq "$total" ] && [ "$failed" -eq 0 ]
}

post_health_check() {
    attempt=0
    while [ "$attempt" -lt "$POST_HEALTH_TIMEOUT_SEC" ]; do
        if run_with_timeout 5 curl -fsS -o "$OUT/post-health.body" http://127.0.0.1:8080/small.txt >/dev/null 2>&1 && grep -qx 'nginx stress s1 short connection' "$OUT/post-health.body"; then
            log "post_health_check=ok attempt=$((attempt + 1))"
            return 0
        fi
        attempt=$((attempt + 1))
        sleep 1
    done
    return 1
}

graceful_quit() {
    MASTER_PID=$(cat "$BASE/nginx.pid" 2>/dev/null || true)
    [ -n "$MASTER_PID" ] || return 1
    run_with_timeout 5 nginx -s quit -c "$CONF" -p "$BASE/" >/dev/null 2>&1 || return 1
    i=0
    while [ "$i" -lt 8 ]; do
        if ! kill -0 "$MASTER_PID" 2>/dev/null; then
            log "graceful_quit=ok"
            return 0
        fi
        sleep 1
        i=$((i + 1))
    done
    return 1
}

assert_no_zombie_or_residual() {
    if ps | grep '[n]ginx:' | grep -Eq ' Z |defunct'; then return 1; fi
    if ps | grep '[n]ginx:' >/dev/null 2>&1; then return 1; fi
    log "nginx_residual=none"
}

trap cleanup_all EXIT INT TERM
init_timeout_cmd
( sleep "$WATCHDOG_TIMEOUT_SEC"; log "watchdog timeout"; kill -TERM $$ ) &
WATCHDOG_PID=$!

cleanup_nginx
prepare_packages || fail "prepare packages"
prepare_tree || fail "prepare tree"
start_nginx || fail "start nginx"
run_short_connection_l1 || fail "short connection L1"
post_health_check || fail "post-stress health check"
graceful_quit || fail "graceful quit"
assert_no_zombie_or_residual || fail "nginx residual or zombie"
pass
