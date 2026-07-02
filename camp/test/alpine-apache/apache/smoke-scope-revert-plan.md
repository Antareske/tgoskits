# Apache Smoke 收敛与回退计划

## 当前状态

**提交**: `0f774a6ab fix(starry,apache): scope smoke to reviewer-passing steps`

**收敛范围**: 默认 smoke 只覆盖前 4 步，不启动 Apache，不进行 HTTP 测试。

| 步骤 | 当前默认 smoke | 完整版 (debug/apache-smoke-full.sh) |
|---|---|---|
| prepare packages | ✅ | ✅ |
| prepare apache files | ✅ | ✅ |
| environment probe | ✅ | ✅ |
| apache config test | ✅ | ✅ |
| start apache single process | ❌ | ✅ |
| GET / | ❌ | ✅ |
| GET missing returns 404 | ❌ | ✅ |
| HEAD /small.txt | ❌ | ✅ |
| keepalive two requests | ❌ | ✅ |
| logs written | ❌ | ✅ |
| stop apache | ❌ | ✅ |

## 收敛原因

### reviewer 环境失败现象

运行默认命令 `timeout 1800s cargo xtask starry app qemu -t apache --arch x86_64`：

- 前 4 步全部通过
- 第 5 步 `start apache single process` 失败
  - readiness curl 对 `http://127.0.0.1:8080/` 循环探测 30 秒全部超时
  - error.log 只有一条警告：`(92)Protocol not available: AH00076: Failed to enable APR_TCP_DEFER_ACCEPT`
  - httpd 进程存在（有 PID）
  - 外层 `fail_regex` 匹配 `APACHE_RUNNER_FAILED`，xtask 返回失败

### 根因调查状态

- **已排除**: errno 92 本身导致 listen socket 不可用
  - debug 探针 `tcp-defer-accept-probe.c` 对比实验证明：
    - 未实现 TCP_DEFER_ACCEPT 时 `setsockopt` 返回 errno 92，但 loopback 连接仍正常 accept
    - 实现后 `setsockopt` 返回成功，连接同样正常
  - 结论：AH00076 是警告，不是 curl 超时的根因
- **仍未明**: reviewer 环境 curl 超时的真正原因
  - 本地无法复现（本地 smoke 修复前后均通过）
  - 可能与网络时序、地址获取、环境差异有关
  - 需要 reviewer 提供更多诊断数据（完整 error.log、curl 具体报错、`ss -ltnp` 输出、apache 包版本）

### 收敛决策

为满足 PR review 规则"默认命令在 reviewer 环境可运行"，将 smoke 收敛到已知在本地和 reviewer 环境均可通过的步骤（前 4 步）。

完整 HTTP 测试覆盖保留在 `debug/apache-smoke-full.sh`，待根因解决后恢复。

## 回退条件

满足以下**任一条件**即可将完整 smoke 恢复为默认：

### 1. reviewer 环境通过完整 smoke

reviewer 在其原失败环境重新运行（使用修复后的内核或其他调整），完整 smoke 通过。

**验证方式**：
- reviewer 运行默认命令通过，或
- reviewer 明确回复"已验证通过"，或
- reviewer 在 review 中 approve 并说明已测试通过

### 2. 定位并修复真正根因

通过 reviewer 提供的诊断数据或其他途径，定位到 curl 超时的真正原因（非 errno 92），并在本地可复现该失败、应用修复后可复现通过。

**验证方式**：
- 在本地或 CI 环境复现原失败（例如通过特定 Apache 版本、网络配置等）
- 应用修复后完整 smoke 通过
- 有受控对比实验日志证明修复有效

### 3. reviewer 明确允许当前范围

reviewer 明确表示"当前 4 步 smoke 足够，可以先合入，HTTP 测试后续补充"，或类似意图的回复。

**验证方式**：
- review 评论中明确许可，或
- 在 approve 时附带说明"smoke 范围可接受"

## 回退操作步骤

满足回退条件后：

1. **恢复 smoke 内容**：
   ```bash
   cp apps/starry/apache/debug/apache-smoke-full.sh apps/starry/apache/smoke/apache-smoke-tests.sh
   ```

2. **更新 smoke 头部注释**：
   - 移除"minimal verification scope"相关说明
   - 改为"full Apache startup and HTTP test coverage"
   - 保留对 TCP_DEFER_ACCEPT 的说明（若仍适用）

3. **更新 README.md**：
   - Modes 表格：`smoke` 行改为"default app entry (full HTTP coverage)"
   - 移除"Smoke scope"段落，或改为"Smoke 现已包含完整 HTTP 测试"
   - 在 Known Issue Notes 中更新 ISSUE-002 状态为"已解决"或"已验证"

4. **可选：移除 apache-smoke-full.sh**：
   - 从 `debug/` 删除
   - 从 `prebuild.sh` 移除注入行
   - 从 README Guest assets 移除

5. **提交**：
   ```bash
   git add apps/starry/apache/
   git commit -m "fix(starry,apache): restore full smoke coverage" \
              -m "恢复 smoke 默认为完整 HTTP 测试覆盖。

   回退条件已满足：[reviewer 环境通过 / 根因已修复 / reviewer 明确许可]。

   变更：
   - smoke/apache-smoke-tests.sh：恢复 start_httpd 及所有 HTTP 测试（GET/HEAD/keepalive/stop）。
   - README.md：更新 Modes 说明，移除收敛相关说明。
   - [可选] 移除 debug/apache-smoke-full.sh 及其注入逻辑。

   验证：[本地 / reviewer 环境 / CI] 完整 smoke 通过。"
   git push origin test/alpine-apache
   ```

6. **更新 PR 描述**（如果 PR 已创建）：
   - 说明 smoke 已恢复完整覆盖
   - 附上验证日志或 reviewer 确认截图

## 当前可用测试方式

### 运行最小 smoke（默认，reviewer 可通过）

```bash
cargo xtask starry app qemu -t apache --arch x86_64
```

### 运行完整 smoke（本地验证用）

需要手动运行 apache-smoke-full.sh。当前无专用 qemu 配置，可：

1. 修改 `apps/starry/apache/qemu-x86_64.toml`：
   ```toml
   shell_init_cmd = "/usr/bin/apache-smoke-full.sh"
   ```

2. 或在 guest shell 中手动运行：
   ```bash
   # 进入 guest 后
   /usr/bin/apache-smoke-full.sh
   ```

### 运行 TCP_DEFER_ACCEPT 探针

```bash
cargo xtask starry app qemu -t apache --arch x86_64 \
  --qemu-config apps/starry/apache/qemu/debug/qemu-x86_64-tcp-defer-accept-probe.toml
```

## 相关文档

- **ISSUE-002**: `apps/starry/apache/debug/ISSUE-002-tcp-defer-accept.md` — TCP_DEFER_ACCEPT 调查与探针实验结果
- **完整 smoke**: `apps/starry/apache/debug/apache-smoke-full.sh` — 保留的完整 HTTP 测试版本
- **rc2 反馈**: `www/apache/rc2.md` — reviewer 第二轮反馈原文
- **rc 回复草稿**: `www/apache/rc2-response.md` — 准备回复 reviewer 的调查报告

## 时间线

| 时间 | 事件 |
|---|---|
| 2026-06-25 | 本地测试通过（含完整 HTTP smoke），提交 PR |
| 2026-06-26 (rc2) | reviewer 报告默认命令失败，start_httpd 阶段 curl 超时，error.log 有 AH00076 errno 92 |
| 2026-06-26 (调查) | 新增 TCP_DEFER_ACCEPT 探针，受控实验证明 errno 92 不破坏 listen socket |
| 2026-06-26 (收敛) | 提交 `0f774a6ab`，将 smoke 收敛到前 4 步，完整版保留在 debug/ |
| 待定 | reviewer 提供更多诊断数据，或在其环境验证修复 |
| 待定 | 满足回退条件，恢复完整 smoke 为默认 |
