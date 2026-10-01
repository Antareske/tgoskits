#!/bin/sh
# StarryOS SG2002 initialization script
# Called from /etc/inittab ::once (primary) and /etc/profile (fallback).

echo "[starry-init] Initializing..."

# 1. Start SSH server
if [ -f /usr/sbin/sshd ] && [ -f /etc/ssh/sshd_config ]; then
    echo "[starry-init] Starting SSH server..."
    if [ ! -d /var/empty ]; then
        mkdir -p /var/empty
        chmod 755 /var/empty
    fi
    mkdir -p /var/log
    /usr/sbin/sshd -f /etc/ssh/sshd_config 2>&1 &
    echo "[starry-init] SSH server started on port 22"
fi

# 2. Bring up WiFi DHCP
if [ -d /sys/class/net/wlan0 ]; then
    echo "[starry-init] WiFi interface detected, requesting DHCP..."
    udhcpc -i wlan0 -n -q 2>/dev/null &
fi

echo "[starry-init] Initialization complete."
