# feat/net-benchmark 资料索引

本目录归档 `feat/net-benchmark` 分支（原 `feat/net-enhance`）独有的应用文档。分支 tip：`31b365b58`（2026-09-14）。整理日期：2026-11-15。

## 来源

| 本目录文件 | 分支内路径 |
| --- | --- |
| `ltp-netstress-README.md` | `apps/starry/qemu/ltp-netstress/README.md` |
| `iperf3-README.md` | `apps/starry/iperf3/README.md` |

该分支更早的计划、周报与 net-bench 分析文档仍在 `feat/net-enhance/` 目录（`www` 分支已跟踪）。本次整理未改动那批文件。

## 内容

| 文档 | 内容 |
| --- | --- |
| `ltp-netstress-README.md` | StarryOS 上运行 LTP netstress 的请求响应基准：测量范围、与 iperf3 的分工、上游契约、手动执行方式 |
| `iperf3-README.md` | StarryOS iperf3 板级基准：TCP 场景矩阵，Orange Pi 5 Plus / AKA-00-SG2002 的运行方式 |

## 与 dev 的关系

相对当前 `dev`，该分支的代码差异只剩 `apps/starry/qemu/ltp-netstress/`、`apps/starry/iperf3/README.md` 和 `apps/.ignore`；其余 net-bench 改动已进入主线。
