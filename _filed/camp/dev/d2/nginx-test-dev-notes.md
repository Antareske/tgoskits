# Nginx 测例开发须知（D2）

## 目标与范围

- Nginx 测试以 StarryOS 场景为主，优先在 QEMU 中迭代。
- 目标是建立可迭代、可复现、可回归的测试体系，不追求一次性“全通过”。
- 任何 StarryOS 结论都应尽量有 Linux 对照，避免误判。

## 目录与管理约束

- 当前 Nginx 测试统一在 `apps/starry/nginx` 维护。
- 采用四类目录：
  - `smoke/`：唯一接入 tgoskits 全局测试入口的 nginx 测试。
  - `phase/`：按阶段单元维护功能测试，命名采用 `x-x`，例如 `nginx-1-3-lifecycle-tests.sh`。
  - `stress/`：单独管理并发/压力测试，不与阶段功能测试混管。
  - `debug/`：单问题定位与最小复现脚本，允许灵活实验。
- 不在 `test-suit/starryos` 新增或接入 nginx phase/stress/debug 测例。

## 运行策略

- 默认避免全量测试，优先跑指定 case 或单阶段脚本。
- 优先 `riscv64`，一般先做单核路径验证。
- 常用入口：
  - `cargo xtask starry app run -t nginx --arch riscv64`
  - guest 内执行：`/usr/bin/nginx-smoke-tests.sh`
- 本地迭代可用：`apps/starry/nginx/nginx-cli-tests.sh`（smoke/phase 子命令）。

## 阶段测试规则

- 以 www/d2/nginx-test-plan.md 和 www/d2/nginx-test-tracker.md 为准推进计划；及时更新后者（勾选项和备注等）。
- phase 脚本以阶段单元负责该阶段“全部测试项”，即使和 smoke 有重叠也要在 phase 内显式覆盖。
- 阶段测试关注功能正确性与语义一致性；压力测试从阶段中剥离。
- 发现阻塞点时优先记录为 known issue 并保留探针，不让整轮测试长期卡住。

## 防阻塞要求

- 涉及网络请求、`nc`、控制命令（如 `nginx -s quit`）必须设置短超时。
- 建议脚本内统一超时封装并加入 watchdog 超时兜底。
- 失败后应强制清理 nginx 进程，避免污染后续步骤。

## 问题定位建议

- 在阶段测试中发现的无法定位的问题，在 debug 中进行最小单元测试以发现问题。
- 可以修改内核以打印 debug 信息定位问题，但是测试通过后需要将其恢复。

## 提交与协作约束

- 与 nginx 无关的工作不要混入 nginx 分支提交。
- www/d2 中文档记得跟进。
- www 目录不要跟踪、提交；保持 www 外的提交和文档对 www 内容和文档无感知。
- 保持提交粒度清晰：结构整理、脚本调整、文档更新可分层提交。
- 更新测试结构后同步 `apps/starry/nginx/README.md` 与 `apps/starry/README.md`。
