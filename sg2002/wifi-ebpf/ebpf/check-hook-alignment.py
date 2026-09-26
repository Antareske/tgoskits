#!/usr/bin/env python3
"""Decide whether netmon's hook symbols in a built kernel would be accepted by
the kprobe layer, by decoding the entry instruction with the same rules the
kprobe crate applies on RISC-V:

  * odd address                      -> InvalidAddress
  * 16-bit entry ((halfword&3)!=3)   -> rejected for c.ebreak, PC-relative
                                        branches (c.jal/c.j/c.beqz/c.bnez) and
                                        c.jr/c.jalr
  * 32-bit entry                     -> rejected when the address is not
                                        4-byte aligned, or the opcode is
                                        auipc/jalr/branch/jal/fence/system

Usage: check-hook-alignment.py [path/to/starryos]
"""

import re
import subprocess
import sys

OBJDUMP = "/opt/riscv64-linux-musl-cross/bin/riscv64-linux-musl-objdump"

HOOKS = [
    # Fragment sets must stay identical to the loader's PROBES table.
    ("sdio_read", ["14sdmmc_protocol", "8transfer", "8SdioCard", "15submit_read_dma"]),
    ("sdio_write", ["14sdmmc_protocol", "8transfer", "8SdioCard", "16submit_write_dma"]),
    ("wifi_start", ["7aic8800", "7control", "14AicWifiControl", "11WifiControl", "5start"]),
    ("queue_poll", ["6ax_net", "13queue_runtime", "8executor", "18QueueGroupExecutor", "4poll"]),
    ("port_tx", ["6ax_net", "13queue_runtime", "8executor", "14QueueFramePort",
                 "17EthernetFramePort", "8transmit"]),
    ("port_rx", ["6ax_net", "13queue_runtime", "8executor", "14QueueFramePort",
                 "17EthernetFramePort", "7receive"]),
    ("sched_irq", ["6ax_net", "13queue_runtime", "5state", "14PollGroupState",
                   "12schedule_irq"]),
]

PC_RELATIVE_16 = {(0x1, 0x1), (0x1, 0x5), (0x1, 0x6), (0x1, 0x7)}
UNSUPPORTED_32 = {0x17, 0x67, 0x63, 0x6f, 0x0f, 0x73}


def symbols(elf):
    out = subprocess.run(["nm", elf], capture_output=True, text=True).stdout
    syms = []
    for line in out.splitlines():
        parts = line.split()
        if len(parts) >= 3:
            syms.append((int(parts[0], 16), parts[2]))
    return syms


def entry_bytes(elf, addr, count=4):
    out = subprocess.run(
        [OBJDUMP, "-s", f"--start-address={addr}", f"--stop-address={addr + count}", elf],
        capture_output=True, text=True).stdout
    data = []
    for line in out.splitlines():
        match = re.match(r"^\s+[0-9a-f]+\s+((?:[0-9a-f]{2,8}\s+)+)", line)
        if match:
            for group in match.group(1).split():
                data += [int(group[i:i + 2], 16) for i in range(0, len(group), 2)]
    return bytes(data[:count])


def verdict(addr, raw):
    if len(raw) < 4:
        return "UNREADABLE"
    halfword = raw[0] | (raw[1] << 8)
    if addr & 1:
        return "REJECTED (odd address)"
    if (halfword & 0x3) != 0x3:
        funct3 = (halfword >> 13) & 0x7
        rs2 = (halfword >> 2) & 0x1F
        if halfword == 0x9002:
            return "REJECTED (c.ebreak)"
        if (halfword & 0x3, funct3) in PC_RELATIVE_16:
            return "REJECTED (PC-relative 16-bit branch)"
        if (halfword & 0x3) == 0x2 and funct3 == 0x4 and rs2 == 0:
            return "REJECTED (c.jr/c.jalr)"
        return "OK (16-bit entry)"
    if addr & 0x3:
        return "REJECTED (2-byte aligned, entry encodes 32-bit)"
    word = int.from_bytes(raw[:4], "little")
    opcode = word & 0x7F
    if opcode in UNSUPPORTED_32:
        return f"REJECTED (32-bit entry opcode {opcode:#04x})"
    return "OK (32-bit entry)"


def main():
    elf = sys.argv[1] if len(sys.argv) > 1 else \
        "target/riscv64gc-unknown-none-elf/release/starryos"
    all_syms = symbols(elf)
    for label, fragments in HOOKS:
        matches = [s for s in all_syms if all(f in s[1] for f in fragments)]
        if len(matches) > 1:
            methods = [s for s in matches if s[1].startswith("_RNv")]
            if len(methods) == 1:
                matches = methods
        if len(matches) != 1:
            print(f"{label:<11} UNRESOLVED ({len(matches)} matches)")
            continue
        addr, name = matches[0]
        raw = entry_bytes(elf, addr)
        aligned = "4-byte" if addr & 0x3 == 0 else "2-byte"
        print(f"{label:<11} {addr:#012x}  {aligned}  {verdict(addr, raw)}")
        print(f"{'':<11} entry bytes: {raw.hex(' ')}")
        print(f"{'':<11} {name}")


if __name__ == "__main__":
    main()
