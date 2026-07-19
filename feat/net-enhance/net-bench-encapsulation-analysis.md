# net-bench 封装程度分析

## 概述

本文档分析 `apps/starry/net-bench/` 测试套件在流程封装、资源管理和产品化方面的系统性欠缺。以 `apps/starry/nginx/` 建立的 xtask app 范式为参照标准，按维度逐一对比并提出改进方向。

## 参照标准：nginx 范式

nginx 测试定义了项目中 Starry app 的"产品级"封装范式：

```
cargo xtask starry app qemu -t nginx --arch x86_64
```

核心特征：

1. **单一入口**：一条命令触发完整生命周期（prebuild → build → QEMU run → 标记匹配）
2. **资源管理入体系**：rootfs 由 xtask image registry 统一管理（下载、SHA256 校验、`.tgos-images/` 缓存、文件锁并发控制）；prebuild.sh 只做无网络的脚本安装
3. **guest 内包安装有回退**：`nginx-alpine-mirror.sh` 检测 Alpine 版本分支 → 清华镜像 → dl-cdn 回退 → sentinel 防重复 → 重试+超时
4. **CI 原生集成**：QEMU toml 中的 `success_regex` / `fail_regex` 匹配 runner 输出标记
5. **脚本不感知 xtask 内部实现**：不硬编码 xtask 的 tmp 目录结构

对应的参考 app：
- **dropbear**：prebuild 时离线解析 APKINDEX、下载 apk 包、解包到 overlay，无需 guest 运行时网络
- **doom**：prebuild 时在 staging root 中完整编译 doomgeneric，用 `readelf` 递归解析依赖
- **claw-code**：prebuild 时从 GitHub 克隆并编译，缓存到 `~/.cache/`

## net-bench 当前封装度

### 已有的良好实践

- `core/lib.sh` 集中管理常量、前置检查、iperf3 生命周期、环境指纹、结果汇总
- prebuild.sh 的 `readelf -d` 递归依赖解析（一致性优于手工指定）
- `bin/setup` → `env/setup-common.sh` 的状态化管理（`.bench-state.json` 记录创建的资源，teardown 精确回滚）
- `bin/bench` 的实验性自动检测（`detect-env.sh` JSON 输出，供上层脚本消费）

### 欠缺一：双入口割裂

net-bench 有两条完全独立的执行路径，生命周期不统一：

| | Starry 测试 | Linux baseline |
|---|---|---|
| 入口 | `cargo xtask starry app qemu --test-case net-bench` | `bash run-linux-baseline.sh x86_64 vhost` |
| prebuild | 走 xtask → prebuild.sh（apk 安装 iperf3） | 无 prebuild，运行时构造 initramfs |
| 内核 | StarryOS（cargo build） | 外部 Linux vmlinuz（手动放置或 apt-get 拉取） |
| rootfs | xtask overlay 注入 | 手动 `debugfs rdump` → 最小化 cpio |
| 标记 | `NET_BENCH_PASSED`（QEMU toml regex） | 同类标记但无 CI regex 消费 |

Linux baseline 不与 xtask 框架交互，不走 prebuild → build → run 阶段划分。`run.sh` 调用 `cargo xtask` 而 `run-linux-baseline.sh` 直接拼 QEMU 命令行。两者的生命周期管理逻辑各自独立。

### 欠缺二：资源拉取游离于 xtask 体系

net-bench 有三种资源拉取路径，其中两种在体系内，一种在体系外：

**体系内（xtask image registry 管理）：**
- Alpine rootfs：`cargo xtask starry rootfs --arch $ARCH` → registry 下载 → SHA256 校验 → `.tgos-images/` 缓存
- iperf3（prebuild 阶段）：`qemu-user + apk add`，带 4 次重试退避，APK 缓存到 `target/net-bench-apk-cache/`

**体系外（裸 shell 命令）：**
- Linux guest 内核（`ensure_x86_kernel()`）：直接 `apt-get download`，无 SHA256 校验、无版本固化、无重试/回退、无并发锁。拉取的是 Ubuntu 仓库中当前最新的 generic 内核，不可复现

对比 dropbear 的做法：prebuild.sh 解析 APKINDEX 获取精确版本号 → 下载特定 `.apk` → 离线安装到 overlay，整个过程可复现且不依赖运行时网络。

### 欠缺三：脚本感知 xtask 内部实现细节

`run-linux-baseline.sh` 的 `locate_alpine_image()` 硬编码了 xtask 内部路径：

```bash
local flat="$WORKSPACE/tmp/axbuild/rootfs/$image_name"
local nested="$WORKSPACE/tmp/axbuild/rootfs/$image_name/$image_name"
```

`tmp/axbuild/rootfs/` 是 `image::storage` 模块的内部实现细节，不是稳定 API。如果 xtask 变更 rootfs 存储布局（例如统一走 `.tgos-images/` 下的嵌套结构），此函数即失效。

nginx/dropbear/doom 等 app 从不直接操作这些路径——它们通过 xtask 传入的环境变量（`STARRY_ROOTFS`、`STARRY_STAGING_ROOT`、`STARRY_OVERLAY_DIR`）获取资源位置，由 xtask 负责路径解析。

### 欠缺四：guest 侧测试逻辑重复维护

Starry guest 和 Linux guest 的测试矩阵相同，但实现在两个独立的代码位置：

```
core/net-bench-common.sh       # Starry guest 侧 run_test()
run-linux-baseline.sh           # Linux guest init 脚本内嵌 run_test()
```

新提交 `2529b4d6a` 已将 iperf3 参数（端口、时长、udp64 参数）同步，但结构上的重复没有消除。如果将来增加测试用例（如 tcp8、udp512），需要两处修改。

理想情况：测试矩阵为单一源文件，由 Starry guest 和 Linux guest 共同引用。nginx 的 runner 通过 `shell_init_cmd` 分发到不同 mode，所有 mode 共享 `runner-lib.sh` 中的 `runner_run_stage()`，不存在重复的测试编排逻辑。

### 欠缺五：CI 集成不对称

| | Starry 测试 | Linux baseline |
|---|---|---|
| QEMU toml 中的 `success_regex` | `NET_BENCH_PASSED` | 无 toml，无 regex |
| QEMU toml 中的 `fail_regex` | `panic` / `NET_BENCH_FAILED` | 无 |
| 超时控制 | QEMU toml `timeout = 300` | `timeout 300` 外壳命令 |
| CI 发现 | xtask app discovery 扫描 | 不被 CI 感知 |
| 结果收集 | `results/` 目录 | 同一目录但无 CI 汇总 |

Starry 测试可以被 `cargo xtask starry app qemu --test-case net-bench --all` 在矩阵中批量运行，而 Linux baseline 需要手动逐个调用。

### 欠缺六：预构建资源无完整性校验

| 资源 | 当前校验方式 | 缺失 |
|---|---|---|
| Alpine rootfs（xtask 管理） | SHA256（registry 下载时） | — |
| iperf3（prebuild apk） | apk 自身签名校验 | 无额外校验 |
| Linux vmlinuz（ensure_x86_kernel） | **无** | SHA256、版本记录、签名校验 |
| initramfs.cpio.gz | gzip 完整性 + `init` 存在性（`validate_initramfs`） | 内容校验（busybox 可执行、iperf3 依赖库完整） |

对比 xtask image registry 的标准流程：下载 → SHA256 校验 → 解包 → 文件锁保护并发访问。Linux 内核拉取没有走这个路径。

### 欠缺七：错误恢复和可观测性

- **Star 侧**：prebuild 时 apk 有 4 次重试退避；run.sh 有 `set -euo pipefail` 和 `nb_die` 前置检查。QEMU 超时后 CI 能捕获
- **Linux baseline 侧**：
  - `ensure_x86_kernel()` 无重试（`apt-get download` 失败即退出）
  - `prepare_linux_rootfs()` 中 iperf3 依赖库 `libcrypto.so*` 手工指定，无 `readelf` 自动解析（与 prebuild.sh 的 `copy_runtime_deps()` 不一致）
  - 若 Alpine rootfs 中的 iperf3 依赖链变化（如新增 `libssl.so*`），initramfs 将缺少库文件
  - 没有清理阶段（bridge/tap 由 `bin/setup` 创建，但 Linux baseline 脚本不感知状态管理）

## 改进方向（按优先级）

### P0：消除 xtask 内部路径依赖

`locate_alpine_image()` 不应硬编码 `tmp/axbuild/rootfs/`。方式：
- 通过 `cargo xtask starry rootfs --arch $ARCH --locate-only`（需新增 xtask 接口）获取路径
- 或复用 prebuild 阶段传入的环境变量（`STARRY_ROOTFS`、`STARRY_STAGING_ROOT`）

### P1：Linux 内核纳入 image registry

将 Linux guest 内核定义为 managed image，走 xtask 统一的下载 → SHA256 → 缓存 → 锁流程。固化版本号，确保基线可复现。

### P2：统一 Linux baseline 入口

实现 `cargo xtask starry app qemu --test-case net-bench-linux-baseline`：
- prebuild.sh 阶段准备 initramfs（复用 `copy_runtime_deps()` 的 `readelf` 依赖解析）
- QEMU toml 中定义 `shell_init_cmd`、`success_regex`、`fail_regex`
- 与 Starry 测试共享同一套 xtask 生命周期

### P3：消除 guest 测试逻辑重复

抽取测试矩阵为独立脚本（如 `core/test-matrix.sh`），Starry guest 和 Linux guest 共用同一份 `run_test()` 定义。

### P4：CI 集成对称化

Linux baseline 的 QEMU toml 加入 CI 矩阵，与 Starry 测试一并跑，结果一并汇总。

### P5：完善校验和重试

- `ensure_x86_kernel()` 增加重试和镜像回退
- initramfs 构建后用 `ldd` 或等效方式验证 iperf3 所有依赖库已包含
- 增加清理阶段的完整性检查

## 总结

net-bench 的 Starry 测试侧已具备较好的封装基础（xtask 集成、prebuild 依赖解析、状态化网络管理）。问题集中在 Linux baseline 侧：独立入口、裸资源拉取、硬编码内部路径、测试逻辑重复、CI 不可见。这些问题不阻塞功能，但限制了 net-bench 作为"网络性能测试产品"的易用性和可维护性。后续逐步对齐 nginx 级别的封装范式可解决大部分问题。
