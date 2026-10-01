W04 - 异步串口契约收敛 + benchmark 补测收尾¶
周期：2026-07-11 ~ 2026-07-18

分支：uart-16550-lichee

提交数：23 个

本周工作¶
TX backpressure：阻塞写不再短写¶
之前 write(fd, buf, 1024) 在 TX ring buffer 满时返回短写（只写了部分字节）。现在阻塞模式下会等待 ring buffer 腾出空间，保证完整提交。

两端验证通过： - QEMU：/bin/benchmark 正常，无性能退化 - D1 真板：64B 达 96.8% 线速，1024B 达 98.8% 线速，短写从 36 次/65536B 降为 0

commit 4253725

Writer 契约收敛：移除 Clone，锁定 SPSC¶
AsyncUartWriter 之前可以 Clone，多个 writer 共享同一个 TX ring buffer 但没做并发保护。现在改为不可 Clone，unsafe 唯一构造，&mut self 提交写入。

内核层通过 Arc<SpinNoPreempt<Writer>> 保留 cloneable adapter，锁不跨等待点。

关键指标：QEMU P50 改善 7.36%-15.75%，D1 最大退化 64B P50 +0.107%（单次样本，不声明统计显著性）。

commit a84b1b1

Reader 契约收敛：锁定 SPSC 消费者¶
对应 writer 的对称收敛——每个驱动只允许一个 unsafe raw reader，RX 数据 push/pop 不向 crate 外 safe API 开放。copier 恰好启动一次且晚于 benchmark，消除启动期的角色冲突。

验证：62 unit + 8 doctest + 10 compile-fail 通过，QEMU build+boot 与 D1 benchmark 退出码 0。

commit ad05ae9

Benchmark 补测：TX 延迟、抖动、CPU 计数¶
补测了之前缺失的 TX 指标： - jitter summary：p99_p50_ratio、max_p50_ratio、slow_over_line_plus10ms - CPU/counter proxy：S40 输出 user/ring/hw/no-progress/drain 计数器

D1 结果：64B 约 96.7% 线速，1024B 约 98.8% 线速，slow_poll_exh=0、yield_exh=0。RX fixed payload 不做。SMP 正确性不在此次验证范围。

commit e5e42d9

取消 user ring / completion queue¶
D1 真板 115200 bps 已是吞吐瓶颈（驱动达 96-99% 线速）。现有 TX ring + copier 已覆盖提交/执行分离，加用户态 completion queue 或 mmap ring 在当前波特率下无可见收益。保留为远期选项。

commit b81f0cd

下周¶
维护性清理，四项：

评估 memtrack 是否集成，不集成则记录移除决策
评估 ProcessMode::Manual 是否还有使用方，决定保留或删除
清理超过 90 天未使用的预留接口，保留的加注释说明用途
release LTO 检查：开发期不启用，发版前恢复
目标是不留模棱两可的债务项，每项有明确结论。commit 7836240

参考¶
本周整理了两篇总结性笔记，放在 notes/ 下：

异步 UART 驱动总体架构：环形缓冲区、copier 任务、ISR→waker 机制、Writer/Reader unsafe 契约、与 io_uring 的对比与取舍
异步 UART 性能测试设计与结果：测试设计思路、各 section 测什么及结果含义、边界与不做的事情
性能测试报告