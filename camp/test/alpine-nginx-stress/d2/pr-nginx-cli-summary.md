# PR 总结：统一 Nginx CLI 测试入口并重构目录

## 背景

本次调整的目标是把 Nginx 测试从“分散脚本 + 部分 test-suit case 试验接入”收敛为**apps 侧统一 CLI 工作流**，降低维护成本并避免非 smoke 测试误接入全局测试入口。

另外，这里也一并完成了历史资产归拢：此前 `normal` 下已有的两个 nginx 相关测例不再继续作为当前主测试入口，现统一归到 `apps/starry/nginx` 体系中进行维护和迭代。

## 本次改动

### 1) 统一测试结构到 `apps/starry/nginx`

新增并整理四类目录：

- `smoke/`：仅保留并接入当前 smoke 测试脚本（CI 入口）
- `phase/`：按阶段单元组织测试，命名统一为 `x-x` 风格（如 `nginx-1-3-lifecycle-tests.sh`）
- `stress/`：单独管理压力测试（当前先以说明文档归档）
- `debug/`：用于单问题定位与灵活复现实验

### 2) 统一本地开发 CLI

提供 `apps/starry/nginx/nginx-cli-tests.sh`，便于本地按 smoke / phase 子命令执行。

说明：该 CLI 主要面向测试迭代，不作为全局 CI 的直接接入点。

### 3) 明确接入边界

- 在 tgoskits 全局测试入口中，当前仅接入 nginx smoke。
- phase / stress / debug 现不接入全局测试，避免阻塞主流程与引入不稳定项。

## 阶段性推进规划

当前 nginx 测试规划分为三层推进：

1. **Smoke**：持续保持唯一接入入口，保证回归效率。
2. **Phase**：按阶段测试语义推进。
3. **Stress/Debug**：
   - stress：集中做并发与压力特性验证，不影响主回归链路；
   - debug：围绕单问题做最小复现与定位（如多 worker 卡死、异常请求路径不稳定等）。

该分层可以兼顾：CI 稳定性、阶段语义完整性、以及问题定位效率。
