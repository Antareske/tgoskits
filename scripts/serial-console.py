#!/usr/bin/env python3
"""Simple serial console.

Type text and press Enter to send a line to the serial port.
Press Ctrl-C once to send ^C to the board.
Press Ctrl-C twice in a row to exit the program.
"""

from __future__ import annotations

import argparse
import os
import select
import subprocess
import sys
import termios
import time
import tty


def configure_serial(device: str, baud: int) -> None:
    subprocess.run(
        [
            "stty",
            "-F",
            device,
            str(baud),
            "raw",
            "-echo",
            "-ixon",
            "-ixoff",
            "-crtscts",
        ],
        check=True,
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("device", help="serial device, e.g. /dev/ttyUSB0")
    parser.add_argument("baud", nargs="?", type=int, default=1500000, help="baud rate")
    args = parser.parse_args()

    configure_serial(args.device, args.baud)

    serial_fd = os.open(args.device, os.O_RDWR | os.O_NOCTTY | os.O_SYNC)
    stdin_fd = sys.stdin.fileno()
    old_tty = termios.tcgetattr(stdin_fd)
    tty.setraw(stdin_fd)
    os.set_blocking(stdin_fd, False)
    os.set_blocking(serial_fd, False)

    ctrl_c_deadline = 0.0

    try:
        while True:
            now = time.monotonic()
            if ctrl_c_deadline and now >= ctrl_c_deadline:
                ctrl_c_deadline = 0.0

            timeout = None
            if ctrl_c_deadline:
                timeout = max(0.0, ctrl_c_deadline - now)

            ready, _, _ = select.select([stdin_fd, serial_fd], [], [], timeout)
            if not ready:
                ctrl_c_deadline = 0.0
                continue

            if stdin_fd in ready:
                chunk = os.read(stdin_fd, 1)
                if not chunk:
                    break

                byte = chunk[0]
                if byte == 3:
                    if ctrl_c_deadline:
                        break
                    os.write(serial_fd, b"\x03")
                    ctrl_c_deadline = time.monotonic() + 1.0
                    continue

                ctrl_c_deadline = 0.0

                if byte in (10, 13):
                    os.write(serial_fd, b"\n")
                elif byte in (8, 127):
                    continue
                else:
                    os.write(serial_fd, bytes([byte]))

            if serial_fd in ready:
                data = os.read(serial_fd, 4096)
                if not data:
                    break
                os.write(sys.stdout.fileno(), data)
                sys.stdout.flush()

    finally:
        termios.tcsetattr(stdin_fd, termios.TCSADRAIN, old_tty)
        os.close(serial_fd)

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
