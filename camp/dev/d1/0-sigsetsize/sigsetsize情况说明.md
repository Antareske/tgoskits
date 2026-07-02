在 Linux 运行 `epoll_pwait2` 测例未通过。继续追查后发现，StarryOS 在处理这类带 `sigmask` 参数的信号相关 syscall 时，和 Linux 的 `sigsetsize` 语义存在偏差。在 `test-suit/starryos/normal/qemu-smp1/syscall` 范围内，至少有 `test-epoll-pwait-sigsetsize`、`test-epoll-pwait2`、`test-pselect-ppoll`、`test-signalfd4` 四个在 starryos 跑通的测例，在 Linux 用户态运行都未通过。

## 说明

这类 raw syscall ABI 里，libc 需要额外补一个 `sigsetsize` 参数，用来表示 `sigmask` 的有效字节数，该有效字节数对应 `sigmask` 在内核中 u64 的 `SignalSet` 类型，因此应当传入 8。

在 Linux 上观察到的行为是：

- 当 `sigmask == NULL` 时，`sigsetsize` 不参与校验。
- 当 `sigmask != NULL` 时，`sigsetsize` 必须严格等于 8（也不接受 0）。

而 StarryOS 当前实现允许更宽松的取值范围。PR #250 之前，内核只允许 `0` 和 `8`（`size_of::<SignalSet>()`）；#250 之后放宽为允许 `0` 和任意大于等于 `8` 的值，均和 Linux 语义不符。

由于 #250 中提到 "16（musl `sigset_t` 大小）"，可能是误把用户态 128 位 `sigset_t` 对应的字节数 16 当作正确输入，因此推断 #250 的修改有误，在 `sigmask` 非空时，starryos 应该严格检查 `sigsetsize` 是否为 8。而 #250 提到的网络应用错误有可能是应用本身传递了错误的参数。在 Linux 环境下，上述的 `sigsetsize` 相关测例，包括一例 #250 新增测例，都因为这一原因未通过。

## musl 源码确认

#250 提到 musl 的 `epoll_pwait` 封装传入了 `sigsetsize=16`。musl 的 `epoll_pwait` 封装如下：

<https://git.musl-libc.org/cgit/musl/tree/src/linux/epoll.c>

```c
int epoll_pwait(int fd, struct epoll_event *ev, int cnt, int to, const sigset_t *sigs)
{
	int r = __syscall_cp(SYS_epoll_pwait, fd, ev, cnt, to, sigs, _NSIG/8);
#ifdef SYS_epoll_wait
	if (r==-ENOSYS && !sigs) r = __syscall_cp(SYS_epoll_wait, fd, ev, cnt, to);
#endif
	return __syscall_ret(r);
}
```

也就是说 musl 直接传 `_NSIG / 8`。

继续检查 musl 所有架构 `signal.h` 中的 `_NSIG` 定义：

<https://git.musl-libc.org/cgit/musl/tree/arch/*/bits/signal.h>

| `_NSIG` | 架构 |
| ------: | --- |
| 65 | aarch64, arm, generic, i386, loongarch64, m68k, microblaze, or1k, powerpc, powerpc64, riscv32, riscv64, s390x, sh, x32, x86_64 |
| 128 | mips, mips64, mipsn32 |

因此除了 `mips`、`mips64`、`mipsn32` 会传 `sigsetsize = 16`，其余大多数架构上传入的都是 `8`。而 StarryOS 里定义的 `SignalSet` 本身也是 `u64`，因此内核只接受 `8` 更符合 Linux 语义。

## Linux 用户态实验

为了排除测试封装的干扰，又额外写了一个 Linux 用户态 raw syscall 实验，直接调用 `epoll_pwait`，分别测试：`0`、`1`、`7`、`8`、`9`、`16`、`17`、`32`、`sizeof(sigset_t)`。

实验结果如下：

| `sigsetsize` | `sigmask != NULL` | `sigmask == NULL` |
| ---: | --- | --- |
| 0 | `ret=-1 errno=EINVAL` | `ret=0 errno=0` |
| 1 | `ret=-1 errno=EINVAL` | `ret=0 errno=0` |
| 7 | `ret=-1 errno=EINVAL` | `ret=0 errno=0` |
| 8 | `ret=0 errno=0` | `ret=0 errno=0` |
| 9 | `ret=-1 errno=EINVAL` | `ret=0 errno=0` |
| 16 | `ret=-1 errno=EINVAL` | `ret=0 errno=0` |
| 17 | `ret=-1 errno=EINVAL` | `ret=0 errno=0` |
| 32 | `ret=-1 errno=EINVAL` | `ret=0 errno=0` |
| `sizeof(sigset_t)` (`128`) | `ret=-1 errno=EINVAL` | `ret=0 errno=0` |

分别测试了 gcc 和 musl 两套静态链接构建，结果一致。

因此认为，在 Linux 上，`epoll_pwait` 的 raw syscall 路径里，只要 `sigmask` 非空，`sigsetsize` 就只接受 `8`。

## Linux 下未通过的 sigsetsize 相关测例

| 测例 | GCC 静态 Linux 输出 | musl 静态 Linux 输出 | 预期 | 实际 | 原因 |
| --- | --- | --- | --- | --- | --- |
| `test-epoll-pwait-sigsetsize` | `FAIL ... errno=22 (Invalid argument)` | `FAIL ... errno=22 (Invalid argument)` | 测例认为非空 `sigmask` 搭配 `sigsetsize=16` 也应成功 | Linux 返回 `EINVAL` | 测例直接调用 `raw_epoll_pwait(..., &mask, 16)`，把 `16` 当作合法输入；但 Linux 对这条 raw syscall 路径只接受 `8`。 |
| `test-epoll-pwait2` | 两处都 `FAIL ... errno=22 (Invalid argument)` | 两处都 `FAIL ... errno=22 (Invalid argument)` | 测例认为 `sizeof(sigset_t)` 合法，且带 mask 的等待应超时返回 `0` | 首个带 mask 调用就先因 `EINVAL` 失败，后面的 masked wait 也直接失败 | 测例把用户态 `sizeof(sigset_t)` 直接传给 `raw_epoll_pwait2`。在当前 Linux/x86_64 用户态里这个值是 `128`，不是内核要求的 `8`。 |
| `test-pselect-ppoll` | 多个节点都 `FAIL ... errno=22 (Invalid argument)` | 多个节点都 `FAIL ... errno=22 (Invalid argument)` | 测例期望 timeout 场景返回 `0`，有数据时返回 `1` | 这些检查点都被前置的 `EINVAL` 打断 | 测例封装 `raw_pselect6` 和 `raw_ppoll` 时，都把 `sizeof(sigset_t)` 当作长度传入。只要 `sigmask` 非空，就会先撞上 Linux 的 `sigsetsize` 校验。 |
| `test-signalfd4` | 进程提前 `*** buffer overflow detected ***: terminated` | `FAIL ... sigsetsize=0 succeeds | errno=22 (Invalid argument)` | 测例认为 `sigsetsize=0` 应成功 | musl 下该节点返回 `EINVAL`；gcc 版本后续又被 FORTIFY 终止 | 测例在 `test_sigsetsize()` 中直接断言 `syscall(__NR_signalfd4, -1, &mask, 0, 0) >= 0`。但 Linux 实际并不接受这里的 `0`。另外 gcc 静态链接版本在后续 `read(fd, small, 127)` 处还会触发 glibc FORTIFY。 |


## 改动说明

| 项目 | 改动说明 |
| --- | --- |
| 内核 `check_sigset_size` | 改为仅接受 `sigsetsize == 8`。 |
| 内核 `sys_ppoll` | 改为仅在 `sigmask != NULL` 时校验 `sigsetsize`。 |
| `test-epoll-pwait-sigsetsize` | ABI 按 `sigsetsize == 8` 调整断言。 |
| `test-epoll-pwait2` | ABI 按 `sigsetsize == 8` 调整调用和断言。 |
| `test-pselect-ppoll` | ABI 按 `sigsetsize == 8` 调整 raw syscall 封装。 |
| `test-signalfd4` | ABI 按 `sigsetsize == 8` 调整失败预期。 |

已通过本地 syscall 测试和 riscv normal 测试。


## 结论

从对齐 Linux 语义来看，`sigmask != NULL` 时，`sigsetsize` 应当只允许 `8`。而用户态调用 `sigmask` 相关 ABI 时，常见将用户态的 `sizeof(sigset_t)` 传入 `sigsetsize` 的错误，包括未通过的 Linux 参考测例中的相同错误。使用 AI 封装或测试 ABI 时，尤其注意 AI 对此类错误并不敏感。

建议：
从 Linux 兼容性的角度看，内核把 `sigsetsize` 限制为 `8` 更合理；但考虑到 StarryOS 这部分行为已经存在一段时间，而且 PR #250 的动机也是为了解决实际问题，所以尚不清楚这一改动是否会造成影响。
