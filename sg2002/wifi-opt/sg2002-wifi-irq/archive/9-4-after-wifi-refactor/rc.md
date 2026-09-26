本 PR 将 SG2002/AIC8800 的 PIO 传输完成从让出轮询改为 XFER_COMPLETE 中断唤醒，并为 RX kicker、选择性 W1C、运行时 glue 和 ITS 描述补充配套改动。变更触及 SDHCI 完成语义与板级 WiFi 工作流，功能开发准则适用；实现仍局限于现有驱动核心与 ArceOS glue，未新增对外 API。

阻塞问题：XFER_COMPLETE 的 WaitQueue 等待存在丢唤醒窗口，导致实际完成路径可退化为完整 10ms timeout，不能满足本 PR 的中断即时唤醒目标；详见行内评论。

验证：`cargo fmt --all --check` 通过；`cargo clippy --manifest-path components/sdhci-cv1800/Cargo.toml --all-features -- -D warnings` 通过，`cargo test --manifest-path components/sdhci-cv1800/Cargo.toml --all-features` 通过（0 tests）；aic8800 的同等 clippy 通过、测试 4/4 通过。`axruntime --all-features` 因基线中 userspace 与 kernel TLS 不兼容而在 `ax-cpu` 失败，未把它归因于本 PR。该 head 在 rcore-os/tgoskits 未报告 check run 或传统 commit status；当前环境也无法复现 LicheeRV Nano 的板卡运行。

已检查历史审查与 PR 评论：没有已有审查或讨论需要处理。已检查 base 与开放 PR 搜索结果：未发现除本 PR 外的同一 sdhci-cv1800/aic8800/SG2002 WiFi 实现，未见重复或合并冲突风险。

待修复后请增加能覆盖“ISR 在 waiter 入队前发生”的确定性回归验证，并在实际 SG2002 工作流上重跑上传、下载和双向测试。

Powered by gpt-5.6-terra

<!-- mai-review-job:e351873c-2d37-47f1-bec9-3a06fb869d71 -->



components/sdhci-cv1800/src/lib.rs:205-208

阻塞：这里仍有丢唤醒窗口。`unmask_xfer_complete_signal()` 后、`block_timeout()` 将任务加入 `WaitQueue` 前，传输可能完成；ISR 会屏蔽 XFER 信号并调用 `notify_one_from_irq()`，但队列为空时该通知不会被保存。随后任务只能等满 10ms timeout 才在 recheck 中看到 sticky bit，微秒级完成路径会退化。请用 ISR 发布的 pending/completion 状态配合条件等待，或把清状态、开信号、检查和入队纳入不会丢事件的同步协议，保证 ISR 先发生时等待方不会睡过该事件。