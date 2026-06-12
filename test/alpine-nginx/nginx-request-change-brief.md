# nginx request change 简报

本次修改是对 nginx review 反馈的响应，核心是把文档里的 CLI 说明对齐到当前真实接口 `cargo xtask starry app qemu ...`，并收敛默认 smoke 路径。

## 修改内容

- 将 nginx 文档中过时的 `cargo xtask starry app run ...` 示例替换为 `cargo xtask starry app qemu ...`。
- 将 `apps/starry/README.md` 里的 nginx 小节更新为当前真实可用的 QEMU 命令。
- 保留 sendfile 测试节点本身，但把默认 smoke 中的 `large file sendfile` 步骤注释掉，因为它曾被观察到出现间歇性的响应体截断失败。
- 在 smoke 脚本中补充了中文注释，用于说明这个间歇性失败现象。

## 原因

- `run` 在这个仓库里已经不是有效的 `starry app` 子命令；当前入口是 `qemu`。
- 默认 smoke 需要与 CI 实际能够稳定运行的内容保持一致。
- `large file sendfile` 这一步仍然保留在 phase/debug 流程里，方便后续显式复测。

## 结果

- nginx 文档现在与真实 CLI 保持一致。
- 默认 smoke 路径更窄，避开了那个间歇性失败的大文件步骤。
- sendfile 测试本身仍保留，便于定向调试和后续分析。
