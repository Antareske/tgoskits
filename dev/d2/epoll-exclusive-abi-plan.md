# StarryOS EPOLLEXCLUSIVE ABI 评估与修复方案

## 背景

本分支为支持 nginx multi-worker 场景引入了 `EPOLLEXCLUSIVE` 兼容改动。nginx 多 worker 常见运行方式是多个 worker 进程分别阻塞在 `epoll_wait()`，共同监听同一个 listen socket。当新连接到来时，Linux 的 `EPOLLEXCLUSIVE` 可减少多个 worker 被同一事件同时唤醒造成的 thundering herd 问题。

StarryOS 当前若完全不识别 `EPOLLEXCLUSIVE`，用户态在执行如下注册时可能收到 `EINVAL`，进而影响 nginx 初始化或运行：

```c
struct epoll_event ev = {
    .events = EPOLLIN | EPOLLEXCLUSIVE,
    .data.fd = listen_fd,
};
epoll_ctl(epfd, EPOLL_CTL_ADD, listen_fd, &ev);
```

因此，本 PR 的兼容动机是合理的：StarryOS 需要接受 Linux 合法的 `EPOLLEXCLUSIVE` 使用方式，至少先保证 nginx multi-worker 可运行。

## 术语

`epoll_create1()` 创建一个 epoll 实例，返回 epoll fd，通常记为 `epfd`。

`epoll_ctl(epfd, op, fd, event)` 控制 `epfd` 的 interest list。

`op` 是 operation，表示本次 `epoll_ctl()` 要执行的操作：

`EPOLL_CTL_ADD` 表示把 `fd` 加入 `epfd` 的监听集合。

`EPOLL_CTL_MOD` 表示修改已经加入的 `fd` 的事件配置。

`EPOLL_CTL_DEL` 表示从监听集合删除 `fd`。

`event.events` 是事件和输入 flag 的 bitmask，例如 `EPOLLIN`、`EPOLLOUT`、`EPOLLET`、`EPOLLONESHOT`、`EPOLLEXCLUSIVE`。

## Linux ABI 依据

已在 dev container 中安装并阅读：

`man 2 epoll_ctl`

`man 7 epoll`

`man 2 epoll_wait`

同时对照 Linux `fs/eventpoll.c` 的 `do_epoll_ctl()` 逻辑。

Linux 对 `EPOLLEXCLUSIVE` 的关键 ABI 语义如下：

`EPOLLEXCLUSIVE` 只能用于 `EPOLL_CTL_ADD`。

`EPOLL_CTL_MOD` 的 `event.events` 中包含 `EPOLLEXCLUSIVE` 时返回 `EINVAL`。

如果某个 `(epfd, fd)` entry 已经通过 `EPOLLEXCLUSIVE` 添加，则后续对同一 entry 执行 `EPOLL_CTL_MOD` 返回 `EINVAL`，即使新的 `event.events` 不包含 `EPOLLEXCLUSIVE`。

target `fd` 是 epoll instance 时，`event.events` 包含 `EPOLLEXCLUSIVE` 返回 `EINVAL`。

`EPOLLEXCLUSIVE` 只能与以下 bit 组合：

```text
EPOLLIN
EPOLLOUT
EPOLLERR
EPOLLHUP
EPOLLWAKEUP
EPOLLET
EPOLLEXCLUSIVE
```

因此，以下组合应返回 `EINVAL`：

```text
EPOLLONESHOT | EPOLLEXCLUSIVE
EPOLLRDHUP | EPOLLEXCLUSIVE
EPOLLPRI | EPOLLEXCLUSIVE
EPOLLRDNORM | EPOLLEXCLUSIVE
EPOLLRDBAND | EPOLLEXCLUSIVE
EPOLLWRNORM | EPOLLEXCLUSIVE
EPOLLWRBAND | EPOLLEXCLUSIVE
其他不在 EPOLLEXCLUSIVE_OK_BITS 中的 bit | EPOLLEXCLUSIVE
```

Linux 还会在 `EPOLL_CTL_ADD` 和 `EPOLL_CTL_MOD` 内部将 `EPOLLERR | EPOLLHUP` 加入 interest mask，因为这两个事件是 always-reported bits。用户不指定也会被报告。

`EPOLLWAKEUP` 是电源管理相关 flag。Linux 在调用者无 `CAP_BLOCK_SUSPEND` 或内核无 PM sleep 支持时会清除或静默忽略该 bit。StarryOS 若暂不实现 suspend blocker，也应倾向于识别并忽略该 bit，而不是把它作为未知 bit 直接拒绝。

## 当前 StarryOS 实现问题

当前分支在 `os/StarryOS/kernel/src/file/epoll.rs` 中加入：

```rust
const EXCLUSIVE = EPOLLEXCLUSIVE;
```

但 `os/StarryOS/kernel/src/syscall/io_mpx/epoll.rs` 当前对 `EPOLL_CTL_ADD` 和 `EPOLL_CTL_MOD` 共用同一个 `parse_event()`：

```rust
let parse_event = || -> AxResult<(EpollEvent, EpollFlags)> {
    let event = read_epoll_event(event)?;
    let events = IoEvents::from_bits_truncate(event.events);
    let flags =
        EpollFlags::from_bits(event.events & !events.bits()).ok_or(AxError::InvalidInput)?;
    Ok((
        EpollEvent {
            events,
            user_data: event.data,
        },
        flags,
    ))
};
```

这会造成以下问题：

`EPOLL_CTL_ADD` with `EPOLLIN | EPOLLEXCLUSIVE` 会通过，这是本 PR 需要的正例。

`EPOLL_CTL_MOD` with `EPOLLIN | EPOLLEXCLUSIVE` 也会通过，违反 Linux ABI。

`EPOLLONESHOT | EPOLLEXCLUSIVE` 会通过，违反 Linux ABI。

`EPOLLRDHUP | EPOLLEXCLUSIVE`、`EPOLLPRI | EPOLLEXCLUSIVE` 等组合会通过，违反 Linux ABI。

`EpollInterest` 当前不保存 `exclusive` 状态，`TriggerMode::from_flags()` 只消费 `EPOLLONESHOT` 和 `EPOLLET`，所以无法判断旧 entry 是否通过 `EPOLLEXCLUSIVE` 添加，也无法实现“exclusive entry 后续 `EPOLL_CTL_MOD` 返回 `EINVAL`”。

当前改动只是在参数解析层接受 `EPOLLEXCLUSIVE`，并未实现 Linux 的 exclusive wakeup 行为。短期这可以作为 nginx 兼容 no-op，但必须明确不能声称已实现防 thundering herd 的唤醒语义。

## 共识

优先对齐 Linux ABI 的接受/拒绝语义，先保证 StarryOS 对用户态的 syscall 兼容性。

短期可以只把 `EPOLLEXCLUSIVE` 作为 ABI 兼容状态记录，不实现真正 exclusive wakeup 优化。

PR 描述和代码注释需要明确：当前目标是允许 Linux 合法的 `EPOLLEXCLUSIVE` 注册路径通过，并拒绝 Linux 明确非法的组合；实际唤醒行为仍可能等同非 exclusive。

后续若要完整实现性能语义，需要在底层 wait queue 或 `PollSet` 支持 exclusive waiter，而不是只在 epoll entry 中保存 flag。

## 修改方案

### 1. syscall 层按 op 分支解析

不要让 `EPOLL_CTL_ADD` 和 `EPOLL_CTL_MOD` 共用一个没有 op 语义的 `parse_event()`。

建议将解析拆成：

```rust
fn parse_add_event(raw: epoll_event, fd: i32) -> AxResult<(EpollEvent, EpollFlags)>;
fn parse_mod_event(raw: epoll_event) -> AxResult<(EpollEvent, EpollFlags)>;
```

或保留一个函数但显式传入 `op`：

```rust
fn parse_event_for_op(op: u32, raw: epoll_event, fd: i32) -> AxResult<(EpollEvent, EpollFlags)>;
```

### 2. 在 syscall 层定义 EPOLLEXCLUSIVE 合法组合

对齐 Linux：

```rust
const EPOLLEXCLUSIVE_OK_BITS: u32 =
    EPOLLIN | EPOLLOUT | EPOLLERR | EPOLLHUP | EPOLLWAKEUP | EPOLLET | EPOLLEXCLUSIVE;
```

如果 `linux_raw_sys` 未导出某些常量，应先确认导出路径；尽量避免手写魔数。若必须手写，需要注释标明 Linux ABI 常量来源。

### 3. EPOLL_CTL_ADD 语义

`EPOLL_CTL_ADD` 应读取 `event`。

若 StarryOS 暂不实现 suspend blocker，应清除或忽略 `EPOLLWAKEUP`。

如果 `event.events` 包含 `EPOLLEXCLUSIVE`：

检查 `event.events & !EPOLLEXCLUSIVE_OK_BITS == 0`，否则返回 `EINVAL`。

检查 target `fd` 不是 epoll instance，否则返回 `EINVAL`。

将 `exclusive` 状态保存到 epoll entry。

### 4. EPOLL_CTL_MOD 语义

`EPOLL_CTL_MOD` 应读取 `event`。

若 StarryOS 暂不实现 suspend blocker，应清除或忽略 `EPOLLWAKEUP`。

如果新的 `event.events` 包含 `EPOLLEXCLUSIVE`，返回 `EINVAL`。

如果旧 entry 是通过 `EPOLLEXCLUSIVE` 添加的，返回 `EINVAL`。

否则正常修改 entry。

### 5. EPOLL_CTL_DEL 语义

`EPOLL_CTL_DEL` 应继续忽略 `event`，允许 `event == NULL`。

当前 StarryOS 的 `DEL` 分支不调用 `parse_event()`，这个方向正确。

### 6. EpollInterest 保存 exclusive 状态

最小改动是在 `EpollInterest` 中增加：

```rust
exclusive: bool,
```

或保存完整：

```rust
flags: EpollFlags,
```

`Epoll::modify()` 应在替换旧 interest 前检查旧 entry：

```rust
if old.is_exclusive() {
    return Err(AxError::InvalidInput);
}
```

这样即使 syscall 层遗漏检查，core 层也不会允许 exclusive entry 被 `EPOLL_CTL_MOD` 修改。

## 测试方案

应扩展 `test-suit/starryos/normal/qemu-smp1/syscall/test-epoll-exclusive`。

保留并加强正例：

`EPOLL_CTL_ADD` with `EPOLLIN | EPOLLEXCLUSIVE` 返回 0。

可选增加 `EPOLL_CTL_ADD` with `EPOLLOUT | EPOLLEXCLUSIVE` 返回 0。

可选增加 `EPOLL_CTL_ADD` with `EPOLLIN | EPOLLET | EPOLLEXCLUSIVE` 返回 0。

必须增加负例：

`EPOLL_CTL_MOD` with `EPOLLIN | EPOLLEXCLUSIVE` 返回 `EINVAL`。

`EPOLL_CTL_ADD` with `EPOLLIN | EPOLLONESHOT | EPOLLEXCLUSIVE` 返回 `EINVAL`。

`EPOLL_CTL_ADD` with `EPOLLIN | EPOLLRDHUP | EPOLLEXCLUSIVE` 返回 `EINVAL`。

`EPOLL_CTL_ADD` with `EPOLLPRI | EPOLLEXCLUSIVE` 返回 `EINVAL`。

先 `ADD EPOLLIN | EPOLLEXCLUSIVE`，再 `MOD EPOLLIN` 返回 `EINVAL`。

target fd 是 epoll instance 时，`ADD EPOLLIN | EPOLLEXCLUSIVE` 返回 `EINVAL`。

## unknown bit 测例分歧

当前 `test-epoll-exclusive` 包含“普通 unknown bit 应返回 `EINVAL`”的负例。复查 Linux `eventpoll.c` 后，这一点不能直接作为 Linux ABI 结论使用。

Linux 对 `EPOLLEXCLUSIVE` 场景有明确 mask 校验：`event.events & ~EPOLLEXCLUSIVE_OK_BITS` 非 0 时返回 `EINVAL`。

但 Linux 对普通非 `EPOLLEXCLUSIVE` 场景并没有同样的全量 unknown bit 拒绝逻辑。部分未知或私有 bit 可能被保留、忽略或随路径表现不同。

因此，关于 unknown bit 的测试应先在 Linux 环境下运行，记录实际 Linux 行为，再决定 StarryOS 测例应如何改进。

建议新增一个 host/Linux 探针程序，至少记录以下行为：

```text
EPOLL_CTL_ADD: EPOLLIN | (1u << 27)
EPOLL_CTL_MOD: EPOLLIN | (1u << 27)
EPOLL_CTL_ADD: EPOLLIN | EPOLLEXCLUSIVE | (1u << 27)
```

记录每个 case 的返回值和 `errno`，再据此调整 StarryOS 测例。不要在未验证 Linux 实际行为前，把普通 unknown bit 负例写成“Linux 语义”。

## 推荐 PR 描述口径

本 PR 应描述为：

```text
对齐 Linux epoll_ctl() 对 EPOLLEXCLUSIVE 的 ABI 校验，允许 nginx multi-worker 使用的合法 EPOLLIN | EPOLLEXCLUSIVE 注册路径通过，并拒绝 Linux 明确非法的 MOD、ONESHOT、RDHUP、PRI、epoll target 等组合。StarryOS 当前仅记录 EPOLLEXCLUSIVE 作为 ABI 状态，不实现 exclusive wakeup 优化；实际防 thundering herd 行为留待后续 PollSet/wait queue 层支持。
```

## 后续工作

实现 syscall 层 `EPOLLEXCLUSIVE` op 分支校验。

保存 entry 的 `exclusive` 状态并禁止后续 `EPOLL_CTL_MOD`。

补齐 StarryOS QEMU 回归测试。

在 Linux 上先运行 unknown bit 探针并记录结果，再调整相关测试断言。

运行 `cargo fmt`。

运行 `cargo xtask clippy --package starry-kernel`。

运行相关 StarryOS syscall grouped case，例如 `cargo xtask starry test qemu --arch x86_64 -c syscall`。
