#!/bin/sh
set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
if [ -r /usr/bin/nginx-alpine-mirror.sh ]; then
    . /usr/bin/nginx-alpine-mirror.sh
elif [ -r "$SCRIPT_DIR/../nginx-alpine-mirror.sh" ]; then
    . "$SCRIPT_DIR/../nginx-alpine-mirror.sh"
fi

ID=S2
BASE=/tmp/nginx-stress-s2
CONF="$BASE/conf/keepalive.conf"
WWW="$BASE/www"
OUT="$BASE/out"
LOGDIR="$BASE/logs"
TIMEOUT_CMD=
WATCHDOG_PID=
MASTER_PID=
REQUEST_TIMEOUT_SEC=30
POST_HEALTH_TIMEOUT_SEC=20

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
    printf 'nginx stress s2 keepalive\n' > "$WWW/small.txt"
    cat > "$CONF" <<'EOF'
daemon off;
master_process on;
worker_processes 1;
error_log /tmp/nginx-stress-s2/logs/error.log debug;
pid /tmp/nginx-stress-s2/nginx.pid;
events { worker_connections 128; }
http {
    include /etc/nginx/mime.types;
    access_log /tmp/nginx-stress-s2/logs/access.log;
    sendfile off;
    keepalive_timeout 5;
    server {
        listen 127.0.0.1:8080;
        root /tmp/nginx-stress-s2/www;
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
            grep -qx 'nginx stress s2 keepalive' "$OUT/health.body" && return 0
        fi
        i=$((i + 1))
        sleep 1
    done
    return 1
}

keepalive_client() {
    client_id=$1
    requests=$2
    out_file="$OUT/client.$client_id.raw"
    set --
    i=1
    while [ "$i" -le "$requests" ]; do
        [ "$i" -gt 1 ] && set -- "$@" --next
        set -- "$@" -o /dev/null --write-out '%{http_code}\n' "http://127.0.0.1:8080/small.txt"
        i=$((i + 1))
    done
    run_with_timeout "$REQUEST_TIMEOUT_SEC" curl --http1.1 --keepalive-time 60 --no-progress-meter --silent --show-error "$@" > "$out_file" 2>> "$OUT/client.$client_id.err"
}

run_keepalive_load() {
    clients=8
    requests_per_client=20
    client=1
    client_pids=
    while [ "$client" -le "$clients" ]; do
        keepalive_client "$client" "$requests_per_client" &
        client_pids="$client_pids $!"
        client=$((client + 1))
    done
    for pid in $client_pids; do
        wait "$pid" || true
    done

    success=0
    failed=0
    fail_log="$OUT/failures.log"
    : > "$fail_log"

    client=1
    while [ "$client" -le "$clients" ]; do
        raw="$OUT/client.$client.raw"
        norm="$OUT/client.$client.norm"
        tr -d '\r' < "$raw" > "$norm"
        count=$(grep -c '^200$' "$norm" || true)
        if [ "$count" -eq "$requests_per_client" ]; then
            success=$((success + 1))
        else
            failed=$((failed + 1))
            printf 'client=%s expected=%s got=%s\n' "$client" "$requests_per_client" "$count" >> "$fail_log"
        fi
        client=$((client + 1))
    done

    log "level=L1 clients=$clients requests_per_client=$requests_per_client success=$success failed=$failed"
    if [ "$failed" -ne 0 ]; then
        log "fail_samples=$(sed -n '1,8p' "$fail_log" | tr '\n' ';')"
    fi
    [ "$success" -eq "$clients" ] && [ "$failed" -eq 0 ]
}

post_health_check() {
    attempt=0
    while [ "$attempt" -lt "$POST_HEALTH_TIMEOUT_SEC" ]; do
        if run_with_timeout 5 curl -fsS -o "$OUT/post-health.body" http://127.0.0.1:8080/small.txt >/dev/null 2>&1 && grep -qx 'nginx stress s2 keepalive' "$OUT/post-health.body"; then
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
( sleep 1200; log "watchdog timeout"; kill -TERM $$ ) &
WATCHDOG_PID=$!

cleanup_nginx
prepare_packages || fail "prepare packages"
prepare_tree || fail "prepare tree"
start_nginx || fail "start nginx"
run_keepalive_load || fail "keepalive load"
post_health_check || fail "post-stress health check"
graceful_quit || fail "graceful quit"
assert_no_zombie_or_residual || fail "nginx residual or zombie"
pass
