# StarryOS 网络命名空间入门：概念、组成、现状与实现计划

## 1. 先用一句话理解网络命名空间

网络命名空间（network namespace，简称 netns）就是“给一组进程一套独立的网络世界”。

在普通系统里，所有进程通常共享同一套网络资源：

- 同一张网卡列表，例如 `lo`、`eth0`
- 同一张 IP 地址表
- 同一套路由表
- 同一套 TCP/UDP 端口绑定状态
- 同一套防火墙规则
- 同一套 `/proc/net/*` 视图

网络命名空间把这些资源切开。不同 namespace 里的进程就像运行在不同机器上：

- A namespace 里绑定了 `127.0.0.1:8080`
- B namespace 里也可以绑定 `127.0.0.1:8080`
- 两者不会冲突
- A 看到的网卡和路由不一定是 B 能看到的
- A 的 `localhost` 不是 B 的 `localhost`

一个最典型的用途是容器。容器里的进程觉得自己有独立的网卡、IP、路由和端口空间，但它其实仍然运行在同一个内核里。

## 2. 为什么需要网络命名空间

如果没有网络命名空间，所有进程都共用一套网络状态，会有几个问题。

第一，端口会互相冲突。

例如两个容器都想启动 nginx，并监听 `0.0.0.0:80`。如果没有 netns，它们会抢同一个全局端口，第二个会失败。使用 netns 后，每个容器都有自己的端口空间，两个 `:80` 可以同时存在。

第二，网络视图无法隔离。

容器里执行 `ip addr` 或读取 `/proc/net/dev` 时，不应该看到宿主机所有网卡。它应该只看到自己 namespace 里的设备，例如 `lo` 和一张虚拟网卡 `eth0`。

第三，网络安全边界不清晰。

如果不同应用共享同一套路由、packet socket、netlink 广播事件，就可能出现信息泄漏或错误操作。例如一个 namespace 里的路由事件不应该广播到另一个 namespace。

第四，无法实现标准容器语义。

Linux 容器依赖多个 namespace：mount、pid、uts、ipc、user、network。网络命名空间是容器网络隔离的核心部分。

## 3. 一个直观例子

假设有两个网络命名空间：root netns 和 child netns。

root netns：

```text
interfaces: lo, eth0
routes: default via eth0, 127.0.0.0/8 via lo
tcp ports: 22, 80
```

child netns：

```text
interfaces: lo
routes: 127.0.0.0/8 via lo
tcp ports: empty
```

这时：

- root 里可以访问外网，因为它有 `eth0` 和 default route
- child 里只能访问自己的 loopback，因为它只有 `lo`
- root 里监听 `127.0.0.1:8080` 不影响 child
- child 里监听 `127.0.0.1:8080` 不影响 root
- child 读取 `/proc/net/dev` 应该只看到 `lo`

注意：child namespace 不是“权限更低的网络模式”，而是“一套独立的网络资源集合”。如果以后给 child 加一对 veth 虚拟网卡，它也可以拥有自己的 `eth0`、IP 和路由。

## 4. 网络命名空间到底隔离哪些东西

一个完整的网络命名空间通常至少需要隔离以下组成部分。

### 4.1 网络设备列表

每个 namespace 有自己的设备集合。

常见设备包括：

- `lo`：loopback，本机回环设备
- `eth0`：真实或虚拟以太网设备
- `vethX`：虚拟网卡对，常用于容器连接宿主机
- `tapX` / `tunX`：用户态网络设备
- bridge：网桥，用于连接多个虚拟设备

Linux 中，每个 netns 创建时默认只有一个 down 状态的 `lo`。容器运行时通常会创建 veth pair，把一端放进容器 netns，另一端留在宿主机 namespace。

Starry 当前目标更简单：新 netns 先拥有一套 loopback-only 网络栈，也就是只支持 `lo`，不支持外网。

### 4.2 IP 地址表

设备上配置了哪些 IP 地址，也属于网络命名空间。

例如：

```text
root netns:
  lo    127.0.0.1/8
  eth0  10.0.2.15/24

child netns:
  lo    127.0.0.1/8
```

两个 namespace 都可以有 `127.0.0.1`，因为 loopback 是各自独立的。

### 4.3 路由表

路由表决定“发往某个 IP 的包从哪个设备出去”。

例如：

```text
127.0.0.0/8      dev lo
0.0.0.0/0        via 10.0.2.2 dev eth0
```

root namespace 可以有 default route，所以能访问外网。新创建的 loopback-only namespace 不应该继承 root 的 default route，否则它虽然看不到 `eth0`，却可能仍然通过全局路由偷偷访问外网，这就是隔离泄漏。

### 4.4 TCP/UDP socket 集合

这是实现网络命名空间时最容易被低估的一部分。

协议栈内部通常会维护所有 TCP/UDP socket 的集合，例如：

- 哪些 TCP socket 正在连接
- 哪些 TCP socket 正在监听
- 哪些 UDP socket 已经绑定端口
- 哪些 packet 在接收队列里

如果 socket 集合是全局的，那么不同 namespace 的 socket 仍然会互相看到彼此。这会导致：

- 端口冲突跨 namespace 发生
- `accept()` 可能接受到别的 namespace 的连接
- loopback 流量可能跨 namespace 投递
- poll/waker 事件可能互相干扰

所以网络命名空间不能只在 syscall 层判断“当前是不是 root netns”。协议栈内部的数据结构也必须变成 per-netns。

### 4.5 TCP listen table / 端口绑定表

TCP 服务端监听端口时，协议栈通常需要一张 listen table：

```text
port 80   -> listening socket A
port 8080 -> listening socket B
```

这张表必须是 namespace 内部的。

否则 root netns 里监听了 `:8080`，child netns 就不能监听 `:8080`，这和 Linux 容器语义不一致。

Starry 当前的实现已经把 TCP listen table 和 TCP bound-port conflict table 放进 `NetStack`，实现了 per-netns 隔离。

### 4.6 协议栈服务对象

在 Starry 的 `axnet-ng` 里，网络协议栈核心对象是 `Service`。它封装了 smoltcp 的 `Interface` 和 `Router`。

它负责：

- poll 网络设备
- 驱动 smoltcp 处理 socket 状态机
- 维护路由查询
- 处理 DHCP
- 维护 ARP 表
- 注册网络设备 waker

如果 `Service` 是全局的，那么所有 namespace 会共享同一套路由、设备、IP 地址和协议栈状态。网络命名空间必须把它变成 per-netns。

### 4.7 `/proc/net/*` 视图

Linux 下很多程序不是直接通过 syscall 查询网络状态，而是读 `/proc/net/*`。

常见文件包括：

- `/proc/net/dev`：网络设备统计
- `/proc/net/arp`：ARP 表
- `/proc/net/route`：路由表
- `/proc/net/tcp` / `/proc/net/udp`：socket 状态

这些文件必须按“读取者所在的网络命名空间”显示内容。

例如非 root netns 只应该看到 `lo`，不应该看到 root namespace 的 `eth0`。

Starry 当前已经让 `/proc/net/dev` 对网络命名空间可见性敏感：root netns 显示 `lo + eth0`，非 root netns 只显示 `lo`。

### 4.8 ioctl 网络接口查询

很多传统程序使用 socket ioctl 查询网卡信息，例如：

- `SIOCGIFCONF`：列出接口
- `SIOCGIFFLAGS`：查询接口 flags
- `SIOCGIFADDR`：查询接口 IP
- `SIOCGIFHWADDR`：查询 MAC
- `SIOCGIFMTU`：查询 MTU
- `SIOCGIFINDEX`：查询 ifindex

这些也必须按 namespace 过滤。

Starry 当前已经实现了基本语义：

- root netns：`eth0 + lo`
- 非 root netns：仅 `lo`
- 非 root 查询 `eth0`：返回 `ENODEV`
- `SIOCGIFCONF` sizing call 也按 namespace 返回正确大小

### 4.9 netlink

现代 Linux 网络工具大量依赖 netlink，尤其是 `NETLINK_ROUTE`。

例如：

- `ip link`
- `ip addr`
- `ip route`
- 监听网卡上下线事件
- 监听地址变化事件

netlink socket 也有 namespace 归属。正确语义是：

- netlink socket 创建时属于当前 netns
- 之后即使进程 `setns()` 到别的 netns，这个 socket 仍然属于创建时的 netns
- netlink 查询应该返回 socket 所属 netns 的信息
- netlink 广播事件不能跨 netns 泄漏

Starry 当前 netlink 层还没有完全做到这一点，这是后续计划中的重点。

### 4.10 packet socket / raw socket 权限

`AF_PACKET` socket 可以直接收发链路层 packet，权限很高。Linux 语义不是“只有 root netns 才能创建”，而是：

- 需要 `CAP_NET_RAW`
- socket 只能绑定和访问本 netns 可见的设备

也就是说，如果一个非 root netns 里有自己的 `eth0`，并且进程在对应 user namespace 里有 `CAP_NET_RAW`，它可以创建 packet socket。

Starry 当前 packet socket 仍然偏保守：非 root netns 被拒绝。这不是最终 Linux 语义，后续需要改成“按设备可见性过滤”。

## 5. 网络命名空间和用户/权限的关系

很多人会把 netns 和 userns 混在一起。它们有关联，但不是同一个东西。

网络命名空间隔离的是网络资源。

用户命名空间隔离的是 UID/GID 映射和 capability 语义。

Linux 中，创建和操作网络命名空间通常需要 capability，例如：

- `unshare(CLONE_NEWNET)` 需要 `CAP_SYS_ADMIN`
- 创建 raw socket 需要 `CAP_NET_RAW`
- 修改网络配置通常需要 `CAP_NET_ADMIN`

但是从实现顺序看，不需要先完整实现 user namespace 才能实现 net namespace。

Starry 当前已经有一个简化 credential/capability 框架：

- `Cred` 记录 `uid/euid/gid/egid/fsuid/fsgid` 等
- `has_cap_net_raw()` 当前近似为 `euid == 0`
- `UserNamespace` 有简化的 `uid_map/gid_map` 支持

这足够支撑 netns 的第一阶段实现。完整的 Linux capability bitmask、`CAP_SYS_ADMIN`、`CAP_NET_ADMIN`、userns 内 root capability 等，可以作为后续增强。

所以 Starry 的网络命名空间不应该被“完整多用户支持”阻塞。更合理的顺序是：

1. 先实现网络资源隔离，也就是每个 netns 有自己的 `NetStack`
2. 保留简化 capability 检查，例如 `euid == 0`
3. 后续再把权限检查替换成完整 capability/userns 模型

## 6. Starry 当前已经实现了什么

截至当前分支，Starry 的网络命名空间支持已经从“只有 ns_id 的壳”推进到了“协议栈核心状态 per-netns”。

### 6.1 namespace 数据结构

`NetNamespace` 现在包含：

```rust
pub struct NetNamespace {
    pub ns_id: u64,
    pub stack: Arc<NetStack>,
}
```

含义是：每个网络命名空间不仅有一个 ID，还真正持有一套网络协议栈。

root namespace 使用：

```rust
ROOT_NET_STACK
```

新 namespace 使用：

```rust
NetStack::new_loopback()
```

也就是新建一套 loopback-only 独立栈。

### 6.2 axnet-ng 的全局栈改造成 NetStack

之前 `axnet-ng` 有三个关键全局单例：

```rust
LISTEN_TABLE
SOCKET_SET
SERVICE
```

这会导致所有 namespace 共享同一套 TCP/UDP socket、listen table、router 和 smoltcp service。

当前已经重构为：

```rust
pub struct NetStack {
    pub listen_table: ListenTable,
    pub socket_set: SocketSetWrapper<'static>,
    pub service: Once<Mutex<Service>>,
    ...
}
```

并且：

- `TcpSocket` 持有 `Arc<NetStack>`
- `UdpSocket` 持有 `Arc<NetStack>`
- `RawSocket` 持有 `Arc<NetStack>`
- `SocketSetWrapper` 变成 per-stack
- `ListenTable` 变成 per-stack
- `Service` 变成 per-stack
- TCP bound-port conflict table 变成 per-stack
- TCP/UDP ephemeral port counter 变成 per-stack

### 6.3 socket 创建时捕获当前 netns

`sys_socket()` 当前会读取调用进程所在的 netns：

```rust
current().as_thread().proc_data.nsproxy.lock().net_ns.lock().stack.clone()
```

然后把这个 `Arc<NetStack>` 传给：

- `TcpSocket::new(stack)`
- `UdpSocket::new(stack)`
- `RawSocket::new(..., stack)`

这很重要。正确语义不是“socket 每次操作时看当前 task 的 netns”，而是“socket 创建时属于哪个 netns，之后一直属于那个 netns”。

例如：

1. 进程在 netns A 中创建 socket
2. 进程调用 `setns()` 切到 netns B
3. 继续操作旧 socket

旧 socket 仍然应该使用 netns A 的网络栈。当前 TCP/UDP/Raw socket 已经通过持有 `Arc<NetStack>` 实现了这个方向。

### 6.4 非 root namespace 的 loopback-only 栈

新建网络命名空间时，当前创建的是一套 loopback-only 栈：

```text
interfaces: lo
addresses: 127.0.0.1/8
routes: 127.0.0.0/8 via lo
```

这意味着：

- 非 root netns 内 TCP/UDP loopback 通信可以在自己的栈内完成
- 非 root netns 不继承 root 的 `eth0`
- 非 root netns 不继承 root 的 default route
- 非 root netns 无法通过 root 栈泄漏外网访问

这是容器网络隔离的基础版本。

### 6.5 `/proc/net/dev` 和 ioctl 可见性

当前可见性规则：

| 查询 | root netns | 非 root netns |
|---|---|---|
| `/proc/net/dev` | `lo` + `eth0` | `lo` |
| `SIOCGIFCONF` | `lo` + `eth0` | `lo` |
| `SIOCGIFCONF` sizing | 2 个接口大小 | 1 个接口大小 |
| 查询 `lo` | 成功 | 成功 |
| 查询 `eth0` | 成功 | `ENODEV` |

这保证了应用枚举网络接口时不会在非 root namespace 看到 root namespace 的 `eth0`。

## 7. 当前还没有完全实现什么

虽然核心协议栈已经 per-netns，但还有一些 Linux 语义没有完成。

### 7.1 netlink socket 还需要捕获 netns

当前 TCP/UDP/Raw socket 已经捕获了 `NetStack`，但 netlink socket 后续也应捕获 `NetNamespace` 或 `NetStack`。

需要实现：

- `NetlinkSocket` 增加 namespace 归属字段
- `RTM_GETLINK` / `RTM_GETADDR` 基于 socket 所属 netns 返回
- `NETLINK_ROUTE` broadcast 按 namespace 过滤
- 进程 `setns()` 后，旧 netlink socket 的语义不变

### 7.2 packet socket 的 Linux 语义还需修正

当前 packet socket 对非 root netns 仍然偏保守。

计划改成：

- 创建时检查 `CAP_NET_RAW`
- bind/send/recv/ioctl 按 socket 所属 netns 的设备可见性过滤
- 非 root netns 如果只有 `lo`，就只能访问 `lo`
- 未来如果加入 veth，容器 netns 内 packet socket 可以访问自己的 veth 设备

### 7.3 `/proc/net/arp` 仍需 per-netns

当前 `/proc/net/arp` 仍然调用 root/global 方向的 ARP 查询接口。

正确做法是：

- 按当前读取者 netns 查询对应 `NetStack`
- root netns 返回 root stack ARP 表
- loopback-only netns 通常返回空 ARP 表

### 7.4 还没有 veth/tap/bridge

当前新 netns 是 loopback-only。它能证明网络栈隔离，但还不是完整容器网络。

完整容器网络通常需要：

- veth pair
- 把 veth 一端移动到 child netns
- root netns 侧接 bridge 或 NAT
- child netns 配置 IP 和 default route

这些属于后续阶段。

### 7.5 权限模型仍是简化 capability

当前能力判断主要是 `euid == 0`。

后续需要：

- `Cred` 增加 capability bitmask
- 增加 `has_cap_sys_admin()`
- 增加 `has_cap_net_admin()`
- `unshare(CLONE_NEWNET)` 使用正确权限检查
- user namespace 内 root 获得对应 namespace capability

但这不是当前 netns 隔离工作的硬前置。

## 8. 推荐实现路线

### 阶段 1：接口可见性修正（已完成）

目标：让应用在不同 namespace 看到不同接口列表。

内容：

- `/proc/net/dev` 非 root netns 只显示 `lo`
- `SIOCGIFCONF` 非 root netns 只返回 `lo`
- `SIOCGIFCONF` sizing call 按 namespace 返回大小
- `SIOCGIF*` 查询 `eth0` 时在非 root netns 返回 `ENODEV`

这个阶段解决的是“看起来隔离”。

### 阶段 2：协议栈核心 per-netns（已完成）

目标：让 socket 真的用不同网络栈。

内容：

- 新增 `NetStack`
- `NetNamespace` 持有 `Arc<NetStack>`
- root 使用 `ROOT_NET_STACK`
- `clone_ns()` 创建 loopback-only stack
- `TcpSocket` / `UdpSocket` / `RawSocket` 捕获 stack
- socket set、listen table、service、端口表 per-stack

这个阶段解决的是“真的隔离”。

### 阶段 3：netlink 和 packet socket 修正（计划）

目标：补齐现代 Linux 网络工具和 raw/packet 语义。

内容：

- `NetlinkSocket` 捕获 netns
- `RTM_GETLINK` / `RTM_GETADDR` 使用 socket 所属 netns
- netlink broadcast 按 netns 过滤
- `AF_PACKET` 从“非 root netns 禁止”改为“按能力和设备可见性判断”
- `/proc/net/arp` 改为 per-netns

这个阶段解决的是“工具和边界一致”。

### 阶段 4：虚拟设备和完整容器网络（远期）

目标：让非 root netns 不只是 loopback-only，而是真正可接入网络。

内容：

- veth pair
- tap/tun
- bridge
- 设备移动到指定 netns
- per-netns IP 配置
- per-netns default route
- root netns 与 child netns 之间转发/NAT

这个阶段解决的是“容器能联网”。

### 阶段 5：完整 capability/user namespace（远期）

目标：贴近 Linux 权限语义。

内容：

- `Cred` 增加 capability masks
- `CAP_SYS_ADMIN`
- `CAP_NET_ADMIN`
- `CAP_NET_RAW`
- user namespace capability 继承和映射
- `unshare(CLONE_NEWUSER | CLONE_NEWNET)` 语义

这个阶段解决的是“权限模型标准化”。

## 9. 判断实现是否正确的几个测试场景

### 9.1 接口可见性测试

root netns：

```sh
cat /proc/net/dev
```

应该看到：

```text
lo
eth0
```

unshare netns 后：

```sh
unshare -n cat /proc/net/dev
```

应该只看到：

```text
lo
```

### 9.2 端口隔离测试

root netns：

```sh
bind 127.0.0.1:8080
```

child netns：

```sh
bind 127.0.0.1:8080
```

应该都成功。两个 socket 不应该互相冲突。

### 9.3 loopback 隔离测试

root netns 监听：

```text
127.0.0.1:8080
```

child netns 连接：

```text
127.0.0.1:8080
```

不应该连接到 root netns 的服务。child 的 `127.0.0.1` 是 child 自己的 loopback。

### 9.4 setns 后 socket 归属测试

流程：

1. 在 netns A 创建 socket
2. 调用 `setns()` 切换到 netns B
3. 继续操作旧 socket

旧 socket 应该继续使用 netns A 的网络栈。

当前 TCP/UDP/Raw socket 的设计已经满足这个方向，因为它们持有创建时的 `Arc<NetStack>`。

### 9.5 netlink 归属测试（计划项）

流程：

1. 在 netns A 创建 netlink socket
2. `setns()` 到 netns B
3. 通过旧 netlink socket 查询 link

正确结果应该是 netns A 的 link，而不是 netns B 的 link。

这是后续 netlink 修正需要覆盖的测试。

## 10. 初学者最容易误解的点

### 10.1 “非 root netns”不是“低权限网络模式”

网络命名空间描述的是资源隔离，不是权限高低。

非 root netns 现在只有 `lo`，是因为我们还没有实现 veth/tap/bridge，不是因为非 root netns 天生不能有 `eth0`。

### 10.2 只隐藏 `eth0` 不等于实现了 netns

如果只是让 `/proc/net/dev` 不显示 `eth0`，但底层 TCP/UDP socket 仍然用全局 socket set，那么隔离是假的。

真正的 netns 必须让协议栈内部状态也 per-netns。

### 10.3 socket 应该属于创建时的 namespace

很多查询函数可以看“当前 task 的 netns”，但 socket 操作不能这么做。

socket 一旦创建，它就属于创建时的 netns。进程之后切换 namespace，不应该改变旧 socket 的归属。

### 10.4 loopback 也必须隔离

`127.0.0.1` 经常被误以为是全局的。实际上每个 netns 都有自己的 loopback。

容器里的 `localhost` 不是宿主机的 `localhost`。

### 10.5 user namespace 不是 net namespace 的硬前置

完整 Linux 语义下，权限检查和 user namespace 关系很密切。

但工程实现可以分阶段：先做网络资源隔离，再完善 capability/user namespace。Starry 当前就是这个路线。

## 11. 当前 Starry 的总体状态总结

可以把当前状态概括为：

```text
已完成：
  - NetNamespace 持有 NetStack
  - root namespace 使用 ROOT_NET_STACK
  - 新 netns 创建 loopback-only NetStack
  - TCP/UDP/Raw socket 捕获创建时的 NetStack
  - SocketSet / ListenTable / Service / TCP port table per-stack
  - /proc/net/dev 按 netns 过滤
  - SIOCGIFCONF 和 SIOCGIF* 按 netns 过滤

未完成：
  - NetlinkSocket 捕获 netns
  - netlink route response/broadcast 按 socket netns 过滤
  - AF_PACKET 按设备可见性实现，而不是按 root netns 拒绝
  - /proc/net/arp per-netns
  - veth/tap/bridge 等虚拟设备
  - 完整 Linux capability/user namespace 模型
```

因此，Starry 当前已经完成了网络命名空间最核心、最危险的部分：协议栈状态从全局改为 per-netns。

后续重点不再是“是否真的隔离 socket”，而是补齐 Linux 用户态可见接口和容器联网能力。
