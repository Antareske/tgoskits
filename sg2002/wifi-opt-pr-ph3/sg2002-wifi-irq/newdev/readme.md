9/4 14:36

情况说明：aic8800d80 继续修复

dev主线的 aic8800 驱动在几天内发生全面重构，重构到一半时本人提交了 aic8800d80 的启动问题修复 pr（www/newdev/archive-august/PR描述-sg2002-wifi-irq-initFix.md），见 wt-sg2002-wifi-irq-fixInit，之后 dev 主线部分采纳了该 pr（www/newdev/archive-august/pr-rsp.md），
./archive-august 都是该 pr 时期相关内容（对应 pr 的 wt-sg2002-wifi-irq-fixInit）；主线 dev 继续重构+修复并发布了之后的 aic8800 修复提交，但是在 aic8800d80 上实测启动时 panic，似乎是主线那边用的不是 aic8800d80 的硬件做的测试，因此在本分支在 dev 的基础上继续做 aic8800d80 的修复工作。

panic 第一处：www/logs/931.log（纯 dev 镜像），本分支做最小修复（www/newdev/dev主线wakeup回读值校验panic分析-2026-09-03.md）后出现 panic 第二处：www/logs/2.log。

从现在开始在 www/newdev 中维护一份类似 www/newdev/archive-august/问题修复追踪.md 的 "9月问题修复追踪.md"，记录本分支相对 dev 的 aic8800d80 修复追踪，文档要求保证正确性。

其它：

./sg2002-image-build 要求把生成的镜像都放到本工作树的相应位置；通常会要求你使用 swap kernel 复用之前的镜像；每次内核改动重新编译建议先删除 tmp 中的 starry 编译相关中间产物（不要 cargo clean，不要全删 tmp）。