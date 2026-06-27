# Apache 测例文档首版

## 背景

这是 Apache on StarryOS 的首次 PR 提交，目标是把 Apache 测例、目录结构和阶段划分完整整理出来，方便后续持续补充和定位问题。

Apache 测例借鉴了 nginx 的组织方式，但测试内容以 Apache 自身行为为准，重点覆盖：

- smoke 基础可运行性
- prefork MPM
- 静态 HTTP 行为
- 目录与权限规则
- sendfile / Range / 缓存语义
- 日志与生命周期
- CGI 和模块特性

## 本次内容

- 新增 Apache app 目录说明和运行入口说明。
- 新增 smoke、phase、debug、qemu 和 stress 的文档说明。
- 补充 `www/apache` 下的测试方案与任务跟踪。
- 为 phase20 增加问题记录，明确它只负责 prefork 启动、请求处理和 clean stop。
- 将 restart / graceful / stop 这类生命周期测试放到 phase50。

## 重点说明

- phase20 不再承担 restart 语义。
- debug 文档用于记录单点问题和复现入口。
- `all` 只是把 smoke 和各个 phase 串起来，不代表所有细节都要在一个阶段里测完。
- 所有 StarryOS 结果都以 Linux Alpine 同脚本或等价命令作为对照。

## 验证

- 重新阅读并清理了 Apache 相关 Markdown。
- 对齐了文档中 phase20 / phase50 / debug / all 的边界。

## 本地验证

- 已在本地跑通 `apache` 的 `all` 命令，覆盖四个架构：`x86_64`、`riscv64`、`aarch64`、`loongarch64`。
- `all` 会按顺序执行 smoke 和各个 phase，验证基本可用性、MPM、静态 HTTP、目录规则、日志、CGI 和模块特性。

## 备注

- 这是文档和测试组织的首版整理，后续会继续补充更细的 Apache 测例和 issue 记录。
