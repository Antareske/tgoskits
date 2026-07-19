# Starry Nginx 测试 CI 重构方案（Proposal）

> 本文是 nginx 测试重构的权威详细设计文档。`apps/starry/nginx/README.md` 仅做
> 简述并指向本文。历史上的 `nginx-ci-refactor-plan.md` / `problems.md` /
> 旧版 `NGINX_CI_*` marker 方案均已废弃，以本文为准。

## 1. 背景与根因

### 1.1 现状

- `apps/starry/nginx/` 根目录散落约 30 个 `qemu-*.toml`，命名语义混乱
  （`phase1` / `phase2` / `phase5` 等非标准 id 与 `phase00` / `phase31` 等标准 id 混用）。
- 入口脚本分散：`smoke/nginx-smoke-tests.sh`、`phase/*.sh`（14 个阶段）、
  `nginx-all-tests.sh`（顺序调用）、`nginx-cli-tests.sh`、`debug/*.sh`。
- `prebuild.sh` 把每个脚本铺平注入到 guest `/usr/bin/nginx-phaseNN-tests.sh`，
  注入名与标准 id 不一致。
- 只有 x86_64 覆盖全部阶段；其他架构仅 phase12/13/31/32 有验证。

### 1.2 根因：阶段间状态泄漏

`nginx-all-tests.sh` 以 `set -eu` 顺序、以**独立进程**方式调用每个 phase 脚本。
每个 phase 脚本头部都有：

```sh
( sleep 90; log "watchdog timeout"; kill -TERM $$ ) &
```

问题链：

1. watchdog 是后台子 shell，phase 脚本正常结束时**不回收**它（无 `trap`、无 `kill`）。
2. `kill -TERM $$` 中 `$$` 是该 phase 进程的 PID。phase 退出后该 PID 被释放。
3. 下一个 phase 进程**复用**了同一 PID。
4. 旧 watchdog 在 90s 后醒来，对已复用的 PID 发 `TERM`，**误杀正在跑的下一阶段**，
   表现为 `Terminated` 且回落到交互 shell 卡死，需人工 `exit`（见
   `www/log/x86-all.log:454-455`、`www/log/riscv-all.log`）。

衍生隐患：

- phase 中途 `fail`（`exit 1`）或被 `TERM` 时**不执行 `cleanup_nginx`**，nginx 残留。
- 多个 phase 复用 `127.0.0.1:8080/8081/8082`，残留 nginx 占端口 → 下一阶段
  `start_nginx` bind 失败 → 假阴性。
- 磁盘 `/tmp/nginx-phaseNN` 各阶段独立但无回收，长流程累积。

## 2. 目标

1. 封装统一入口 `nginx-runner.sh`，支持 `smoke` / `phase` / `all` / `stress` / `debug`
   各模式独立运行。
2. `all` 模式（smoke + 全部 phase 顺序跑）必须保证**阶段强隔离**，彻底消除状态泄漏。
3. 重组散乱的 qemu TOML，目录即语义。
4. 仅 `smoke` 接入上层 CI；`phase` / `all` / `debug` / `stress` 为手工复测，不改 `.github` workflow。

## 3. 目录结构

> **关键约束（静态分析自 scripts/axbuild/src/starry/app.rs）**：CI 用
> `cargo xtask starry app qemu --all --arch <arch>`（.github/workflows/starry-apps.yml:115），
> 该路径靠 `resolve_qemu_config` **在 case 根目录**查找 `qemu-<arch>.toml`
> （app.rs:537、579）。app 发现 `collect_apps_in_dir` 命中 nginx 后即停止递归
> （app.rs:250），且 `collect_prefixed_toml_files` 不递归子目录（app.rs:834），
> 所以子目录里的 TOML 既**不会**被误判成新 app，也**不会**被默认发现选中——
> 只能通过显式 `--qemu-config` 使用。
>
> 结论：smoke/CI 入口**必须留在 case 根**为 `qemu-<arch>.toml`；其余手工入口放子目录。

```
apps/starry/nginx/
├── build-*.toml                  # 四架构构建配置（不动）
├── prebuild.sh                   # 注入 runner/ 与各阶段脚本到 guest overlay
│
├── qemu-x86_64.toml              # ← smoke/CI 入口（根目录，--all 自动发现）
├── qemu-riscv64.toml             #   shell_init_cmd = nginx-runner.sh smoke
├── qemu-aarch64.toml
├── qemu-loongarch64.toml
│
├── qemu/                         # 手工入口，仅显式 --qemu-config 使用
│   ├── all/                      # 全量（smoke + 全部 phase），四架构各一个
│   │   ├── qemu-x86_64.toml      #   shell_init_cmd = nginx-runner.sh all
│   │   ├── qemu-riscv64.toml
│   │   ├── qemu-aarch64.toml
│   │   └── qemu-loongarch64.toml
│   ├── phase/                    # 单阶段复测
│   │   ├── qemu-x86_64-phase00.toml ... qemu-x86_64-phase90.toml  (14 个)
│   │   ├── qemu-riscv64-phase12/13/31/32.toml
│   │   ├── qemu-aarch64-phase12/13.toml
│   │   └── qemu-loongarch64-phase12.toml
│   └── debug/                    # 单问题调试
│       ├── qemu-x86_64-bad-method-debug.toml
│       ├── qemu-x86_64-bad-method-matrix.toml
│       ├── qemu-x86_64-short-connection-debug.toml
│       ├── qemu-x86_64-short-connection-direct-debug.toml
│       ├── qemu-x86_64-timing-debug.toml
│       ├── qemu-riscv64-timing-debug.toml
│       └── qemu-x86_64-sendfile-on-debug.toml
│
├── runner/
│   ├── nginx-runner.sh           # 统一入口
│   ├── nginx-runner-lib.sh       # 共享库（隔离/timeout/cleanup/端口等待）
│   └── nginx-alpine-mirror.sh    # 现有 apk mirror helper，平移
│
├── smoke/nginx-smoke-tests.sh    # 现有，不动
├── phase/nginx-*.sh              # 14 个阶段脚本，仅删 watchdog、加 trap
├── debug/*.sh                    # 问题定位脚本，手工 debug 入口使用
└── stress/README.md              # 占位
```

根目录旧的散乱 `qemu-*-phaseNN.toml` / `qemu-*-all.toml` / `qemu-*-*-debug.toml` /
`nginx-all-tests.sh` / `nginx-cli-tests.sh` 迁移后删除（git history 可回溯）。
四个 `qemu-<arch>.toml` 保留在根但 `shell_init_cmd` 改为 `nginx-runner.sh smoke`。

## 4. 统一入口 `nginx-runner.sh`

```
/usr/bin/nginx-runner.sh <mode> [arg]
```

| mode | shell_init_cmd 示例 | 用途 | CI |
|------|--------------------|------|----|
| `smoke` | `nginx-runner.sh smoke` | 仅 smoke | ✅ 唯一接入 |
| `phase <id>` | `nginx-runner.sh phase phase31` | 单阶段复测 | ❌ |
| `all` | `nginx-runner.sh all` | smoke + 全部 phase，强隔离 | ❌ |
| `stress` | `nginx-runner.sh stress` | 压测（当前打印 skip） | ❌ |
| `debug <name>` | `nginx-runner.sh debug bad-method-debug` | 单问题调试 | ❌ |

### 4.1 统一 marker

| 事件 | marker |
|------|--------|
| 阶段开始 | `NGINX_RUNNER_PHASE_BEGIN phase=<id>` |
| 单阶段通过 | `NGINX_RUNNER_PHASE_PASS phase=<id>` |
| 单阶段失败 | `NGINX_RUNNER_PHASE_FAIL phase=<id> rc=<rc>` |
| 全部通过（终态） | `NGINX_RUNNER_PASSED mode=<mode>` |
| 失败收尾（终态） | `NGINX_RUNNER_FAILED phase=<id> rc=<rc>` |

QEMU 入口 regex 只关心两个终态 marker + panic 兜底：

```toml
success_regex = ["(?m)^NGINX_RUNNER_PASSED\\b"]
fail_regex    = ["(?i)\\bpanic(?:ked)?\\b", "(?m)^NGINX_RUNNER_FAILED\\b"]
```

`fail_regex` 收紧到精确终态 marker，不用宽泛的 `NGINX_.*_FAILED`，避免误伤阶段内
`KNOWN_ISSUE` 诊断输出。smoke 脚本自身的 `NGINX_APP_SMOKE_PASSED/FAILED` 保留不动，
runner 在 smoke 模式捕获其退出码后额外打印统一终态 marker。

## 5. all 模式阶段隔离契约（核心）

`all` 模式按阶段 id 顺序执行 smoke + 14 个 phase。runner 必须对**每个阶段**强制以下
隔离，且无论阶段成功/失败/超时都执行收尾。这是本次重构解决根因的关键。

### 5.1 进程隔离

- 每个阶段在**独立子进程**运行：`run_with_timeout <sec> sh <script>`，**不用 source**，
  避免 `set -eu`/变量/函数污染。
- 超时由 runner 统一用 `timeout <sec>` 包裹，**phase 脚本内不得再起 watchdog**
  （删除 `( sleep N; kill -TERM $$ ) &`）。超时职责单一上移到 runner。

### 5.2 收尾隔离（每阶段后无条件执行）

runner 在每个阶段（成功或失败）返回后执行 `isolate_after_phase`：

```sh
isolate_after_phase() {
    # 1. 杀残留 nginx（优雅 → 强杀）
    killall -q nginx 2>/dev/null || true
    sleep 1
    killall -q -9 nginx 2>/dev/null || true
    # 2. 回收阶段产生的后台 job（防御性，phase 已不再起 watchdog）
    #    runner 自身不在阶段执行期间起后台 job
    # 3. 确认目标端口已释放，最多等 N 秒
    wait_ports_free 8080 8081 8082
    # 4. 清理阶段临时目录
    rm -rf /tmp/nginx-phase* 2>/dev/null || true
}
```

### 5.3 端口释放等待

下一阶段 `start_nginx` 前，runner 确认 `8080/8081/8082` 不再 LISTEN，最多轮询 N 秒，
避免上一阶段残留占端口导致假阴性。

### 5.4 失败传播策略

`all` 模式默认 **fail-fast**：任一阶段 `PHASE_FAIL` 即打印 `NGINX_RUNNER_FAILED` 终止。
（可选 `all --keep-going` 跑完全部再汇总，作为后续增强，本期不实现。）

### 5.5 phase 脚本需配合的最小改动

每个 phase 脚本：

1. **删除** `( sleep 90; ... kill -TERM $$ ) &` watchdog 行。
2. **新增** `trap cleanup_nginx EXIT INT TERM`，保证中途 `fail`/`exit`/被杀时也清理
   自身 nginx，作为 runner 隔离的第二道防线。
3. 保留各自独立的 `/tmp/nginx-phaseNN` BASE 与 marker（runner 通过退出码判定，
   不依赖 phase 内 marker 文本）。

## 6. QEMU 入口组织

### 6.1 现有散乱 TOML 整合规则

- **smoke/CI 入口留在 case 根**为 `qemu-<arch>.toml`（四架构），因 `--all` 默认发现
  只认根目录（见 §3 约束）。仅把 `shell_init_cmd` 从 `nginx-smoke-tests.sh` 改为
  `nginx-runner.sh smoke`，success/fail_regex 换成统一终态 marker。
- 命名标准化：非标准 id（`phase1`/`phase2`/`phase5`/`phase6`/`phase0`/`phase7`/`phase9`）
  → 标准 id（`phase13`/`phase20`/`phase50`/`phase60`/`phase00`/`phase70`/`phase90`）。
  文件名统一 `qemu-<arch>-<phase-id>.toml`。
- x86_64 覆盖全部 14 阶段，全部迁入 `qemu/phase/`。
- 其他架构只保留实际验证过的阶段（riscv64: 12/13/31/32；aarch64: 12/13；
  loongarch64: 12），其余删除，等实际通过再补。
- 4 个 all 入口 TOML 整合为 `qemu/all/`（每架构一个，`shell_init_cmd =
  nginx-runner.sh all`）。由于 `starry app qemu` 不支持 `--shell-init-cmd` 覆盖
  （见 §8），all 模式必须有独立 TOML，不能复用 smoke TOML。

### 6.2 数量对比

| 当前 | 迁移后 |
|------|--------|
| 根目录散乱 ~30 toml | 根 smoke 4（CI）+ qemu/all 4 + qemu/phase ~21 + qemu/debug 7 |
| 命名不一致 | 目录即语义，文件名统一 |
| all 入口 4（不稳定，命名乱） | all 4（`qemu/all/`，统一 runner all 模式） |

## 7. 阶段 ID 对照表

| phase-id | 脚本 | 说明 |
|----------|------|------|
| `smoke`  | `nginx-smoke-tests.sh` | smoke |
| `phase00` | `nginx-0-0-env-rlimit-tests.sh` | env / rlimit |
| `phase12` | `nginx-1-2-lifecycle-tests.sh` | lifecycle 1.2 |
| `phase13` | `nginx-1-3-lifecycle-tests.sh` | lifecycle 1.3 |
| `phase20` | `nginx-2-0-http-basic-tests.sh` | HTTP basic |
| `phase31` | `nginx-3-1-short-connection-tests.sh` | short connection |
| `phase32` | `nginx-3-2-keepalive-tests.sh` | keepalive |
| `phase33` | `nginx-3-3-slow-header-tests.sh` | slow header |
| `phase41` | `nginx-4-1-sendfile-off-tests.sh` | sendfile off |
| `phase42` | `nginx-4-2-sendfile-on-tests.sh` | sendfile on |
| `phase43` | `nginx-4-3-range-tests.sh` | range |
| `phase50` | `nginx-5-0-request-body-tests.sh` | request body |
| `phase60` | `nginx-6-0-log-fs-tests.sh` | log / fs |
| `phase70` | `nginx-7-0-signal-lifecycle-tests.sh` | signal lifecycle |
| `phase90` | `nginx-9-0-config-feature-tests.sh` | config feature |

## 8. 用法

### CI（仅 smoke）

```bash
# CI 实际命令（.github/workflows/starry-apps.yml:115），靠根目录 qemu-<arch>.toml 自动发现
cargo xtask starry app qemu --all --arch x86_64
# riscv64 / aarch64 / loongarch64 同理；或显式：
cargo xtask starry app qemu -t nginx --arch x86_64 \
  --qemu-config apps/starry/nginx/qemu-x86_64.toml
```

### 手工单阶段

```bash
cargo xtask starry app qemu -t nginx --arch x86_64 \
  --qemu-config apps/starry/nginx/qemu/phase/qemu-x86_64-phase31.toml
```

### 手工 all

```bash
cargo xtask starry app qemu -t nginx --arch x86_64 \
  --qemu-config apps/starry/nginx/qemu/all/qemu-x86_64.toml
```

> **已确认**：`starry app qemu`（`ArgsAppQemu`，scripts/axbuild/src/starry/app.rs:43）
> 只接受 `--qemu-config`，**无 `--shell-init-cmd` 覆盖**（该 flag 仅 `perf` 路径有，
> 且 `starry test qemu` 已移除该 flag，见 mod.rs:1096 测试
> `command_rejects_removed_shell_init_cmd_flag`）。`shell_init_cmd` 只能从 TOML 读取。
> 因此 all 模式**必须有独立 TOML**，目录为 `qemu/all/`，
> 内容除 `shell_init_cmd = "/usr/bin/nginx-runner.sh all"` 与更长的 `timeout` 外，
> 与同架构 smoke TOML 一致。

### 手工 debug

```bash
cargo xtask starry app qemu -t nginx --arch x86_64 \
  --qemu-config apps/starry/nginx/qemu/debug/qemu-x86_64-bad-method-debug.toml
```

## 9. 迁移步骤

1. 新建 `runner/nginx-runner.sh` + `runner/nginx-runner-lib.sh`，实现 §4 入口与 §5 隔离契约。
2. 修复 14 个 phase 脚本：删 watchdog、加 `trap cleanup_nginx EXIT INT TERM`（§5.5）。
3. 改根目录四个 `qemu-<arch>.toml`：`shell_init_cmd = nginx-runner.sh smoke`，
   regex 换统一终态 marker（smoke/CI 入口，留在根）。
4. 新建 `qemu/{all,phase,debug}/`，按 §6/§7 写入手工 TOML。
5. 更新 `prebuild.sh`：注入 `runner/` 两脚本与标准 id 命名的 phase 脚本，删 `nginx-all-tests.sh` 注入行。
6. 删根目录散乱 `qemu-*-phaseNN.toml`/`qemu-*-all.toml`/`qemu-*-*-debug.toml`、
   `nginx-all-tests.sh`、`nginx-cli-tests.sh`。
7. 静态验证：`dash -n` 所有脚本、TOML 可解析、`app qemu --all` 仍只发现 nginx 一个 qemu app。

## 10. 待确认项

- ~~xtask 是否支持 `--shell-init-cmd` 覆盖~~ → **已确认不支持**，all 用独立
  `qemu/all/` TOML（见 §6.1、§8）。
- ~~guest 收尾是否主动 `poweroff -f`/`reboot -f`~~ → **已确认不需要**：ostool
  匹配到 success/fail_regex 后主动 `child.kill()` 终止 QEMU
  （ostool-0.23.1 src/run/qemu.rs:568-574），不依赖 guest 关机。runner 打印终态
  marker 即可，QEMU 由宿主侧终止。
