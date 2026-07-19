# net-bench 重构计划：三层架构与板载测试迁移

## 目标

在当前 net-bench 实现的基础上，引入三层架构设计，解决当前封装问题并为未来迁移到真实开发板测试做好准备。

## 架构设计

### 整体流程（俯视图）

```
用户入口
   ↓
统一命令层 (xtask)
   ↓
测试编排层 (orchestrator) ← 读取测试矩阵配置
   ↓
环境适配层 (adapter) ← 根据 --env 选择具体实现
   ↓
执行环境 (QEMU/板载) → 运行测试 → 产生原始输出
   ↓
结果收集层 (adapter.collect)
   ↓
统一结果格式 (JSON schema)
   ↓
CI 汇总 / 用户查看
```

### 三层架构详解

#### 第一层：测试编排层（Environment-agnostic）

**职责**：定义"测什么"，与执行环境无关

```
net-bench/
  core/
    test-matrix.yaml          # 声明式测试定义
    orchestrator.sh           # 编排引擎
    result-schema.json        # 统一的结果格式
```

**test-matrix.yaml 示例**：

```yaml
# 测试矩阵定义
tests:
  - id: tcp64-c2s
    protocol: tcp
    parallel: 64
    duration: 10
    direction: client-to-server
    
  - id: tcp64-s2c
    protocol: tcp
    parallel: 64
    duration: 10
    direction: server-to-client
    
  - id: udp64
    protocol: udp
    parallel: 64
    bandwidth: 10G
    duration: 10

# 支持的执行环境
environments:
  - qemu-starry      # 当前 QEMU + StarryOS
  - qemu-linux       # 当前 Linux baseline
  - board-starry     # 未来：真实开发板
```

**orchestrator.sh 流程**：

```bash
# 编排引擎伪代码
orchestrator_run() {
  local env=$1  # qemu-starry / qemu-linux / board-starry
  
  # 加载对应环境的 adapter
  source "adapters/${env}.sh"
  
  # 标准生命周期
  adapter_setup      # 准备执行环境
  adapter_deploy     # 部署测试资源（iperf3、脚本）
  
  # 遍历测试矩阵
  while IFS= read -r test; do
    local test_id=$(jq -r '.id' <<< "$test")
    adapter_execute "$test_id"  # 执行单个测试
  done < <(jq -c '.tests[]' test-matrix.yaml)
  
  adapter_collect    # 收集结果
  adapter_teardown   # 清理环境
}
```

**关键设计原则**：
- orchestrator 只知道测试 ID 和参数
- 不知道是 QEMU 还是板载
- 不知道用 SSH 还是串口
- 所有环境特定逻辑都在 adapter 层

---

#### 第二层：环境适配层（Adapter pattern）

**职责**：定义"怎么测"，将统一接口翻译成环境特定操作

每种环境实现 **5 个标准方法**：

```bash
# adapters/interface.sh - 所有 adapter 必须实现的接口

adapter_setup()      # 启动/连接执行环境
adapter_deploy()     # 部署测试资源（iperf3、脚本）
adapter_execute()    # 执行测试矩阵中的一个测试
adapter_collect()    # 收集结果和日志
adapter_teardown()   # 清理资源
```

**三种 adapter 实现对比**：

| 操作 | qemu-starry | qemu-linux | board-starry（未来） |
|------|-------------|------------|---------------------|
| **setup** | 启动 QEMU + StarryOS | 启动 QEMU + Linux | 检查板载 IP 可达 + 串口连接 |
| **deploy** | rootfs overlay 注入 | initramfs 打包注入 | scp 到板载 /tmp/ |
| **execute** | QEMU 内运行 iperf3 | QEMU 内运行 iperf3 | ssh 到板载运行 iperf3 |
| **collect** | 读 QEMU stdout | 读 QEMU stdout | 读 ssh 日志 + 串口日志 |
| **teardown** | pkill qemu-system | pkill qemu-system | 可选：PDU 断电重启 |

**adapter 示例（qemu-starry）**：

```bash
# adapters/qemu-starry.sh

adapter_setup() {
  # 通过 xtask 获取资源
  cargo xtask starry image fetch alpine-x86_64
  cargo xtask starry rootfs --arch "$ARCH"
  
  # prebuild：apk 安装 iperf3 到 overlay
  qemu-user apk add iperf3
  
  # 构建 StarryOS 内核
  cargo build --target "$ARCH"
  
  # 启动 QEMU
  qemu-system-x86_64 -kernel ... -initrd ... -netdev tap,...
  QEMU_PID=$!
}

adapter_deploy() {
  # 已通过 rootfs overlay 部署，无额外操作
  :
}

adapter_execute() {
  local test_id=$1
  # 在 QEMU 内执行测试
  /tmp/net-bench-common.sh run_test "$test_id"
}

adapter_collect() {
  # 从 QEMU stdout 提取性能数据，转换为 JSON
  parse_iperf3_output > "results/${ENV}-${ARCH}-${test_id}.json"
}

adapter_teardown() {
  pkill -P "$QEMU_PID"
  ip link del tap0
}
```

**adapter 示例（board-starry，未来）**：

```bash
# adapters/board-starry.sh

adapter_setup() {
  # 不启动 QEMU，检查板载连接
  ssh root@"$BOARD_IP" 'uptime' || nb_die "Board not reachable"
  screen -dmS serial /dev/ttyUSB0  # 启动串口监控
  
  # 不需要构建内核（板载已烧录 StarryOS）
}

adapter_deploy() {
  # 通过网络部署（不是 rootfs overlay）
  scp iperf3 root@"$BOARD_IP":/bin/
  scp test-matrix.sh root@"$BOARD_IP":/tmp/
}

adapter_execute() {
  local test_id=$1
  # 通过 SSH 执行
  ssh root@"$BOARD_IP" "/tmp/test-matrix.sh run_test $test_id"
  
  # 同时监控串口日志（防止 SSH 断开时丢失输出）
  tail -f /tmp/serial.log | grep -E 'NET_BENCH_PASSED|panic'
}

adapter_collect() {
  # 从 SSH 输出 + 串口日志提取数据
  parse_ssh_output > "results/${ENV}-${ARCH}-${test_id}.json"
}

adapter_teardown() {
  # 可选：通过 PDU 硬重启板子
  if [ -n "$POWER_CONTROL" ]; then
    curl -X POST "http://pdu.local/outlet/3/reboot"
  fi
}
```

---

#### 第三层：资源管理层（xtask image registry）

**职责**：管理所有外部依赖（内核、rootfs、工具二进制）

```
.tgos-images/
  ├─ alpine-x86_64-3.19.1/       ← Alpine rootfs
  ├─ linux-kernel-6.8.0/         ← Linux guest 内核
  ├─ iperf3-3.16-x86_64/         ← x86_64 iperf3
  └─ iperf3-3.16-aarch64-static/ ← 板载用静态链接版本
```

**registry 配置示例**：

```toml
# xtask/src/image/registry.toml

[[image]]
name = "linux-kernel-x86_64"
version = "6.8.0-generic"
url = "https://mirrors.tuna.tsinghua.edu.cn/ubuntu/pool/..."
sha256 = "a1b2c3d4..."
extract_to = ".tgos-images/linux-kernels/"

[[image]]
name = "iperf3-aarch64-static"
version = "3.16"
url = "https://github.com/esnet/iperf/releases/..."
sha256 = "e5f6g7h8..."
```

**提供的能力**：
- SHA256 校验（防止下载损坏）
- 版本固化（可复现）
- 并发锁（多 CI job 并行安全）
- 重试 + 镜像回退（网络容错）

**adapter 使用方式**：

```bash
# adapter 不直接下载资源
# wget https://...  ❌

# 通过 registry 统一获取
KERNEL_PATH=$(cargo xtask starry image fetch linux-kernel-x86_64 --print-path)
qemu-system-x86_64 -kernel "$KERNEL_PATH" ...
```

---

## 数据流

```
test-matrix.yaml (声明式测试定义)
    ↓
orchestrator 读取测试列表
    ↓
adapter.execute("tcp64-c2s") 
    ↓
执行环境内运行 iperf3
  - QEMU 内运行（qemu-starry/qemu-linux）
  - 板载通过 SSH 运行（board-starry）
    ↓
原始输出（stdout / 串口 / ssh 日志）
    ↓
adapter.collect() 解析
    ↓
统一 JSON schema
    ↓
results/
  ├─ qemu-starry-x86_64-tcp64-c2s.json
  ├─ qemu-linux-x86_64-tcp64-c2s.json
  └─ board-starry-aarch64-tcp64-c2s.json
    ↓
CI 汇总对比（Starry vs Linux，QEMU vs 板载）
```

---

## 统一结果格式（JSON schema）

```json
{
  "environment": "qemu-starry",
  "arch": "x86_64",
  "timestamp": "2026-07-18T14:30:00Z",
  "fingerprint": {
    "kernel_version": "starry-0.1.0",
    "kernel_commit": "2529b4d6a",
    "iperf3_version": "3.16",
    "network_topology": "vhost",
    "qemu_version": "8.2.0"
  },
  "tests": [
    {
      "id": "tcp64-c2s",
      "status": "passed",
      "throughput_gbps": 8.2,
      "cpu_percent": 45.3,
      "retransmits": 12,
      "duration_sec": 10.5,
      "raw_output": "...",
      "timestamp": "2026-07-18T14:30:15Z"
    }
  ]
}
```

所有环境产生相同格式的 JSON，CI 可以跨环境对比。

---

## 完整流程示例

### 当前 QEMU-Starry 测试

```bash
$ cargo xtask starry app test net-bench --env qemu-starry --arch x86_64
```

**执行步骤**：

1. **xtask 入口** → 解析 `--env qemu-starry`，加载 `adapters/qemu-starry.sh`
2. **orchestrator** → 读取 `test-matrix.yaml`
3. **adapter_setup** → 启动 QEMU + StarryOS
4. **adapter_deploy** → rootfs overlay 注入 iperf3
5. **遍历测试矩阵** → 执行 tcp64-c2s、tcp64-s2c、udp64 等
6. **adapter_collect** → 解析输出为 JSON
7. **adapter_teardown** → 清理 QEMU 和网络资源
8. **结果输出** → `results/qemu-starry-x86_64-*.json`

### 未来板载测试

```bash
$ cargo xtask starry app test net-bench --env board-starry --board-ip 192.168.1.100
```

**执行步骤**（差异部分）：

3. **adapter_setup** → 检查板载 SSH 可达 + 启动串口监控
4. **adapter_deploy** → scp iperf3 到板载 /tmp/
5. **遍历测试矩阵** → 通过 SSH 执行测试
6. **adapter_collect** → 从 SSH 输出 + 串口日志提取数据
7. **adapter_teardown** → 可选：PDU 断电重启板子

**关键**：orchestrator 和 test-matrix.yaml 完全不变。

---

## 关键设计优势

### 1. 测试逻辑只写一次
- `test-matrix.yaml` 定义测试，所有环境共用
- 新增 tcp128 测试时，三种环境自动支持
- 当前的 `run_test()` 在 Starry guest 和 Linux guest 中重复维护 → 消除

### 2. 适配新环境只需实现 5 个方法
- 添加板载支持：创建 `adapters/board-starry.sh`
- 不需要修改 orchestrator、test-matrix、结果收集
- 当前的双入口割裂（run.sh vs run-linux-baseline.sh）→ 统一

### 3. 结果格式统一
- QEMU 和板载的结果可以直接对比
- CI 用同一套脚本汇总所有环境
- 当前的 CI 集成不对称 → 对称化

### 4. 资源管理集中化
- 所有依赖走 image registry，SHA256 + 版本固化
- adapter 不关心下载逻辑，只关心"资源在哪"
- 当前的 Linux 内核裸 apt-get 下载 → 纳入体系

### 5. CI 矩阵化
```yaml
# .github/workflows/net-bench.yml
strategy:
  matrix:
    environment: [qemu-starry, qemu-linux, board-starry]
    arch: [x86_64, riscv64, aarch64]
```

所有环境用同一条命令入口，矩阵自动展开。

---

## 高优先级增强事项

基于当前实现和三层架构设计，以下是优先级排序的改进任务。

---

### P0：消除 xtask 内部路径依赖

**问题**：`run-linux-baseline.sh` 的 `locate_alpine_image()` 硬编码了 xtask 内部路径：

```bash
local flat="$WORKSPACE/tmp/axbuild/rootfs/$image_name"
local nested="$WORKSPACE/tmp/axbuild/rootfs/$image_name/$image_name"
```

这是 xtask `image::storage` 模块的内部实现细节，不稳定。

**解决方案**：

方案 A：新增 xtask 接口
```bash
# 让 xtask 提供稳定的查询接口
ROOTFS_PATH=$(cargo xtask starry rootfs --arch x86_64 --locate-only)
```

方案 B：通过环境变量传递（推荐）
```bash
# xtask 在调用 adapter 前设置环境变量
export STARRY_ROOTFS=/path/to/alpine-x86_64.ext4
export STARRY_OVERLAY_DIR=/path/to/overlay

# adapter 直接使用
locate_alpine_image() {
  echo "$STARRY_ROOTFS"
}
```

**优先级理由**：阻止未来 xtask 重构时脚本失效，且实现简单。

**预计工作量**：2-4 小时

---

### P1：抽取测试矩阵为声明式配置

**问题**：测试逻辑在两处重复维护：

```bash
core/net-bench-common.sh       # Starry guest 侧
run-linux-baseline.sh          # Linux guest init 脚本内嵌
```

新增测试用例需要两处修改。

**解决方案**：

创建 `core/test-matrix.yaml`：

```yaml
tests:
  - id: tcp64-c2s
    protocol: tcp
    parallel: 64
    duration: 10
    port: 5201
    direction: client-to-server
    iperf3_args: "-c 10.0.2.2 -P 64 -t 10"
    
  - id: tcp64-s2c
    protocol: tcp
    parallel: 64
    duration: 10
    port: 5201
    direction: server-to-client
    iperf3_args: "-c 10.0.2.2 -P 64 -t 10 -R"
    
  - id: udp64
    protocol: udp
    parallel: 64
    duration: 10
    port: 5201
    bandwidth: 10G
    iperf3_args: "-c 10.0.2.2 -P 64 -t 10 -u -b 10G"
```

创建统一的测试执行器 `core/run-matrix.sh`：

```bash
#!/bin/bash
# core/run-matrix.sh - 从 YAML 读取测试矩阵并执行

run_test_from_matrix() {
  local test_id=$1
  local matrix_file=${2:-"core/test-matrix.yaml"}
  
  # 解析 YAML（使用 yq 或 python）
  local test_config=$(yq eval ".tests[] | select(.id == \"$test_id\")" "$matrix_file")
  local iperf3_args=$(echo "$test_config" | yq eval '.iperf3_args' -)
  
  # 执行测试（环境无关）
  if is_server_mode; then
    iperf3 -s -p 5201 &
    SERVER_PID=$!
  fi
  
  iperf3 $iperf3_args
  
  # 输出标准格式结果
  echo "NET_BENCH_TEST_RESULT: $test_id"
}
```

Starry guest 和 Linux guest 都调用同一个 `run-matrix.sh`。

**优先级理由**：为三层架构铺路，消除重复维护，新增测试用例时一次性生效。

**预计工作量**：6-8 小时（包括 YAML 解析和测试验证）

---

### P2：Linux 内核纳入 image registry

**问题**：`ensure_x86_kernel()` 直接用 `apt-get download` 拉取 Linux 内核：

- 无 SHA256 校验
- 无版本固化（拉取的是"当前最新"，不可复现）
- 无重试/回退
- 无并发锁

**解决方案**：

在 `xtask/src/image/registry.toml` 中定义：

```toml
[[image]]
name = "linux-kernel-x86_64"
version = "6.8.0-45-generic"
url = "https://mirrors.tuna.tsinghua.edu.cn/ubuntu/pool/main/l/linux/linux-image-6.8.0-45-generic_6.8.0-45.45_amd64.deb"
sha256 = "a1b2c3d4e5f6..."
extract_to = ".tgos-images/linux-kernels/"
extract_filter = "boot/vmlinuz-*"  # 只提取内核文件

[[image]]
name = "linux-kernel-riscv64"
version = "6.8.0-45-generic"
url = "https://mirrors.tuna.tsinghua.edu.cn/ubuntu/pool/main/l/linux/linux-image-6.8.0-45-generic_6.8.0-45.45_riscv64.deb"
sha256 = "f6e5d4c3b2a1..."
```

在 `run-linux-baseline.sh` 中使用：

```bash
ensure_x86_kernel() {
  # 通过 registry 获取
  local kernel_path=$(cargo xtask starry image fetch linux-kernel-x86_64 --print-path)
  
  # 找到 vmlinuz 文件
  LINUX_KERNEL=$(find "$kernel_path" -name "vmlinuz-*" | head -1)
  
  [ -f "$LINUX_KERNEL" ] || nb_die "Kernel not found in registry"
}
```

**优先级理由**：确保 Linux baseline 可复现，与 Starry 测试使用同样的资源管理标准。

**预计工作量**：4-6 小时（包括 registry 配置和测试）

---

### P3：统一结果格式为 JSON schema

**问题**：当前结果格式是人类可读的文本，不便于 CI 自动化对比。

**解决方案**：

定义 `core/result-schema.json`：

```json
{
  "$schema": "http://json-schema.org/draft-07/schema#",
  "type": "object",
  "required": ["environment", "arch", "timestamp", "tests"],
  "properties": {
    "environment": {"type": "string", "enum": ["qemu-starry", "qemu-linux", "board-starry"]},
    "arch": {"type": "string"},
    "timestamp": {"type": "string", "format": "date-time"},
    "fingerprint": {
      "type": "object",
      "properties": {
        "kernel_version": {"type": "string"},
        "kernel_commit": {"type": "string"},
        "iperf3_version": {"type": "string"},
        "network_topology": {"type": "string"},
        "qemu_version": {"type": "string"}
      }
    },
    "tests": {
      "type": "array",
      "items": {
        "type": "object",
        "required": ["id", "status"],
        "properties": {
          "id": {"type": "string"},
          "status": {"type": "string", "enum": ["passed", "failed", "skipped"]},
          "throughput_gbps": {"type": "number"},
          "cpu_percent": {"type": "number"},
          "retransmits": {"type": "integer"},
          "duration_sec": {"type": "number"},
          "raw_output": {"type": "string"}
        }
      }
    }
  }
}
```

在 `core/lib.sh` 中新增结果转换函数：

```bash
# 解析 iperf3 输出为 JSON
parse_iperf3_to_json() {
  local test_id=$1
  local iperf3_output=$2
  
  # 提取关键指标（通过 jq 解析 iperf3 的 JSON 输出）
  local throughput=$(echo "$iperf3_output" | jq -r '.end.sum_received.bits_per_second / 1e9')
  local retransmits=$(echo "$iperf3_output" | jq -r '.end.sum_sent.retransmits // 0')
  
  # 生成结果 JSON
  jq -n \
    --arg env "$BENCH_ENV" \
    --arg arch "$ARCH" \
    --arg test_id "$test_id" \
    --argjson throughput "$throughput" \
    --argjson retransmits "$retransmits" \
    '{
      environment: $env,
      arch: $arch,
      timestamp: (now | strftime("%Y-%m-%dT%H:%M:%SZ")),
      tests: [{
        id: $test_id,
        status: "passed",
        throughput_gbps: $throughput,
        retransmits: $retransmits
      }]
    }'
}
```

**优先级理由**：为 CI 自动化对比铺路，便于跨环境性能回归检测。

**预计工作量**：6-8 小时（包括 schema 定义和解析逻辑）

---

### P4：实现 adapter 接口框架

**问题**：当前双入口割裂，无统一抽象。

**解决方案**：

创建 adapter 目录结构：

```
net-bench/
  adapters/
    interface.sh          # 接口定义（文档）
    qemu-starry.sh        # 重构现有 run.sh
    qemu-linux.sh         # 重构现有 run-linux-baseline.sh
    board-starry.sh.template  # 未来板载的模板
```

`adapters/interface.sh`（纯文档）：

```bash
#!/bin/bash
# adapters/interface.sh - Adapter 接口规范（所有 adapter 必须实现）

# adapter_setup - 启动/连接执行环境
# 输出：环境已就绪（QEMU 启动 / 板载连接确认）
adapter_setup() {
  : # 子类实现
}

# adapter_deploy - 部署测试资源（iperf3、测试脚本）
# 输出：资源已部署到执行环境
adapter_deploy() {
  : # 子类实现
}

# adapter_execute <test_id> - 执行单个测试
# 参数：$1 = test_id（如 tcp64-c2s）
# 输出：测试原始输出
adapter_execute() {
  : # 子类实现
}

# adapter_collect - 收集结果和日志
# 输出：results/*.json（符合 result-schema.json）
adapter_collect() {
  : # 子类实现
}

# adapter_teardown - 清理资源
# 输出：环境已清理（QEMU 停止 / 网络资源释放）
adapter_teardown() {
  : # 子类实现
}
```

`adapters/qemu-starry.sh`（重构现有逻辑）：

```bash
#!/bin/bash
# adapters/qemu-starry.sh - QEMU + StarryOS adapter

source "$(dirname "$0")/interface.sh"
source core/lib.sh

adapter_setup() {
  nb_log "Setting up QEMU-Starry environment..."
  
  # 复用现有的 prebuild + run 逻辑
  cargo xtask starry rootfs --arch "$ARCH"
  bash prebuild.sh
  
  # 启动 QEMU（从 run.sh 提取）
  # ...
}

adapter_deploy() {
  # rootfs overlay 已在 prebuild 阶段完成
  nb_log "Deployment complete (via rootfs overlay)"
}

adapter_execute() {
  local test_id=$1
  nb_log "Executing test: $test_id"
  
  # 在 QEMU 内执行（通过 shell_init_cmd 或监控 stdout）
  # 调用 core/run-matrix.sh run_test "$test_id"
}

adapter_collect() {
  nb_log "Collecting results..."
  
  # 从 QEMU stdout 提取结果，转换为 JSON
  parse_iperf3_to_json "tcp64-c2s" "$stdout" > results/qemu-starry-x86_64-tcp64-c2s.json
}

adapter_teardown() {
  nb_log "Tearing down QEMU..."
  pkill -P "$QEMU_PID"
  ip link del tap0 2>/dev/null || true
}
```

`adapters/board-starry.sh.template`（未来实现的模板）：

```bash
#!/bin/bash
# adapters/board-starry.sh.template - 真实开发板 adapter 模板

source "$(dirname "$0")/interface.sh"
source core/lib.sh

# 配置（从环境变量或配置文件读取）
BOARD_IP=${BOARD_IP:-"192.168.1.100"}
BOARD_SERIAL=${BOARD_SERIAL:-"/dev/ttyUSB0"}
POWER_CONTROL=${POWER_CONTROL:-""}  # 可选：PDU 地址

adapter_setup() {
  nb_log "Checking board connectivity..."
  
  # 检查 SSH 可达性
  if ! ssh -o ConnectTimeout=5 root@"$BOARD_IP" true; then
    nb_die "Board at $BOARD_IP not reachable via SSH"
  fi
  
  # 启动串口监控（后台）
  if [ -e "$BOARD_SERIAL" ]; then
    screen -dmS board-serial "$BOARD_SERIAL"
    nb_log "Serial monitor started on $BOARD_SERIAL"
  fi
  
  nb_log "Board ready at $BOARD_IP"
}

adapter_deploy() {
  nb_log "Deploying resources to board..."
  
  # 通过 scp 部署
  scp iperf3 root@"$BOARD_IP":/bin/ || nb_die "Failed to deploy iperf3"
  scp core/run-matrix.sh root@"$BOARD_IP":/tmp/ || nb_die "Failed to deploy test script"
  scp core/test-matrix.yaml root@"$BOARD_IP":/tmp/ || nb_die "Failed to deploy test matrix"
  
  nb_log "Deployment complete"
}

adapter_execute() {
  local test_id=$1
  nb_log "Executing test on board: $test_id"
  
  # 通过 SSH 执行测试
  ssh root@"$BOARD_IP" "/tmp/run-matrix.sh run_test $test_id" > /tmp/board-output.log 2>&1
  
  # 同时监控串口日志（捕获 panic 等异常）
  if [ -e "$BOARD_SERIAL" ]; then
    timeout 30 screen -X -S board-serial hardcopy /tmp/serial.log
  fi
}

adapter_collect() {
  nb_log "Collecting results from board..."
  
  # 从 SSH 输出提取结果
  parse_iperf3_to_json "$test_id" "$(cat /tmp/board-output.log)" > results/board-starry-aarch64-"$test_id".json
  
  # 保存串口日志
  if [ -e /tmp/serial.log ]; then
    cp /tmp/serial.log results/board-serial-"$test_id".log
  fi
}

adapter_teardown() {
  nb_log "Cleaning up board..."
  
  # 停止串口监控
  screen -X -S board-serial quit 2>/dev/null || true
  
  # 可选：硬重启板子（清理残留进程）
  if [ -n "$POWER_CONTROL" ]; then
    nb_log "Power cycling board via $POWER_CONTROL"
    curl -X POST "$POWER_CONTROL/reboot"
    sleep 30
  fi
  
  nb_log "Teardown complete"
}
```

**优先级理由**：为三层架构核心，实现后可以逐步迁移现有逻辑，且为板载测试预留接口。

**预计工作量**：12-16 小时（包括接口定义、两个 adapter 重构、测试验证）

---

### P5：创建统一入口（orchestrator）

**问题**：当前需要手动选择 run.sh 或 run-linux-baseline.sh。

**解决方案**：

创建 `core/orchestrator.sh`：

```bash
#!/bin/bash
# core/orchestrator.sh - 环境无关的测试编排引擎

set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
source "$SCRIPT_DIR/lib.sh"

usage() {
  cat <<EOF
Usage: orchestrator.sh --env <environment> --arch <arch> [options]

Environments:
  qemu-starry      QEMU + StarryOS
  qemu-linux       QEMU + Linux baseline
  board-starry     Real development board

Options:
  --board-ip <ip>  Board IP address (for board-starry)
  --test <id>      Run specific test (default: all tests in matrix)
  --help           Show this help

Examples:
  # QEMU Starry
  ./orchestrator.sh --env qemu-starry --arch x86_64
  
  # Linux baseline
  ./orchestrator.sh --env qemu-linux --arch x86_64
  
  # Board (future)
  ./orchestrator.sh --env board-starry --arch aarch64 --board-ip 192.168.1.100
EOF
  exit 1
}

# 解析参数
ENV=""
ARCH=""
BOARD_IP=""
TEST_ID=""

while [[ $# -gt 0 ]]; do
  case $1 in
    --env) ENV=$2; shift 2 ;;
    --arch) ARCH=$2; shift 2 ;;
    --board-ip) BOARD_IP=$2; shift 2 ;;
    --test) TEST_ID=$2; shift 2 ;;
    --help) usage ;;
    *) nb_die "Unknown option: $1" ;;
  esac
done

[ -z "$ENV" ] && nb_die "Missing --env"
[ -z "$ARCH" ] && nb_die "Missing --arch"

# 加载对应的 adapter
ADAPTER_FILE="$SCRIPT_DIR/../adapters/${ENV}.sh"
[ -f "$ADAPTER_FILE" ] || nb_die "Adapter not found: $ADAPTER_FILE"
source "$ADAPTER_FILE"

# 导出环境变量
export BENCH_ENV=$ENV
export ARCH=$ARCH
export BOARD_IP=$BOARD_IP

nb_log "Starting net-bench orchestrator"
nb_log "Environment: $ENV, Arch: $ARCH"

# 标准生命周期
adapter_setup
adapter_deploy

# 执行测试
if [ -n "$TEST_ID" ]; then
  # 单个测试
  adapter_execute "$TEST_ID"
else
  # 遍历测试矩阵
  while IFS= read -r test; do
    test_id=$(echo "$test" | yq eval '.id' -)
    nb_log "Running test: $test_id"
    adapter_execute "$test_id"
  done < <(yq eval '.tests[]' -o json "$SCRIPT_DIR/test-matrix.yaml")
fi

adapter_collect
adapter_teardown

nb_log "All tests completed. Results in results/"
```

在 xtask 中集成（可选）：

```bash
# 未来可以通过 xtask 调用
cargo xtask starry app test net-bench --env qemu-starry --arch x86_64

# xtask 内部调用
bash apps/starry/net-bench/core/orchestrator.sh --env qemu-starry --arch x86_64
```

**优先级理由**：实现统一入口，消除双入口割裂，为 CI 矩阵化铺路。

**预计工作量**：8-10 小时（包括 orchestrator 实现、xtask 集成、测试）

---

### P6：CI 集成对称化

**问题**：当前只有 Starry 测试有 QEMU toml 和 CI 集成，Linux baseline 独立运行。

**解决方案**：

创建 CI workflow `.github/workflows/net-bench.yml`：

```yaml
name: net-bench

on:
  pull_request:
    paths:
      - 'apps/starry/net-bench/**'
      - 'crates/starry_network/**'
  workflow_dispatch:

jobs:
  net-bench:
    runs-on: ubuntu-latest
    strategy:
      matrix:
        environment:
          - qemu-starry
          - qemu-linux
        arch:
          - x86_64
          # 未来可添加：
          # - riscv64
          # - aarch64
    
    steps:
      - uses: actions/checkout@v3
      
      - name: Setup Rust
        uses: actions-rs/toolchain@v1
        with:
          toolchain: nightly
          
      - name: Install dependencies
        run: |
          sudo apt-get update
          sudo apt-get install -y qemu-system-x86 iperf3 bridge-utils
          
      - name: Run net-bench
        run: |
          cd apps/starry/net-bench
          bash core/orchestrator.sh \
            --env ${{ matrix.environment }} \
            --arch ${{ matrix.arch }}
            
      - name: Upload results
        uses: actions/upload-artifact@v3
        with:
          name: net-bench-results-${{ matrix.environment }}-${{ matrix.arch }}
          path: apps/starry/net-bench/results/*.json
          
  compare:
    needs: net-bench
    runs-on: ubuntu-latest
    steps:
      - uses: actions/download-artifact@v3
        
      - name: Compare results
        run: |
          # 对比 qemu-starry 和 qemu-linux 的性能
          python3 compare-results.py \
            net-bench-results-qemu-starry-x86_64/*.json \
            net-bench-results-qemu-linux-x86_64/*.json
```

**优先级理由**：实现 CI 自动化对比，确保 Starry 性能不退化。

**预计工作量**：6-8 小时（包括 workflow 编写、结果对比脚本、测试）

---

## 实施路线图

### 阶段 1：基础重构（1-2 周）
- [ ] P0：消除 xtask 内部路径依赖
- [ ] P1：抽取测试矩阵为声明式配置
- [ ] P3：统一结果格式为 JSON schema

**里程碑**：测试逻辑单一化，结果可机器解析

### 阶段 2：架构统一（2-3 周）
- [ ] P4：实现 adapter 接口框架
- [ ] P2：Linux 内核纳入 image registry
- [ ] P5：创建统一入口（orchestrator）

**里程碑**：双入口统一，所有资源走 registry

### 阶段 3：CI 增强（1 周）
- [ ] P6：CI 集成对称化
- [ ] 添加性能回归检测
- [ ] 添加跨环境对比报告

**里程碑**：CI 矩阵化，自动化性能对比

### 阶段 4：板载准备（按需）
- [ ] 实现 `adapters/board-starry.sh`
- [ ] 配置板载测试环境（网络、串口、PDU）
- [ ] 编写板载测试文档

**里程碑**：板载测试就绪

---

## 板载测试特有考虑

### 网络拓扑配置

板载测试时，发端和收端不再在同一台机器：

```
当前 QEMU：宿主机 ←─ tap/vhost ─→ QEMU guest
未来板载：CI runner / 测试机 ←─ 以太网 ─→ 开发板
```

需要考虑：
- 发端机和板载之间的网络连通性（防火墙、路由）
- 静态 IP 配置（板载端可能没有 DHCP）
- MTU 和网络参数调优

### 资源部署方式

| 方式 | 适用场景 | 优点 | 缺点 |
|------|---------|------|------|
| scp/ssh | 板载有网络连接 | 简单、快速 | 需要网络可达 |
| SD 卡 | 离线环境 | 不依赖网络 | 需要物理访问 |
| TFTP | u-boot 阶段 | 可在启动早期部署 | 需要 TFTP 服务器 |
| 烧录到 rootfs | 固化部署 | 最可靠 | 修改测试需要重新烧录 |

建议：优先使用 ssh/scp（最灵活），配置文件中预留其他方式的接口。

### 错误恢复机制

板载测试更容易遇到不可恢复错误（内核 panic、网络栈死锁）。需要：

1. **健康检查**：定期 ping + ssh 连接测试
2. **串口监控**：捕获 panic 日志
3. **软重启**：通过串口发送 reboot 命令
4. **硬重启**：通过 PDU（Power Distribution Unit）断电重启
5. **超时机制**：单个测试超时后自动触发恢复流程

建议在 adapter 中实现分级恢复策略：

```bash
recover_board() {
  # Level 1: SSH reboot
  if ssh root@"$BOARD_IP" 'reboot'; then
    sleep 30; return 0
  fi
  
  # Level 2: Serial reboot
  if [ -e "$BOARD_SERIAL" ]; then
    echo "reboot" > "$BOARD_SERIAL"
    sleep 30; return 0
  fi
  
  # Level 3: Power cycle
  if [ -n "$POWER_CONTROL" ]; then
    curl -X POST "$POWER_CONTROL/reboot"
    sleep 60; return 0
  fi
  
  nb_die "All recovery attempts failed"
}
```

### 环境指纹增强

板载测试需要记录更多硬件信息：

```json
{
  "fingerprint": {
    "execution_env": "board-starry",
    "board_model": "RK3588",
    "cpu_cores": 8,
    "ram_mb": 8192,
    "network_interface": "eth0",
    "link_speed": "1000baseT",
    "kernel_version": "starry-0.2.0",
    "kernel_commit": "abc123",
    "bootloader": "u-boot-2024.01",
    "rootfs_version": "buildroot-2024.02"
  }
}
```

这样可以在不同板子之间对比性能。

---

## 总结

重构后的三层架构提供：

1. **环境无关的测试编排** → 测试逻辑只写一次
2. **统一的适配层接口** → 添加新环境只需实现 5 个方法
3. **集中化的资源管理** → 所有依赖可复现、可校验
4. **标准化的结果格式** → CI 自动化对比
5. **板载测试就绪** → 接口已预留，实现时无需改动核心逻辑

按优先级实施后，net-bench 将成为真正的"产品级"网络性能测试套件，从 QEMU 迁移到板载只需实现一个新的 adapter，其余代码零修改。
