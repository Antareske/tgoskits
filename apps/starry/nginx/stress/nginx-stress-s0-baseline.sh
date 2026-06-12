#!/bin/sh
set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
if [ -r /usr/bin/nginx-alpine-mirror.sh ]; then
    . /usr/bin/nginx-alpine-mirror.sh
elif [ -r "$SCRIPT_DIR/../nginx-alpine-mirror.sh" ]; then
    . "$SCRIPT_DIR/../nginx-alpine-mirror.sh"
fi

ID=S0
BASE=/tmp/nginx-stress-s0
CONF="$BASE/conf/baseline.conf"
WWW="$BASE/www"
OUT="$BASE/out"
LOGDIR="$BASE/logs"
TIMEOUT_CMD=
WATCHDOG_PID=
MASTER_PID=

log() { printf 'NGINX_STRESS_%s_LOG: %s\n' "$ID" "$*"; }
pass() { printf 'NGINX_STRESS_%s_TEST_PASSED\n' "$ID"; }
fail() { printf 'NGINX_STRESS_%s_TEST_FAILED\n' "$ID"; log "$*"; exit 1; }

init_timeout_cmd() {
    if command -v timeout >/dev/null 2>&1; then
        TIMEOUT_CMD='timeout'
        return
    fi
    if busybox timeout 2>&1 | grep -qi 'usage'; then
        TIMEOUT_CMD='busybox timeout'
        return
    fi
    fail "timeout command not available"
}

run_with_timeout() {
    sec=$1
    shift
    $TIMEOUT_CMD "$sec" "$@"
}

cleanup_nginx() {
    nginx -s quit -c "$CONF" -p "$BASE/" >/dev/null 2>&1 || true
    sleep 1
    killall -q nginx 2>/dev/null || true
    sleep 1
    killall -q -9 nginx 2>/dev/null || true
}

cleanup_all() {
    cleanup_nginx
    if [ -n "$WATCHDOG_PID" ]; then
        kill "$WATCHDOG_PID" 2>/dev/null || true
    fi
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
    printf 'nginx stress s0 baseline\n' > "$WWW/small.txt"
    cat > "$CONF" <<'EOF'
daemon off;
master_process on;
worker_processes 1;
error_log /tmp/nginx-stress-s0/logs/error.log debug;
pid /tmp/nginx-stress-s0/nginx.pid;
events { worker_connections 64; }
http {
    include /etc/nginx/mime.types;
    access_log /tmp/nginx-stress-s0/logs/access.log;
    sendfile off;
    keepalive_timeout 0;
    server {
        listen 127.0.0.1:8080;
        root /tmp/nginx-stress-s0/www;
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
            grep -qx 'nginx stress s0 baseline' "$OUT/health.body" && return 0
        fi
        i=$((i + 1))
        sleep 1
    done
    return 1
}

run_short_connection_l0() {
    total=100
    concurrency=2
    success=0
    failed=0
    started=0
    batch=0
    rm -f "$OUT"/request.*.status

    while [ "$started" -lt "$total" ]; do
        batch=$((batch + 1))
        slot=0
        pids=
        while [ "$slot" -lt "$concurrency" ] && [ "$started" -lt "$total" ]; do
            started=$((started + 1))
            (
                if run_with_timeout 2 curl -fsS -H 'Connection: close' -o "$OUT/request.$started.body" -w '%{http_code}' http://127.0.0.1:8080/small.txt > "$OUT/request.$started.status" 2> "$OUT/request.$started.err"; then
                    grep -qx '200' "$OUT/request.$started.status" && grep -qx 'nginx stress s0 baseline' "$OUT/request.$started.body" || exit 1
                else
                    exit 1
                fi
            ) &
            pids="$pids $!"
            slot=$((slot + 1))
        done
        for pid in $pids; do
            wait "$pid" || true
        done
        for status_file in "$OUT"/request.*.status; do
            [ -e "$status_file" ] || continue
            body_file=${status_file%.status}.body
            if grep -qx '200' "$status_file" && grep -qx 'nginx stress s0 baseline' "$body_file"; then
                success=$((success + 1))
            else
                failed=$((failed + 1))
            fi
            rm -f "$status_file" "$body_file"
        done
        if [ "$success" -ne $((batch * concurrency)) ] && [ "$started" -lt "$total" ]; then
            failed=$((failed + total - started))
            break
        fi
    done

    log "level=L0 concurrency=$concurrency total=$total success=$success failed=$failed status_200=$success"
    [ "$success" -eq "$total" ] && [ "$failed" -eq 0 ]
}

post_health_check() {
    run_with_timeout 2 curl -fsS -o "$OUT/post-health.body" http://127.0.0.1:8080/small.txt >/dev/null 2>&1 || return 1
    grep -qx 'nginx stress s0 baseline' "$OUT/post-health.body"
}

graceful_quit() {
    MASTER_PID=$(cat "$BASE/nginx.pid" 2>/dev/null || true)
    [ -n "$MASTER_PID" ] || return 1
    run_with_timeout 5 nginx -s quit -c "$CONF" -p "$BASE/" >/dev/null 2>&1 || return 1
    i=0
    while [ "$i" -lt 6 ]; do
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
    if ps | grep '[n]ginx:' | grep -Eq ' Z |defunct'; then
        return 1
    fi
    if ps | grep '[n]ginx:' >/dev/null 2>&1; then
        return 1
    fi
    log "nginx_residual=none"
}

trap cleanup_all EXIT INT TERM
init_timeout_cmd
( sleep 120; log "watchdog timeout"; kill -TERM $$ ) &
WATCHDOG_PID=$!

cleanup_nginx
prepare_packages || fail "prepare packages"
prepare_tree || fail "prepare tree"
start_nginx || fail "start nginx"
run_short_connection_l0 || fail "short connection L0"
post_health_check || fail "post-stress health check"
graceful_quit || fail "graceful quit"
assert_no_zombie_or_residual || fail "nginx residual or zombie"
pass
