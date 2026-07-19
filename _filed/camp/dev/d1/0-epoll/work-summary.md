# 例会工作总结大纲

## 一、参与项目背景

参与 StarryOS 方案一项目实习，目标是通过编写 Linux 兼容性测例、测试并修复 StarryOS 中系统调用的行为偏差，推动内核与 Linux 语义对齐。

领取的两组系统调用：
- **事件复用**：`epoll_create1`、`epoll_ctl`、`epoll_pwait`、`epoll_pwait2`
- **文件同步**：`fsync`、`fdatasync`、`sync_file_range`、`sync`、`syncfs`

---

## 二、已完成：epoll 测例补充（已提交）

**工作内容**

依据 Linux man 手册语义，为四个 epoll 相关 syscall 编写 C 源码级用户态测例，并接入四架构 CI。

**两个测例**

- `test-epoll`：基于 pipe 验证 epoll 核心语义，覆盖 12 个场景，包括 `EPOLLIN`/`EPOLLOUT`/`EPOLLHUP`/`EPOLLET`、`epoll_ctl` 的 ADD/MOD/DEL、多 fd 并发、`fork` 继承、错误路径（`EBADF`/`EINVAL`/`EEXIST`/`ENOENT`）等。
- `test-epoll-pwait2`：通过 `syscall()` 直接调用内核，验证 `epoll_pwait2` 的 ABI 语义，覆盖超时、信号掩码、`EFAULT`、`EINTR` 等边界情况。

**结论**

测例在四个架构（x86_64、aarch64、riscv64、loongarch64）上全部通过，**未发现 StarryOS 行为 bug**，本次 PR 仅为补充测例覆盖。

---

## 三、进行中：文件同步 syscall 语义修复

**工作内容**

为 `syncfs` 和 `sync_file_range` 编写测例，测试过程中发现两个 syscall 均存在行为不一致问题，经排查确认根本原因是实现为 dummy stub，未做任何参数校验。

**发现的 Bug**

| Syscall | 场景 | Linux 期望 | 实际行为 |
|---|---|---|---|
| `syncfs` | 无效 fd（如 `-1`、已关闭 fd） | `-1`，`EBADF` | `0`（错误地成功） |
| `sync_file_range` | `flags` 含非法位（如 `0xFF`） | `-1`，`EINVAL` | `0` |
| `sync_file_range` | pipe fd | `-1`，`ESPIPE` | `-1`，`EINVAL`（errno 错误） |

**根本原因**

- `sys_syncfs`：参数命名为 `_fd`，从未读取，任何调用均返回 `Ok(0)`。
- `sys_sync_file_range`：`flags` 命名为 `_flags` 被丢弃，未校验非法位；pipe fd 经 `downcast_arc::<File>()` 失败后映射到 `EINVAL` 而非 `ESPIPE`。

**本次修复内容**

- `syncfs`：调用 `get_file_like(fd)` 做 fd 合法性校验，无效 fd 返回 `EBADF`；实际文件系统回写仍为 no-op（in-memory VFS 语义正确）。
- `sync_file_range`：新增 flags 非法位检查（`flags & !0x7 != 0` → `EINVAL`）；改用 `downcast_ref` 双类型检查区分 `File`/`Directory` 与其他 fd 类型，pipe 等非文件 fd 返回 `ESPIPE`；range-based writeback 仍为 no-op（符合 advisory 语义）。

**当前状态**

内核修复已完成，测例已编写并接入四架构 CI toml，PR 准备中。

---

## 四、后续计划

- 完成文件同步 PR 的本地验证，提交 PR（含测例源码 + 内核修复）。
- 视情况继续推进 `fsync`、`fdatasync`、`sync` 的测例覆盖。
