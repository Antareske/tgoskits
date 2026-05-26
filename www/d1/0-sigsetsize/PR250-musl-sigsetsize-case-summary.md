# PR250 musl/sigsetsize 案情整理

## 目的

本文整理当前对 PR250 的认知，用于后续提交 PR 或问题分析时说明：

- PR250 当时想解决什么问题
- 当时的前提假设是什么
- Linux raw syscall ABI 的真实行为是什么
- musl 实际上是否会把 `sigsetsize=16` 传给内核
- 仓库里的现有测例和网络场景，哪些能支持原说法，哪些不能

本文只总结当前证据，不修改任何测例结论。

---

## 一、PR250 原始说法

`www/1-sigsetsize/PR250说明.md` 记录了 PR250 的原始判断：

1. StarryOS 的 `epoll_pwait` 在 musl libc 下返回 `EINVAL`
2. 原因是 musl 的 `epoll_pwait` wrapper 会把 `sigsetsize=16` 传给内核
3. StarryOS 当时只接受 `8` 或 `0`，因此拒绝了 `16`
4. 为兼容 musl，于是把 `check_sigset_size` 放宽为接受任意 `>= 8`

对应描述见：

- `www/1-sigsetsize/PR250说明.md:11`
- `www/1-sigsetsize/PR250说明.md:17`

PR250 的出发点是合理的：它试图解释“musl 程序 + epoll 信号掩码”场景为何失败，并给出兼容性修复。

但后续核查表明，**其中关于 musl 和 Linux 内核行为的关键前提存在问题**。

---

## 二、musl 到底是什么

`musl` 是一套 Linux 用户态 `libc` 实现，和 `glibc` 属于同一类东西。

- `glibc`：GNU C Library，Debian/Ubuntu 等常见发行版默认使用
- `musl`：另一套 libc 实现，Alpine Linux 等发行版常见使用

它们都负责两件事：

1. 向用户程序暴露标准 C/POSIX 接口，如 `epoll_pwait`、`ppoll`、`signalfd`
2. 在内部把这些接口封装成真正的 raw syscall ABI 调用

因此，一个关键区分是：

- man page 常先展示 **libc 暴露的函数原型**
- 真正进入内核时，走的是 **raw syscall ABI**

这也是为什么文档表面上可能看不到 `sigsetsize`，但裸 `syscall(...)` 测例里会出现它。

---

## 三、这次问题里的 `sigmask` / `sigsetsize` 是什么

对 `epoll_pwait` / `epoll_pwait2` / `ppoll` / `pselect6` 这类接口来说，`sigmask` 的语义是：

- 在这一次等待期间，临时替换当前线程的信号屏蔽字
- syscall 返回后，再恢复原来的屏蔽字

raw syscall ABI 还需要知道：

- 这个 `sigmask` 指针指向的数据，按多大的内核 ABI 去解释

因此 raw syscall ABI 会额外携带 `sigsetsize`。

仓库里的 syscall 文档已经明确写出这一点：

- `www/syscall-man/epoll_wait.md:49`
- `www/syscall-man/ppoll.md:35`
- `www/syscall-man/signalfd.md:39`

即：

- raw `epoll_pwait()` / `epoll_pwait2()` 有第 6 个参数 `sigsetsize`
- raw `ppoll()` 有第 5 个参数 `sigsetsize`
- raw `signalfd4()` 也有 `sigsetsize`

---

## 四、Linux raw syscall ABI 的真实行为

当前仓库内的 Linux 实测与分析文件已经给出一致结论：

- 当 `sigmask != NULL` 时，Linux 对 `sigsetsize` 的要求是**精确等于 8**
- 不是“`>= 8`”
- 不是“16 也可以”
- 不是“128 也可以”

可直接参考：

- `www/1-sigsetsize/sigsetsize-linux-compat-full.md:13`
- `www/1-sigsetsize/sigsetsize-linux-compat-full.md:26`
- `www/1-sigsetsize/sigsetsize-compat-analysis.md:72`

这些分析的统一结论是：

- `epoll_pwait` / `epoll_pwait2` / `ppoll` / `signalfd4` / `rt_sigprocmask`
- 在 raw syscall 边界上，只接受内核侧 `kernel_sigset_t` 的大小
- 该大小在这里是 `8`

另外还有一个容易混淆但很重要的特例：

- `epoll_pwait(NULL mask)` / `epoll_pwait2(NULL mask)` / `ppoll(NULL mask)`
- Linux 不校验 `sigsetsize`

对应参考：

- `www/1-sigsetsize/sigsetsize-linux-compat-full.md:28`

---

## 五、musl 是否真的会把 16 传给内核

当前找到的 upstream musl 源码证据表明：**不会**。

### 5.1 musl 的 `sigset_t` 确实可以是 16 字节

upstream musl `include/alltypes.h.in` 定义：

```c
TYPEDEF struct __sigset_t { unsigned long __bits[128/sizeof(long)]; } sigset_t;
```

在 64 位平台上，这是 16 字节。

所以“musl 的 `sigset_t` 是 16 字节”这半句没有问题。

### 5.2 但 musl 的 wrapper 传给内核的是 `_NSIG/8`

upstream musl `src/linux/epoll.c` 中：

```c
int epoll_pwait(int fd, struct epoll_event *ev, int cnt, int to, const sigset_t *sigs)
{
    int r = __syscall_cp(SYS_epoll_pwait, fd, ev, cnt, to, sigs, _NSIG/8);
    ...
}
```

这说明 musl wrapper 传给内核的不是 `sizeof(sigset_t)`，而是 `_NSIG/8`。

### 5.3 musl 在 x86_64 上 `_NSIG` 是 65

upstream musl `arch/x86_64/bits/signal.h`：

```c
#define _NSIG 65
```

因此：

```c
_NSIG / 8 == 65 / 8 == 8
```

也就是说：

- musl 的 `sigset_t` 大小可以是 16 字节
- 但 musl `epoll_pwait` wrapper 实际上传给内核的 `sigsetsize` 是 8

### 5.4 这与 PR250 的前提相冲突

因此，PR250 的如下推断链条不能成立：

1. musl `sigset_t` 是 16 字节
2. 所以 musl wrapper 会把 16 传给内核
3. 所以内核应宽松接受 16

第 1 步可以成立，但第 2 步不成立。

更准确的说法应当是：

- musl 用户态类型可能是 16 字节
- 但 musl wrapper 会把 ABI 参数规范化为内核期望值
- 对 `epoll_pwait` 来说，这个值是 `_NSIG/8 == 8`

仓库内已有分析也得出了相同结论：

- `www/1-sigsetsize/sigsetsize-compat-analysis.md:103`
- `www/1-sigsetsize/sigsetsize-compat-analysis.md:105`
- `www/1-sigsetsize/sigsetsize-linux-compat-full.md:204`

---

## 六、宿主机实验目前说明了什么

当前在宿主 Linux 上运行 prefix 为 `1-` 的失败测例，可以看到：

### 6.1 `1-test-epoll-pwait-sigsetsize`

- 失败点：`sigmask + size=16` 期望成功，但 Linux 返回 `EINVAL`
- 对应源码：`test-suit/starryos/normal/qemu-smp1/syscall/test-epoll-pwait-sigsetsize/c/src/main.c:65`

### 6.2 `1-test-epoll-pwait2`

- 失败点：`sigmask + sizeof(sigset_t)` 期望成功，但 Linux 返回 `EINVAL`
- 对应源码：`test-suit/starryos/normal/qemu-smp1/syscall/test-epoll-pwait2/c/src/main.c:111`
- 另一处因同样原因导致带 mask 的等待场景直接 `EINVAL`
- 对应源码：`test-suit/starryos/normal/qemu-smp1/syscall/test-epoll-pwait2/c/src/main.c:262`

### 6.3 `1-test-pselect-ppoll`

- `raw_pselect6` 与 `raw_ppoll` 直接传 `sizeof(sigset_t)`
- 宿主 Linux 上多处返回 `EINVAL`
- 对应源码：`test-suit/starryos/normal/qemu-smp1/syscall/test-pselect-ppoll/c/src/main.c:19`
- 对应源码：`test-suit/starryos/normal/qemu-smp1/syscall/test-pselect-ppoll/c/src/main.c:29`

### 6.4 `1-test-signalfd4`

- `sigsetsize=0` 期望成功，但 Linux 返回 `EINVAL`
- 对应源码：`test-suit/starryos/normal/qemu-smp1/syscall/test-signalfd4/c/src/main.c:133`

这些实验共同说明：

- raw syscall 测例里“传 16/128 也该成功”的预期，与 Linux 实测不一致
- 当前失败是**raw syscall ABI 预期错误**，不是 Linux 的异常行为

---

## 七、现有网络测例能否支持 PR250 的“网络应用失败”说法

目前只能说：**项目里确实有依赖 epoll 的网络场景，但没有找到足以证明 PR250 因果链的直接证据。**

### 7.1 能证明“项目里有 epoll 网络场景”

#### `test-epoll-network`

源码：

- `test-suit/starryos/normal/qemu-smp1/syscall/test-epoll-network/c/src/main.c:48`

它是一个网络 + epoll 综合测例，标题里甚至直接写了 `Tokio-Compat Suite`。

但它调用的是：

- `epoll_wait`

而不是：

- `epoll_pwait`

因此它**不能直接作为 “musl epoll_pwait 传 16 导致网络应用失败” 的证据**。

#### `bug-tcp-send-no-epoll-notify`

源码：

- `test-suit/starryos/normal/qemu-smp1/bugfix/bug-tcp-send-no-epoll-notify/c/src/main.c:2`

它验证的是：

- TCP loopback 写入后，peer 上注册的 epoll 必须被唤醒

其根因写得很清楚：

- `TcpSocket::send()` 写后没有再次 `poll_interfaces()`

这也是一个真实的 epoll 网络问题，但它的根因是**网络栈唤醒路径**，不是 `sigsetsize`。

### 7.2 目前缺失的证据

目前仓库里没有找到这样一条完整证据链：

1. 某个真实 musl 用户态网络程序
2. 通过 libc wrapper 调 `epoll_pwait`
3. wrapper 把 `sigsetsize=16` 传给 StarryOS 内核
4. StarryOS 因拒绝 16 而返回 `EINVAL`
5. 最终导致网络功能失败

现有网络测例可以证明“epoll 网络场景存在”，但**不能证明 PR250 所述的那条具体因果链**。

---

## 八、当前最稳妥的案情结论

基于目前证据，可以较稳妥地得出以下结论：

1. PR250 想解决 musl/epoll 兼容性问题，这个方向本身是合理的。
2. 但 PR250 的关键前提“musl wrapper 会把 `sigsetsize=16` 传给内核”与 upstream musl 源码不一致。
3. Linux raw syscall ABI 对 `sigsetsize` 的要求是：
   - `sigmask != NULL` 时精确等于 8
   - `epoll_pwait(NULL mask)` / `epoll_pwait2(NULL mask)` / `ppoll(NULL mask)` 不校验 size
4. 因此，“把 StarryOS 放宽为接受任意 `>= 8`”不是对齐 Linux，而是偏离 Linux。
5. 当前 prefix 为 `1-` 的失败测例，正是在宿主 Linux 上暴露了这一点。
6. 现有网络测例只能证明“项目里有依赖 epoll 的网络场景”，不能单独证明 PR250 所述的 musl 因果链。

---

## 九、后续提交 PR/分析时建议怎么表述

建议把结论分成“确认事实”和“尚无充分证据”两部分。

### 9.1 可以确认的事实

- Linux raw syscall ABI 确实存在 `sigsetsize` 参数
- Linux 对 non-NULL `sigmask` 的 `sigsetsize` 要求是精确等于 8
- musl 的 `sigset_t` 可以是 16 字节，但 musl `epoll_pwait` wrapper 传的是 `_NSIG/8`
- 在 x86_64 上，musl 的 `_NSIG/8 == 8`
- PR250 新增的部分 raw syscall 测例预期与 Linux 不对齐

### 9.2 不应直接下结论的部分

- “musl wrapper 会把 16 传给内核”
- “依赖 epoll 的网络应用失败，根因就是内核拒绝 16”

这两条目前都缺少充分证据，至少现有仓库材料和宿主实验还不能支撑。

---

## 十、一句话总结

当前最合理的认知是：

**PR250 识别到了一个真实的兼容性关注点，但把 musl 用户态 `sigset_t` 的大小，误当成了 musl wrapper 传给 raw syscall ABI 的 `sigsetsize`；Linux 和 upstream musl 的现有证据都表明，这个前提不成立。**
