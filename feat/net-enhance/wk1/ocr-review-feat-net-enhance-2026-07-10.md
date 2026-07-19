# Code Review: feat/net-enhance — L2 Header Length Accounting

**Session**: 2026-07-10-feat-net-enhance
**Round**: 1
**Reviewers**: 2× Principal Engineer, 2× Code Quality Engineer
**Verdict**: APPROVE

## Summary

This change adds L2 header length accounting to the `ax-net` router's byte counters via a new `l2_header_len()` method on the `Device` trait. It aligns `/proc/net/dev` output with Linux semantics by converting IP-payload byte counts into L2 frame byte counts (adding 14 bytes per Ethernet frame, 0 for loopback).

The code is **clean, minimal, and architecturally sound**. All 4 reviewers agree the core mechanism (trait default → device override → cached field → counter adjustment) is well-designed. No blockers were identified. The findings below are improvements that strengthen correctness and maintainability.

补充：基于 net-bench-proc-net-dev-plan.md 做的实现，暂存在 feat/net-l2-header-len。

---

## Blockers

None.

---

## Should Fix

### 1. Loopback fast paths bypass `l2_header_len` (and the broader caller-must-remember pattern)

- **Severity**: Medium
- **Location**: `net/ax-net/src/router.rs:367`, `743-744`, `886-887`, `998`
- **Confidence**: Very High (4/4 reviewers agree)
- **Issue**: The L2 header adjustment is manually added at each `count_rx`/`count_tx` call site. Two loopback paths (`send_on_device` L743-744, `dispatch_unicast_packet` L886-887) call `count_tx(packet.len())` and `count_rx(packet.len())` without adding `l2_header_len`. This is numerically correct today (loopback returns 0), but creates a maintenance hazard — any future device with a non-zero header routed through a fast path would silently undercount.

- **Suggestion**: Absorb the `l2_header_len` addition into `count_rx()` and `count_tx()` themselves. Both are already methods on `DeviceHandle`, which holds the `l2_header_len` field:

  ```rust
  fn count_rx(&self, ip_payload_len: usize) {
      let frame_len = ip_payload_len + self.l2_header_len;
      self.rx_bytes.fetch_add(frame_len as u64, Ordering::Relaxed);
      self.rx_packets.fetch_add(1, Ordering::Relaxed);
  }

  fn count_tx(&self, ip_payload_len: usize) {
      let frame_len = ip_payload_len + self.l2_header_len;
      self.tx_bytes.fetch_add(frame_len as u64, Ordering::Relaxed);
      self.tx_packets.fetch_add(1, Ordering::Relaxed);
  }
  ```

  Then simplify all call sites to `self.count_rx(packet.len())` / `self.count_tx(packet.len())`. This eliminates the failure mode at the API level and naturally fixes the loopback bypass.

  The parameter rename from `len` to `ip_payload_len` is essential — it documents what unit the caller is expected to provide.

### 2. Stale `NetDevStats` doc comment

- **Severity**: Low
- **Location**: `net/ax-net/src/router.rs:81-83`
- **Confidence**: Medium (2/4 reviewers agree)
- **Issue**: The doc comment on `NetDevStats` states: "Byte counts use the IP packet length carried on the `Medium::Ip` links exposed by this stack." After this change, the counters include L2 framing overhead. The comment is now misleading.

- **Suggestion**: Update to reflect L2 frame byte semantics, e.g.: "Byte counts use L2 frame length (IP-payload length + per-device L2 header length) aligned with Linux `/proc/net/dev` semantics. The L2 header length is device-specific — 14 bytes for Ethernet, 0 for loopback."

---

## Suggestions

### 3. Tighten trait method documentation

- **Severity**: Low
- **Location**: `net/ax-net/src/device/mod.rs:98-106`
- **Issue**: The `l2_header_len()` trait doc enumerates Ethernet-specific details (DMAC+SMAC+EtherType) that belong in the `EthernetDevice` impl. Keep the `/proc/net/dev` reference (it explains *why* the method exists) but describe the abstract contract rather than current implementations.

### 4. De-duplicate field documentation

- **Severity**: Low
- **Location**: `net/ax-net/src/router.rs:283-286`
- **Issue**: The `l2_header_len` field doc restates the trait method doc nearly verbatim. Shorten to a cross-reference: "Cached from `Device::l2_header_len()` — see trait docs for semantics."

### 5. Add `debug_assert!` for `l2_header_len` validation

- **Severity**: Info
- **Location**: `net/ax-net/src/router.rs:296`
- **Issue**: A buggy device returning an implausible `l2_header_len` value would silently corrupt counters. A `debug_assert!(l2_header_len <= 128)` at construction time would catch logic errors in development without affecting release performance.

### 6. Add test coverage for non-zero `l2_header_len`

- **Severity**: Info
- **Location**: `net/ax-net/src/router.rs:1124-1340` (test module)
- **Issue**: The test-only `EmptyDevice` inherits the default `l2_header_len() -> 0`, so tests cannot distinguish "correctly applied" from "not applied at all." Add a test device that overrides `l2_header_len()` to return a known non-zero value and assert the counters include the overhead.

### 7. Document TX counting at enqueue (pre-existing)

- **Severity**: Info
- **Location**: `net/ax-net/src/router.rs:324` (`count_tx`)
- **Issue**: TX byte counting occurs at enqueue time, not at device transmission completion. If the device later fails to transmit, the byte is already counted. This is pre-existing behavior but worth documenting alongside the new `l2_header_len` semantics.

---

## What's Working Well

1. **Clean trait design**: `l2_header_len()` as a default method returning 0 means non-Ethernet devices get correct behavior automatically. The right level of abstraction.

2. **Single capture at construction**: Reading `l2_header_len` once and caching it on `DeviceHandle` avoids locking the device on every packet. Efficient and correct for a compile-time constant.

3. **Consistent RX/TX symmetry**: Both `enqueue_tx` and `device_rx_worker` apply the adjustment in the same way.

4. **Single source of truth**: `EthernetDevice` reuses smoltcp's `EthernetFrame::header_len()` rather than hardcoding 14, matching existing usage at `ethernet.rs:188`.

5. **Minimal diff surface**: The change touches only the essential machinery without unnecessary refactoring.

6. **Appropriate atomics**: `Ordering::Relaxed` is correct for statistics counters.

---

## Clarifying Questions

1. **VLAN / tunnel support**: Some configurations use 802.1Q VLAN tags (adding 4 bytes). If VLAN support is planned, `l2_header_len` may need to become per-packet rather than per-device.

2. **Out-of-tree Device implementations**: Are there `Device` trait implementors outside `ax-net` (e.g., in platform HALs) that might need a non-zero `l2_header_len` override?

3. **Loopback bidirectional counting**: The loopback path counts both TX and RX for the same packet. Is this consistent with Linux `/proc/net/dev` semantics for loopback?

---

## Individual Reviews

| Reviewer | File | Findings |
|----------|------|----------|
| @principal-1 | [principal-1.md](reviews/principal-1.md) | 4 findings (2 Medium, 2 Low) |
| @principal-2 | [principal-2.md](reviews/principal-2.md) | 5 findings (1 Medium, 2 Low, 2 Info) |
| @quality-1 | [quality-1.md](reviews/quality-1.md) | 5 findings (1 High, 2 Low, 2 Info) |
| @quality-2 | [quality-2.md](reviews/quality-2.md) | 5 findings (1 Medium, 2 Low, 2 Info) |
