#!/usr/bin/env python3
"""Simple serial console.

Type text and press Enter to send a line to the serial port.
Press Ctrl-C once to send ^C to the board.
Press Ctrl-C twice in a row to exit the program.
Press Ctrl-] to exit immediately.
"""

from __future__ import annotations

import argparse
import errno
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


def write_all(fd: int, data: bytes) -> bool:
    while data:
        try:
            written = os.write(fd, data)
        except InterruptedError:
            continue
        except BlockingIOError:
            select.select([], [fd], [])
            continue
        except OSError as err:
            if err.errno in (errno.EAGAIN, errno.EWOULDBLOCK):
                select.select([], [fd], [])
                continue
            raise
        if written == 0:
            return False
        data = data[written:]
    return True


def read_available(fd: int, chunk_size: int = 4096) -> bytes | None:
    chunks: list[bytes] = []
    while True:
        try:
            chunk = os.read(fd, chunk_size)
        except InterruptedError:
            continue
        except BlockingIOError:
            break
        except OSError as err:
            if err.errno in (errno.EAGAIN, errno.EWOULDBLOCK):
                break
            raise

        if not chunk:
            return None
        chunks.append(chunk)
        if len(chunk) < chunk_size:
            break

    return b"".join(chunks)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("device", help="serial device, e.g. /dev/ttyUSB0")
    parser.add_argument("baud", nargs="?", type=int, default=1500000, help="baud rate")
    parser.add_argument(
        "--enter",
        choices=("cr", "lf", "crlf"),
        default="cr",
        help="bytes sent for Enter (default: cr)",
    )
    parser.add_argument(
        "--local-echo",
        action="store_true",
        help="echo typed characters locally when the board does not echo input",
    )
    parser.add_argument("--log", help="also append serial output to this file")
    args = parser.parse_args()

    configure_serial(args.device, args.baud)

    serial_fd = os.open(args.device, os.O_RDWR | os.O_NOCTTY | os.O_SYNC)
    stdin_fd = sys.stdin.fileno()
    stdout_fd = sys.stdout.fileno()
    old_tty = termios.tcgetattr(stdin_fd)
    old_stdin_blocking = os.get_blocking(stdin_fd)
    tty.setraw(stdin_fd)
    os.set_blocking(stdin_fd, False)
    os.set_blocking(serial_fd, False)

    enter_bytes = {"cr": b"\r", "lf": b"\n", "crlf": b"\r\n"}[args.enter]
    log_file = open(args.log, "ab", buffering=0) if args.log else None
    ctrl_c_deadline = 0.0

    try:
        sys.stderr.write(
            f"connected to {args.device} @ {args.baud}; Ctrl-C twice or Ctrl-] exits\n"
        )
        sys.stderr.flush()

        while True:
            now = time.monotonic()
            if ctrl_c_deadline and now >= ctrl_c_deadline:
                ctrl_c_deadline = 0.0

            timeout = None
            if ctrl_c_deadline:
                timeout = max(0.0, ctrl_c_deadline - now)

            try:
                ready, _, _ = select.select([stdin_fd, serial_fd], [], [], timeout)
            except InterruptedError:
                continue
            if not ready:
                ctrl_c_deadline = 0.0
                continue

            if stdin_fd in ready:
                chunk = read_available(stdin_fd, 128)
                if chunk is None:
                    break
                if not chunk:
                    continue

                for byte in chunk:
                    if byte == 29:
                        return 0
                    if byte == 3:
                        if ctrl_c_deadline:
                            return 0
                        if not write_all(serial_fd, b"\x03"):
                            return 1
                        ctrl_c_deadline = time.monotonic() + 1.0
                        continue

                    ctrl_c_deadline = 0.0

                    if byte in (10, 13):
                        data = enter_bytes
                    elif byte in (8, 127):
                        data = b"\x7f"
                    else:
                        data = bytes([byte])
                    if not write_all(serial_fd, data):
                        return 1
                    if args.local_echo:
                        if byte in (10, 13):
                            write_all(stdout_fd, b"\r\n")
                        elif byte in (8, 127):
                            write_all(stdout_fd, b"\b \b")
                        elif byte >= 32:
                            write_all(stdout_fd, bytes([byte]))

            if serial_fd in ready:
                data = read_available(serial_fd)
                if data is None:
                    break
                if data:
                    write_all(stdout_fd, data)
                    if log_file is not None:
                        log_file.write(data)

    finally:
        if log_file is not None:
            log_file.close()
        os.set_blocking(stdin_fd, old_stdin_blocking)
        termios.tcsetattr(stdin_fd, termios.TCSADRAIN, old_tty)
        os.close(serial_fd)

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
