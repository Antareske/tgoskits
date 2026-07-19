# nginx smoke request change 响应

本说明回应上一轮 review（基于 `www/rc`）提出的阻塞点与检查项，并补充 x86_64 启动问题、以及 `apps/starry` 下其它 app 可能尚未对齐当前测试框架的情况。

## 1. 阻塞点：keepalive 误判通过（已修复）

### 评审意见

> keepalive 步骤输出了 `punt!` 后仍继续打印 `NGINX_APP_STEP_PASS: keepalive two requests`，脚本在没有拿到有效 HTTP 响应时仍上报通过，不能作为可靠断言。

根因：旧实现依赖 rootfs 上的 busybox `nc`，该 `nc` 在此 rootfs 上只打印 `punt!` 即退出，脚本未对空输出做校验便 `return 0`，从而把真实失败吞成通过。

### 改动

`apps/starry/nginx/smoke/nginx-smoke-tests.sh` 的 `test_keepalive_two_requests` 已完全弃用 busybox `nc`，改用 curl（本身已是 smoke 的硬依赖）发起 keepalive 断言：

- 一条 curl 命令对 `/small.txt`、`/empty.txt` 两个 URL 复用单连接，逐次输出 `http_code` 与 `num_connects`。
- curl 返回错误即 `return 1` 并 dump 原始输出。
- 必须恰好匹配到 **两个 `http_code=200`**，否则失败。
- `num_connects` 合计必须为 **1**（即两请求复用同一 TCP 连接），否则失败。

任何空输出、非 200 响应、连接复用异常都会显式失败并打印诊断，不再有 false-positive。这正面满足评审建议的“只在真实收到两次 200 时通过，否则显式 fail 并打印原始输出”。

相关提交：`c8577c69e`（keepalive 重写、apk 镜像源固定、移除 coreutils）。

## 2. 其它已响应的检查项

- **apk 镜像源漂移**：`nginx-alpine-mirror.sh` 改为读取 rootfs 的 `/etc/alpine-release` 锁定 Alpine 分支，不再用会随时间漂移的 `latest-stable`，并保留 `NGINX_APK_BRANCH` 覆盖。
- **多余依赖**：移除了 `coreutils`，smoke 不再安装无关包。
- **README 接口**：nginx 命令使用当前真实的 `cargo xtask starry app qemu` 接口，引用的配置文件均真实存在。
- **默认配置缺失（上一轮已 resolve）**：`qemu-aarch64.toml` / `qemu-loongarch64.toml` 已提供，相关旧 review 线程已标记 resolved。

## 3. 构建配置规整与 loongarch64 内存（本轮提交 `cd5ed62f4`）

把 `aarch64` / `loongarch64` 构建配置对齐到 `apps/starry` 现行约定（以 `redis` / `mariadb` 为参照），并解决 loongarch64 内存不足：

- **aarch64**：删除未使用的 `env = { AX_IP, AX_GW }` 与冗余的 `plat_dyn = true`（默认即 dynamic，行为不变）。
- **loongarch64**：删除未使用的 `env`；补齐 `ax-hal/loongarch64-qemu-virt`、`ax-driver/plat-static`、`plat_dyn = false`，与 redis/mariadb 的 static 平台写法一致；新增 `axconfig_overrides = ["plat.phys-memory-size=0x8000_0000"]`（2GB）。
- **loongarch64 QEMU**：`-m` 由 `1G` 提升至 `2G`。

loongarch64 走 static 平台，内核识别的物理内存上限取自 axconfig 的 `plat.phys-memory-size`，而非 QEMU `-m`；故内核侧 `phys-memory-size` 与 QEMU `-m` 需同步放大，与 mariadb 的做法一致。x86_64 / riscv64 未改动（x86_64 与 redis 一致，riscv64 是对 redis 的合理裁剪）。

## 4. x86_64 启动问题（变基后的问题，当前未修复），以下系 AI 分析结果

评审在 x86_64 上能跑到 `NGINX_APP_SMOKE_PASSED`，但需说明：在当前 `dev` 上，`cargo xtask starry app qemu --arch x86_64` 存在一个**与 nginx 无关**的启动层问题。

```bash
qemu-system-x86_64: Error loading uncompressed kernel without PVH ELF Note

✓ 已退出串口终端模式
Error: qemu-system-x86_64: Error loading uncompressed kernel without PVH ELF Note
```

- 现象：x86_64 经 app-qemu 路径以纯 `-kernel` 加载 dynamic 内核时报
  `Error loading uncompressed kernel without PVH ELF Note`。
- 根因定位（非 nginx 测例可解）：
  - `dev` 默认 dynamic 平台构建（`scripts/axbuild/src/build.rs` 的 `default_plat_dyn`），且 x86_64 的 qemu 配置 `to_bin = false` 走 ELF `-kernel`，QEMU 要求 PVH ELF Note。
  - dynamic 平台所需的 UEFI 启动改写在 `apply_dynamic_platform_qemu_boot`（`scripts/axbuild/src/test/qemu.rs`，会设 `uefi=true`/`to_bin=true` 及 x86_64 专属总线/分页/设备调整）。
  - 该函数仅在 test / rootfs 路径被调用，**app-qemu 派发路径（`scripts/axbuild/src/context/mod.rs` 的 `qemu()`）未调用它**，因此 dynamic x86_64 在 `app qemu` 下无法正确进入 UEFI 启动。
- 验证为框架级而非本 PR 引入：用 `dev` 自带、未改动的 redis 跑 `cargo xtask starry app qemu --arch x86_64` 复现同样的 PVH 报错。
- 为什么纯靠测例 toml 解决不了：dynamic x86_64 的启动配置（UEFI 驱动总线、关五级分页、关默认设备、嵌套虚拟化等）是 xtask 运行时程序化生成的，toml 没有对应声明字段；仅在 `qemu-x86_64.toml` 写 `uefi=true`/`to_bin=true` 得到的是残缺配置，仍无法启动。

结论：x86_64 的 PVH 启动属 app-qemu 路径的框架层回归，建议作为独立 issue/PR 在 `scripts/axbuild` 修复（在 app-qemu 路径补一次 `apply_dynamic_platform_qemu_boot`），不在本 nginx 测试入口整理范围内。nginx 仍保留可发现的根目录 `qemu-x86_64.toml` smoke 入口；若当前 dev 的 app-qemu x86 dynamic 启动仍未修复，失败原因应归属该框架层问题。

## 5. 其它 app 可能尚未对齐当前测试框架

排查过程中发现该框架层问题影响面不止 nginx，相关风险供后续跟进：

- **dynamic x86_64 的 app-qemu 启动**：任何依赖默认 dynamic 平台、且经 `app qemu --arch x86_64` 运行的 Starry app 都会遇到同一 PVH 启动问题（已用 redis 复现）。这是 `dev` 切换默认 dynamic 后，app-qemu 路径未同步补齐 UEFI 启动改写导致的，需在框架层统一修复。
- **构建配置约定漂移**：本 PR 中 nginx 的 aarch64/loongarch64 曾残留 `env = { AX_IP, AX_GW }`、显式 `plat_dyn` 等过时写法。`apps/starry` 下其它较早加入的 app 可能同样存在与当前 redis/mariadb 约定不一致的构建配置（多余 env、loongarch64 缺 `plat-static`/`loongarch64-qemu-virt`、内存设定方式不统一等），建议后续做一次统一审计对齐。

以上两点不在本 PR 处理范围，仅作为相邻风险面记录，便于后续单独立项。

## 验证方式

四个架构的 smoke 测试命令：

```bash
cargo xtask starry app qemu -t nginx --arch riscv64
cargo xtask starry app qemu -t nginx --arch aarch64
cargo xtask starry app qemu -t nginx --arch loongarch64
cargo xtask starry app qemu -t nginx --arch x86_64
```

- riscv64 / aarch64 / loongarch64 走裸二进制启动，不受 PVH 影响，可完整跑完 smoke。
- x86_64 使用可发现的根目录 `qemu-x86_64.toml` smoke 入口；若当前 `dev` 上仍触发上述 PVH 启动问题，归属 app-qemu 框架层问题。
- CI 的 `cargo xtask starry app qemu --all --arch <arch>` 仅选取各架构对应的 `qemu-<arch>.toml` 作为入口。



历史说明：曾短暂尝试隐藏 x86 nginx configs from discovery；当前已回退，根目录 `qemu-x86_64.toml` 是可发现 smoke 入口。
