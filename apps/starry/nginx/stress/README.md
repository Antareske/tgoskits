# Nginx Stress Tests

This directory is reserved for stress and pressure tests.

Current status:

- S0 baseline is integrated:
  - `/usr/bin/nginx-stress-s0-baseline.sh`
  - `apps/starry/nginx/qemu-x86_64-stress-s0.toml`
  - Linux baseline passed on 2026-06-03.
  - StarryOS x86_64 qemu passed on 2026-06-03.
- S1 short connection churn is integrated:
  - `/usr/bin/nginx-stress-s1-short-conn.sh`
  - `apps/starry/nginx/qemu-x86_64-stress-s1.toml`
  - Linux baseline passed on 2026-06-03.
  - StarryOS x86_64 qemu passed on 2026-06-05.
- S2 keep-alive is integrated:
  - `/usr/bin/nginx-stress-s2-keepalive.sh`
  - `apps/starry/nginx/qemu-x86_64-stress-s2.toml`
  - Linux baseline passed on 2026-06-06.
  - StarryOS x86_64 qemu passed on 2026-06-06.
- Planned items moved out from phase tracking:
  - concurrent 8, 1000 requests
  - concurrent 32, 5000 requests
  - large file concurrent download
  - mixed 200/404/range/large traffic

Management rule:

- Stress tests are managed separately from phase tests.
- Stress tests are not connected to tgoskits CI test entry for nginx.
