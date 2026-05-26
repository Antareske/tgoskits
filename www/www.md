本文档充当我个人的 AGENT.md，用于说明前置事项。

本人负责 StarryOS，目前总是在 QEMU 上测试。

若当前 linux 用户为 root，则你身处于本项目的 dev container 中，否则可能处于 wsl 中。

`./www` 为个人工作目录，不被 git 追踪。你可以在该目录下自由创建所要求的文件等，但是要整理好。

运行测试时，除非明确的指令，避免执行全部测试（避免浪费时间），比如：

`cargo xtask starry test qemu --target riscv64gc-unknown-none-elf`

一般指定要测试的 case：

`cargo xtask starry test qemu --target aarch64-unknown-none-softfloat -c syscall`

优先测试 `riscv64` 架构；一般进行单核测试。除非明确要求，不要测试多个架构。

StarryOS 详细文档见：`docs/docs/development/starryos.md` ，除了 StarryOS 的实现概况外，还涉及 rootfs 构建/挂载，进入 StarryOS 等关键测试操作。

任何 StarryOS 测试都要先经过 Linux 验证，保证测试是正确且与 Linux 对齐的。


当前工作：

阅读 `www/ind.md`，当前处于方向二，工作目录为 `www/d2`，工作基于 `www/d2/nginx-test-plan.md` 展开。pr 样例在 `www/pr-examples` 中。

