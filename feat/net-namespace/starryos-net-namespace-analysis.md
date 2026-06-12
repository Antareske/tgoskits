# StarryOS 网络命名空间：静态分析与实现方案

## 1. 静态分析范围

本次分析覆盖以下源文件：

**axnsproxy（namespace 数据结构）**
- `os/StarryOS/axnsproxy/src/net.rs`
- `os/StarryOS/axnsproxy/src/lib.rs`

**StarryOS kernel syscall 层**
- `kernel/src/syscall/task/clone.rs`
- `kernel/src/syscall/task/namespace.rs`
- `kernel/src/syscall/ns.rs`
- `kernel/src/syscall/net/socket.rs`
- `kernel/src/syscall/net/io.rs`
- `kernel/src/syscall/net/opt.rs`
- `kernel/src/syscall/fs/fd_ops.rs`

**StarryOS kernel 文件层**
- `kernel/src/file/net.rs`
- `kernel/src/file/netlink.rs`
- `kernel/src/file/packet.rs`
- `kernel/src/file/nsfd.rs`
- `kernel/src/pseudofs/proc.rs`

**axnet-ng（协议栈）**
- `modules/axnet-ng/src/lib.rs`
- `modules/axnet-ng/src/service.rs`
- `modules/axnet-ng/src/router.rs`
- `modules/axnet-ng/src/wrapper.rs`
- `modules/axnet-ng/src/listen_table.rs`
- `modules/axnet-ng/src/tcp.rs`
- `modules/axnet-ng/src/udp.rs`
- `modules/axnet-ng/src/general.rs`
- `modules/axnet-ng/src/device/mod.rs`
- `modules/axnet-ng/src/device/loopback.rs`

---

## 2. 现有实现的完整现状

### 2.1 NetNamespace 数据结构

`axnsproxy/src/net.rs`：

```rust
pub struct NetNamespace {
    pub ns_id: u64,
}
```

仅包含 `ns_id`。`clone_ns()` 只分配新 ID，不复制任何网络资源。

`NsProxy` 通过 `Arc<SpinNoIrq<NetNamespace>>` 持有网络命名空间引用：
- `clone_all()`（fork/clone 无 CLONE_NEWNET）：共享同一 `Arc`
- `unshare_net()`：调用 `clone_ns()` 创建新 `NetNamespace`，替换 `Arc`
- `set_ns_net()`（`setns`）：直接替换 `Arc`

### 2.2 syscall 接入

- `sys_clone(CLONE_NEWNET)`：`new_nsproxy.unshare_net()`
- `sys_unshare(CLONE_NEWNET)`：`nsproxy.unshare_net()`
- `sys_setns(fd, CLONE_NEWNET)`：
  - NsFd 路径：直接替换 `net_ns`，无多线程限制检查
  - PidFd 路径：直接替换 `net_ns`，无多线程限制检查

### 2.3 namespace 判断函数

`kernel/src/syscall/ns.rs`：

```rust
pub fn in_root_net_ns() -> bool {
    nsproxy.net_ns.lock().ns_id == 0
}
```

根据当前任务查询，硬编码 `ns_id == 0` 为 root namespace 判断依据。

### 2.4 接口可见性（ioctl 层）

`file/net.rs` 中 `Socket::ioctl()`：

| IOCTL | root ns | 非 root ns |
|---|---|---|
| `SIOCGIFCONF` | 返回 eth0+lo，含 eth0 | 仅返回 lo（sizing 仍报 2×IFREQ） |
| `SIOCGIFFLAGS/ADDR/HWADDR/MTU` 等（eth0） | 正常返回 | `ENODEV` |
| 同上（lo） | 正常返回 | 正常返回 |

**问题**：`SIOCGIFCONF` sizing call（`ifc_buf == NULL`）始终返回 `2 * IFREQ_COMPAT_LEN`，非 root namespace 应只返回 `1 * IFREQ_COMPAT_LEN`（仅 lo）。

### 2.5 netlink 层

`file/netlink.rs`：

- `NetlinkSocket` 无 namespace 字段
- `NETLINK_SOCKETS`：全局静态 registry，`broadcast()` 不区分 namespace
- `build_route_response()`：调用 `in_root_net_ns()`（基于当前任务），非 socket 归属 namespace

**问题**：

1. netlink socket 创建后，其"属于哪个 netns"语义不稳定。`RTM_GETLINK/RTM_GETADDR` 响应取决于调用 `write()` 时当前任务所在的 namespace，而不是 socket 创建时所在的 namespace。`setns()` 后行为会随任务变化，违反 Linux 语义。
2. `broadcast()` 没有 namespace 过滤，`NETLINK_ROUTE` 事件会跨 namespace 泄漏。

### 2.6 packet socket 层

`file/packet.rs`：

- `PacketSocket::new()`：`if !in_root_net_ns() { return Err(EPERM) }`
- `bind_ll()`、`send_packet()`、`recv_packet()`、`ioctl()`：均检查 `in_root_net_ns()`

**问题**：`AF_PACKET` 按 "必须是 root netns" 拒绝，偏离 Linux 语义。Linux 的 `AF_PACKET` 约束是 `CAP_NET_RAW` 加上设备属于本 namespace，而不是 namespace 类型本身。

### 2.7 /proc/net 层

`kernel/src/pseudofs/proc.rs`：

- `/proc/net/arp`：调用 `axnet::arp_entries()`，使用全局 ARP 表，无 namespace 过滤
- `/proc/net/dev`：静态字符串，包含 `eth0`，无任何 namespace 过滤
- `/proc/<pid>/ns/net`：正确展示 `ns_id`

**问题**：非 root namespace 进程读取 `/proc/net/dev` 仍能看到 `eth0`，与 ioctl 层和 netlink 层不一致。

### 2.8 axnet-ng 全局状态（最关键发现）

`axnet-ng/src/lib.rs` 中三个全局变量控制所有网络操作：

```rust
static LISTEN_TABLE: LazyLock<ListenTable> = LazyLock::new(ListenTable::new);
static SOCKET_SET: LazyLock<SocketSetWrapper> = LazyLock::new(SocketSetWrapper::new);
static SERVICE: Once<Mutex<Service>> = Once::new();
```

`TcpSocket::new()` 直接调用 `SOCKET_SET.add()`；`bind()` 直接查 `LISTEN_TABLE.can_listen()`；`connect()` / `send()` 调用全局 `get_service()`。`UdpSocket` 同理。

**结论**：TCP/UDP/raw socket 在构造和每次操作时均直接调用全局协议栈，没有任何 per-namespace 参数化路径。任何 namespace 内创建的 socket 实际共享同一套路由表、ARP 缓存、端口绑定表、连接状态机。

`Service` 的数据结构：
- `iface: smoltcp::Interface`：smoltcp 接口，持有 IP 地址列表
- `router: Router`：路由表+设备列表
- `SOCKET_SET`：所有 smoltcp socket 的集合

这些均是全局单例，`axnet-ng` 当前架构不支持多实例。

### 2.9 setns/unshare 权限语义

`sys_unshare(CLONE_NEWNET)` 和 `sys_setns(CLONE_NEWNET)` 均无 `CAP_SYS_ADMIN` 检查。Linux 要求 `CAP_SYS_ADMIN` 才能进入不属于当前用户 namespace 创建的网络 namespace。

---

## 3. Linux ABI 基线

以下是必须对齐的 Linux 语义，后续实现不应违反：

1. **socket namespace 归属**：socket 属于创建时所在的网络 namespace，进程后续 `setns()` 不迁移已有 socket。`bind/connect/send/recv` 使用 socket 归属 namespace，不使用当前任务 namespace。
2. **新网络 namespace 内容**：默认只含 `lo`，`lo` 初始为 **down**（需 `ip link set lo up` 或 `SIOCSIFFLAGS` 启用）。真实物理设备不自动出现。
3. **端口隔离**：各 namespace 端口绑定空间完全独立；不同 namespace 可同时 bind 相同端口。
4. **rtnetlink 归属**：`NETLINK_ROUTE` socket 归属创建时 namespace；`RTM_GETLINK/GETADDR` 响应使用 socket 的 namespace，不使用 `write()` 时当前任务的 namespace。
5. **`AF_PACKET` 语义**：不是"只有 root netns"，而是需要本 namespace 下的 `CAP_NET_RAW` 并且只能绑定本 namespace 可见设备。新 namespace 可针对 `lo` 创建 packet socket。
6. **`/proc/net` 视图**：展示当前进程 namespace 的设备和状态，非 root namespace 不展示 `eth0`。
7. **`setns/unshare` 权限**：需要 `CAP_SYS_ADMIN`（或用户 namespace 所有者权限）。
8. **不支持操作的错误码**：配置类 ABI 临时不支持返回 `EOPNOTSUPP`/`ENOSYS`，查询类 ABI 必须正确过滤，不能返回错误数据。

---

## 4. 方案评估

### 4.1 原方案的准确之处

- 识别了"NetNamespace 只有 ns_id"为核心缺口，正确。
- 提出 socket 保存创建时 namespace，符合 Linux 语义。
- 提出 netlink 按 socket 归属 namespace 响应，正确。
- 提出 `/proc/net` 按当前进程 namespace 过滤，正确。
- 分阶段（M1/M2/M3）的大方向合理。

### 4.2 原方案需要修正的地方

**问题 1：axnet-ng 全局化程度远超方案预期**

原方案提到"最大技术风险是 axnet 是否支持多实例"，但实际情况比这更具体：

- `SOCKET_SET`、`LISTEN_TABLE`、`SERVICE` 三个全局变量在 `lib.rs` 中静态初始化，`TcpSocket`/`UdpSocket` 的每个方法都直接引用它们。没有任何间接层。
- 原方案中 `NetStackHandle::LoopbackOnly(LoopbackStack)` 方向是可行的，但要求 loopback-only 栈绕过全局 `SOCKET_SET/LISTEN_TABLE/SERVICE`，实际上相当于引入第二套 socket 实现。更低成本的路径是在 `axnet-ng` 内部引入 per-netns context 概念，而不是在 kernel 层包装两套实现。

**问题 2：阶段划分顺序有歧义**

原方案阶段 2（socket 固定 namespace）和阶段 3（阻断外网泄漏）写为分开步骤，并注明"不能单独发布"，但结构上仍保持分离。实际上两者是同一批改动的不同方面，强烈建议合并为单个阶段。

**问题 3：AF_PACKET 描述**

原方案说"非 root namespace 不能创建 AF_PACKET socket"，这是当前实现的错误语义，已被 ABI 基线纠正，方案文档已更新。

**问题 4：lo 初始状态**

原方案 lo 初始状态有"可选"表述。ABI 基线明确：Linux 对齐是初始 down；若 StarryOS 保留 up，只能是有记录的兼容偏离，不能视为最终实现。

**问题 5：setns 多线程检查缺失**

Linux 的 `setns(CLONE_NEWNET)` 对多线程进程无约束（不像 PID/USER namespace），当前 StarryOS 实现也没有相关限制，这部分是正确的，原方案未特别提及。

### 4.3 核心实现路径评估

基于静态分析，实现完整 network namespace 需要解决两个正交问题：

**问题 A：axnet-ng 全局状态参数化**

涉及 `SOCKET_SET`、`LISTEN_TABLE`、`SERVICE`。每条路径需要能选择"用哪个 socket set / listen table / service"。

最小改造路径是将三者包装为 `NetStack` 结构，通过 `Arc` 持有，`TcpSocket`/`UdpSocket` 创建时传入，bind/connect/send/recv 使用 socket 内保存的引用：

```rust
pub struct NetStack {
    pub listen_table: ListenTable,
    pub socket_set: SocketSetWrapper,
    pub service: Mutex<Service>,
}
```

root namespace 使用 `ROOT_NET_STACK`（对应现有全局三件套），非 root namespace 使用独立的 `NetStack`（初始只含 loopback 设备）。

这一改造需要修改 `axnet-ng` 内部，但改动有明确边界——只是把全局引用改为方法参数或 `Arc` 字段，不改变现有逻辑。

**问题 B：socket 层捕获创建时 namespace**

`file/net.rs::Socket` 增加 `net_ns: Arc<SpinNoIrq<NetNamespace>>` 字段，`sys_socket()` 在创建时捕获。`NetNamespace` 持有对应的 `Arc<NetStack>`。所有 ioctl、bind、connect 操作改为从 `self.net_ns` 获取栈引用，而不是调用全局 `in_root_net_ns()`。

`NetlinkSocket` 同理。

---

## 5. 修订后的实现方案

### 5.1 数据结构设计

**axnet-ng 侧：NetStack**

```rust
pub struct NetStack {
    pub listen_table: ListenTable,
    pub socket_set: SocketSetWrapper,
    pub service: Mutex<Service>,
}

impl NetStack {
    pub fn new_loopback_only() -> Arc<Self> { ... }
    pub fn root() -> &'static Arc<Self> { ... } // 返回现有全局栈
}
```

**axnsproxy 侧：NetNamespace 扩展**

```rust
pub struct NetNamespace {
    pub ns_id: u64,
    pub stack: Arc<NetStack>, // 对应的网络栈实例
}
```

root namespace 的 `stack` 指向 `NetStack::root()`（包装现有全局三件套）。`clone_ns()` 创建新 namespace 时，`stack` 指向 `NetStack::new_loopback_only()`。

**axnet-ng 侧：TcpSocket/UdpSocket 持有栈引用**

```rust
pub struct TcpSocket {
    stack: Arc<NetStack>,  // 新增
    handle: SocketHandle,
    // ... 其余不变
}
```

`TcpSocket::new(stack: Arc<NetStack>)` 替代无参 `new()`，`SOCKET_SET.add()` 改为 `stack.socket_set.add()`。

**kernel 侧：Socket 持有 namespace 引用**

```rust
pub struct Socket {
    inner: SocketInner,
    net_ns: Arc<SpinNoIrq<NetNamespace>>,  // 新增
    ip_domain: u32,
    // ...
}
```

`sys_socket()` 创建时捕获当前进程 `net_ns`，从中取 `stack` 传给 `TcpSocket::new()`/`UdpSocket::new()`。

`NetlinkSocket` 同理添加 `net_ns` 字段。

### 5.2 分阶段实现

#### 阶段 1：修正现有可见性不一致（无需改 axnet-ng）

改动范围：仅 `kernel/` 层，不涉及 axnet-ng。

- 修正 `SIOCGIFCONF` sizing call：非 root namespace 返回 `1 * IFREQ_COMPAT_LEN`。
- 修正 `/proc/net/dev`：调用 `in_root_net_ns()` 过滤 eth0（与 ioctl/netlink 一致）。
- 修正 `/proc/net/arp`：非 root namespace 返回空表（`axnet::arp_entries()` 的结果只给 root ns 展示）。
- 修正 `AF_PACKET`：将"必须是 root netns"改为"设备必须在本 namespace 可见"；非 root namespace 创建 packet socket 时，若本 namespace 没有以太网设备，则创建成功但 bind 到不存在设备时返回 `ENODEV`。此阶段可保持对 lo 的 packet socket 暂不支持，但不能以 namespace 类型拒绝。
- 为上述三项添加一致性验证测试。

验证标准：root ns 行为与当前完全一致；非 root ns 中 ioctl/netlink/proc 三路径返回结果相同。

#### 阶段 2：socket 捕获 namespace + 阻断非 root 误用全局栈

**此阶段与阶段 1 的区别**：阶段 1 只改可见性展示，阶段 2 改 socket 操作路径。**两者必须同一批合入**，中间状态不应以"已隔离"发布。

改动范围：`axnet-ng` + `kernel/file/net.rs` + `kernel/file/netlink.rs`。

axnet-ng 改动：
- 将 `LISTEN_TABLE`/`SOCKET_SET`/`SERVICE` 包装为 `NetStack`，提供 `NetStack::root()` 全局入口保持向后兼容。
- `TcpSocket::new(stack)`/`UdpSocket::new(stack)` 接受 `Arc<NetStack>` 参数，所有内部操作从 `self.stack` 获取引用。
- 保持 `TcpSocket::new()` 无参版本作为兼容 shim，内部调用 `NetStack::root()`。

axnsproxy 改动：
- `NetNamespace` 增加 `stack: Arc<NetStack>` 字段。
- `NetNamespace::new_root()` 使用 `NetStack::root()`。
- `NetNamespace::clone_ns()` 使用 `NetStack::new_loopback_only()`（loopback 栈：只含 lo 设备，独立 SOCKET_SET/LISTEN_TABLE）。

kernel 改动：
- `Socket` 增加 `net_ns: Arc<SpinNoIrq<NetNamespace>>`；`sys_socket()` 创建时捕获。
- `Socket` 的 ioctl 逻辑从 `Socket::net_ns` 中读取 namespace，替代调用全局 `in_root_net_ns()`。
- `NetlinkSocket` 增加 `net_ns` 字段；`build_route_response()` 从 `self.net_ns` 判断可见设备，替代 `in_root_net_ns()`。
- `NETLINK_SOCKETS` registry 条目增加 `ns_id`，`broadcast()` 按 `ns_id` 过滤。

验证标准：
- root ns socket 行为与 M0 完全相同。
- 非 root ns socket 的 bind 只能绑定本 namespace 地址（`127.0.0.1`），对其他地址返回 `EADDRNOTAVAIL`。
- 非 root ns socket 的 connect 对 root ns 路由地址返回 `ENETUNREACH`。
- `setns()` 后新建 socket 使用新 namespace；旧 socket 仍使用旧 namespace 的栈。
- 两个不同 netns 可同时 bind `127.0.0.1:8080`（依赖 `NetStack::new_loopback_only()` 实现独立端口表）。

#### 阶段 3：完善 loopback-only 栈

阶段 2 中 `NetStack::new_loopback_only()` 需要能正常完成 TCP/UDP loopback 通信。

改动范围：`axnet-ng`。

- `NetStack::new_loopback_only()` 初始化一个独立的 smoltcp `Service`，只包含 `LoopbackDevice`，IP 地址为 `127.0.0.1/8`。
- 验证：同一 netns 内 TCP/UDP 可通过 `127.0.0.1` 互通；不同 netns 之间的 loopback 通信不可达。

#### 阶段 4（远期）：物理设备 + 虚拟设备支持

依赖前三阶段完成。目标是 veth、tap、bridge、设备迁移等。需要 rtnetlink 支持 `RTM_NEWLINK/DELLINK/NEWADDR/DELADDR/NEWROUTE/DELROUTE`，目前 netlink 层仅实现了只读 dump，配置写路径全部缺失。此阶段不在近期计划内。

### 5.3 关于 lo 初始状态的决策

Linux 对齐：`lo` 初始 down，需 `SIOCSIFFLAGS IFF_UP` 或等效 rtnetlink 操作启用。

StarryOS 现有所有用户态应用均假设 lo 立即可用（如 Python、Java 等在初始化时测试 loopback）。

**建议**：`NetStack::new_loopback_only()` 初始化时将 lo 标记为 up，同时在代码和测试中标注为"兼容性偏离（LO_INITIAL_UP_COMPAT）"。后续实现 `SIOCSIFFLAGS` 支持后，改为 Linux 标准初始 down，同时更新相关兼容性测试。

---

## 6. 主要风险

| 风险 | 影响 | 缓解方法 |
|---|---|---|
| axnet-ng 全局三件套参数化改动影响 ArceOS 其他使用方 | 高 | 保留无参 `TcpSocket::new()` 作为 shim，不破坏现有 API |
| smoltcp `SocketSet` 每个 netns 独立可能带来内存压力 | 中 | loopback-only 栈 buffer size 可以独立配置，不必与 root ns 相同 |
| `NetlinkSocket` NETLINK_KOBJECT_UEVENT 广播 namespace 策略 | 中 | 仅 rtnetlink route 事件按 netns 过滤；uevent 暂时全局广播，记录为待优化 |
| `AF_PACKET` lo 收发实现复杂 | 低 | 阶段 1 只改权限语义，不实现 lo 收发；阶段 2 后再补 |
| `setns/unshare` 缺少权限检查 | 低（沙盒内影响有限） | 记录为兼容性缺口；StarryOS 目前容器场景有限，不是紧急问题 |

---

## 7. 验证清单

### 阶段 1 可验证项

- `[ ]` root ns 中 `SIOCGIFCONF`、`RTM_GETLINK`、`/proc/net/dev` 三者均包含 eth0 和 lo
- `[ ]` 新 netns 中三者均只包含 lo
- `[ ]` 新 netns 中 `SIOCGIFCONF` sizing call 返回 `1 * sizeof(ifreq)`
- `[ ]` 新 netns 中 `/proc/net/arp` 返回空表
- `[ ]` 新 netns 中针对 eth0 的 `SIOCGIFINDEX/SIOCGIFADDR` 返回 `ENODEV`
- `[ ]` 新 netns 中 `AF_PACKET` 不因 namespace 类型报 `EPERM`（权限由 `CAP_NET_RAW` 控制）

### 阶段 2 可验证项

- `[ ]` root ns 创建 socket，`unshare(CLONE_NEWNET)` 后，旧 socket 仍按 root ns 行为
- `[ ]` `setns()` 后新建 socket 属于新 netns
- `[ ]` 两个不同 netns 可并发 bind `127.0.0.1:8080`
- `[ ]` 非 root netns socket 对 root ns 路由地址 `connect()` 返回 `ENETUNREACH`
- `[ ]` 不同 netns 中的 `NETLINK_ROUTE` socket `RTM_GETLINK` 各自只返回本 namespace 设备
- `[ ]` `broadcast()` 路由事件不跨 namespace

### 阶段 3 可验证项

- `[ ]` 同一 netns 内 TCP `127.0.0.1:X` ↔ `127.0.0.1:Y` 可连通
- `[ ]` 不同 netns 的 loopback TCP 连接不可达
- `[ ]` UDP loopback 同上
