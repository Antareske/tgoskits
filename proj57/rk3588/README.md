# proj57/rk3588 — ACT 模型 RK3588 NPU 用户态推理（任务二）

任务二（香橙派 RK3588 / NPU）的用户态推理交付。在 PC 上把赛题统一 ACT 模型
（PyTorch checkpoint）转换为 RKNN 格式，在 RK3588 上用 NPU（RKNPU2）执行
「图像预处理 → NPU 推理 → 动作后处理」完整流水线，输出一次左/右转决策，
供比赛依据模型行为判定。

本目录只负责**用户态推理程序与资产准备**，完成到可直接交付板测的程度；
板测本身（在真实 RK3588 上运行）由后续流程执行。

---

## 1. 结论速览（交付要点）

| 项目 | 值 |
|------|-----|
| 推理后端 | RK3588 NPU，RKNPU2 runtime（`librknnrt.so` 2.4.2a2） |
| 转换工具 | rknn-toolkit2 2.4.2a7（ONNX → RKNN） |
| 量化策略 | **FP16，非量化**（板内存充足，优先保证决策方向正确） |
| 推理程序语言 | Rust（FFI 直连 `librknnrt.so` C API） |
| 可执行文件大小 | golden ≈ 824 KB，review ≈ 784 KB（aarch64，已 strip） |
| 运行时依赖库 | `librknnrt.so` ≈ 7.6 MB |
| 模型大小 | `model.rknn` ≈ 97 MB（FP16）；源 `model.pt`/`model.onnx` ≈ 194 MB |
| 部署目录总大小 | ≈ 106 MB |
| ONNX↔RKNN 一致性 | 反归一化动作 `max_abs_diff ≈ 6.7e-5`，方向完全一致 |
| 左转标志（默认输入/`review_left.jpg`） | 首步 `[left,right]=(-0.00160, 0.00723)`，`right>left ⇒ 左转` |
| 右转标志（`review_right.jpg`） | 首步 `[left,right]=(0.00617, -0.00049)`，`left>right ⇒ 右转` |
| 运行时内存占用 | 程序内 `peak_rss_kb`（`/proc/self/status` VmHWM）+ 外部采样脚本，**待板测实测** |
| 执行推理时长 | 程序内 `timing_ms`（`model_load_ms` / `infer_single_ms` / `infer_total_ms`），**待板测实测** |

> 推理时长与内存占用的数值需在真实板上由 NPU 实跑得到。本交付已把测量逻辑内置进
> 程序输出 JSON，并附独立采样脚本，板测时无需额外工具即可读取。

---

## 2. 目录结构

```
rk3588/
├── act-infer/                     # Rust 推理程序
│   ├── build.rs                   # 链接 librknnrt.so，设置 $ORIGIN/lib rpath
│   ├── src/
│   │   ├── rknn_sys.rs            # RKNPU2 C API 最小 FFI 绑定
│   │   ├── infer_rknn.rs          # 加载 .rknn、设输入、rknn_run、取输出、计时
│   │   ├── preprocess.rs          # 图像 resize+归一化、状态归一化、动作反归一化
│   │   ├── schema.rs              # 输入/输出/计时 JSON 结构
│   │   ├── cli.rs                 # 绝对路径参数、--repeat、--core-mask
│   │   ├── meminfo.rs             # 进程内峰值内存读取（VmHWM 多级回退）
│   │   └── bin/{golden,review}_rknn.rs
│   └── .cargo/config.toml         # aarch64-unknown-linux-gnu 交叉链接器
├── assets/
│   ├── prepare/                   # 运行时资产 + 转换脚本
│   │   ├── export_onnx.py         # checkpoint → 确定性 2 输入 ONNX
│   │   ├── convert_rknn.py        # ONNX → RK3588 .rknn（FP16）
│   │   ├── verify_parity.py       # ONNX vs RKNN(模拟器) 一致性 + 生成 golden.json
│   │   ├── model.rknn             # 转换产物（FP16，git 忽略，脚本可复现）
│   │   ├── stats.json             # QUANTILE 归一化统计量
│   │   ├── golden.json            # RKNN 模拟器反归一化动作（板测对照基准）
│   │   ├── input.jpg / input_state.bin
│   │   └── review_left.jpg / review_right.jpg
│   ├── sdk/aarch64/librknnrt.so   # RKNPU2 runtime（来自 2.4.2a7 SDK）
│   └── sdk/include/rknn_api.h     # 对应头文件（FFI 对齐参考）
├── board/                         # 板测配置（与 apps/starry/*-rknn 同形）
│   ├── init.sh
│   ├── board-orangepi-5-plus.toml         # review（左转方向判定）
│   └── board-orangepi-5-plus-golden.toml  # golden（与基准对照）
├── scripts/
│   ├── prepare-model.sh           # 一键：venv→下载→导出→转换→校验
│   ├── build-rk3588.sh            # 交叉编译 + 打包 install 目录
│   ├── deploy-to-board.sh         # rsync install 目录到板 rootfs
│   ├── on-board-run-golden.sh     # 板上 golden 启动器
│   ├── on-board-run-review.sh     # 板上 review 启动器（左/右）
│   └── on-board-mem-sample.sh     # 外部内存峰值采样（多级回退）
├── image-build/                   # StarryOS 整盘镜像合成（脚本入仓，大二进制 git 忽略）
│   ├── README.md                  # 镜像合成与换新工具说明
│   ├── *.sh                       # build-image / make-dtb / repack-fit / swap-*
│   ├── boot.cmd / boot.scr / starry.its
│   └── output/                    # 生成的镜像与日志（git 忽略）
├── rknn-sdk2/                     # RKNPU2 SDK 包与版本说明（wheel/tar git 忽略）
├── docs/                          # RK3588 阶段文档与报告
│   ├── rk3588-env-prepare.md
│   ├── rk3588-starryos-build-manual.md
│   ├── rk3588-debug-workflow.md
│   ├── act-infer-report.md
│   ├── share-rk3588-act-starry.md
│   └── serial-scripts.md
└── README.md                      # 本文
```

---

## 3. 推理流水线设计

### 3.1 模型导出（PyTorch → ONNX）

`export_onnx.py` 复用 QEMU 阶段（任务三）已验证的导出逻辑：

- 用 `ExportWrapper` 把 ACT 的视觉编码器、状态编码器、Transformer encoder/decoder、
  action head 串成确定性前向；
- **CVAE latent 固定为 checkpoint 的 `inference_latent_mu`** 并折叠成常量，
  导出图只接收 `image`、`state` 两个输入，输出 `action`，保证推理确定可复现；
- 输入约定与赛题一致：`image=[1,3,224,224]` NCHW，`state=[1,2]`；输出
  `action=[1,8,3]`（chunk=8，action_dim=3）。

### 3.2 模型转换（ONNX → RKNN）

`convert_rknn.py` 使用 rknn-toolkit2：

- `target_platform=rk3588`；
- **不量化（FP16）**。理由：RK3588 有 8GB 内存，FP16 数值最接近原模型，
  左/右转方向最稳妥；ACT 含 CVAE + Transformer，INT8 易累积偏差甚至翻转符号。
  脚本保留 `--quantize --dataset` 入口，需要时可走 INT8；
- **RKNN 内部归一化设为恒等**（`mean=0,std=1`）。因为 Rust 端已经做完
  ImageNet 归一化并以 float32 NCHW 喂入，NPU 不能二次归一化，从而与 QEMU
  阶段、校准脚本的预处理保持逐位一致的语义。

### 3.3 运行时（Rust + RKNPU2）

`infer_rknn.rs` 通过 `rknn_sys.rs` 的 FFI 直接调用 `librknnrt.so`：

1. `rknn_init` 加载 `.rknn`（计 `model_load_ms`）；
2. `rknn_query` 取 SDK/驱动版本、输入/输出张量属性；
3. **按元素个数把缓冲映射到模型输入下标**（图像 = `3*224*224`，状态 = `2`），
   对导出图输入顺序变化具有鲁棒性；
4. `rknn_set_core_mask` 选择 NPU 核（`auto` 或三核 `012`）；
5. `rknn_inputs_set` → `rknn_run` → `rknn_outputs_get`（计 `infer_single/total_ms`，
   支持 `--repeat N` 取稳定均值）；
6. 反归一化得到真实动作，输出结构化 JSON。

预处理（`preprocess.rs`）与任务三完全一致：`Resize(224,224, Triangle)` →
`/255` → `Normalize(ImageNet mean/std)` → NCHW float32；状态
`2*(x-q01)/(q99-q01)-1`；动作 `(a+1)/2*(q99-q01)+q01`。

### 3.4 左/右转判定

差速小车约定：`speed_diff = right_wheel - left_wheel`。
`speed_diff > 0 ⇒ 左转`，`< 0 ⇒ 右转`。review 程序直接打印
`ACT_REVIEW_DIRECTION=<left|right|straight>`，板测据此判定模型行为。

---

## 4. 实现过程中的关键决断

1. **沿用任务三的导出与预处理而非重写**：保证 RK3588 输出可直接与 QEMU 阶段
   golden 对照，降低「方向反转」风险。校验显示反归一化 `max_abs_diff≈6.7e-5`。
2. **FP16 非量化**（已与你确认）：板内存充足，优先正确性；保留 INT8 入口备用。
3. **Rust + 直接 FFI，而非走 rknn C++ demo**：满足「优先 Rust」诉求，二进制小
   （<1 MB）、依赖少（仅 `librknnrt.so`）、错误处理清晰；FFI 结构体严格对齐
   SDK 头文件 `rknn_api.h`。
4. **glibc（gnu）而非 musl 目标**：`librknnrt.so` 依赖 `libc.so.6/libstdc++.so.6`
   等 glibc 符号，必须用 `aarch64-unknown-linux-gnu`；这与现有
   `apps/starry/orangepi-5-plus-uvc-rknn` 在 StarryOS 上跑 RKNN 的方式一致。
5. **RKNN 内部归一化恒等**：避免与 Rust 端预处理重复归一化导致数值/方向漂移。
6. **输入按元素数映射**：对 ONNX 导出后 image/state 顺序不敏感，转换链更稳。
7. **内存测量内置 + 外部回退**：考虑 StarryOS procfs 可能不完整，程序内
   `VmHWM→VmRSS→VmPeak→VmSize` 多级回退，并提供独立采样脚本。

---

## 5. 复现：从零产出资产

> 全程在项目内 venv 中执行，不改动全局 Python 环境。

```bash
cd proj57/rk3588

# 一键：venv + 依赖 + 下载 model.pt + 导出 ONNX + 转 RKNN + 校验 + 生成 golden.json
bash scripts/prepare-model.sh
```

依赖（`prepare-model.sh` 自动装入 `.venv`）：

- `torch==2.4.0+cpu`、`torchvision==0.19.0+cpu`
- `onnx==1.17.0`（注意：rknn-toolkit2 2.4.2 依赖 `onnx.mapping`，必须 ≤1.17，
  不能用 1.18+）、`onnxruntime`、`numpy<=1.26.4`、`Pillow`、`setuptools<81`
- `rknn_toolkit2-2.4.2a7`（cp312 wheel，来自 `proj57/rk3588/rknn-sdk2/packages/`）

单步等价命令见 `scripts/prepare-model.sh` 内注释。

---

## 6. 构建与部署

### 6.1 交叉编译 + 打包

```bash
# 需要 gcc-aarch64-linux-gnu 和 rust 目标 aarch64-unknown-linux-gnu
rustup target add aarch64-unknown-linux-gnu
sudo apt-get install -y gcc-aarch64-linux-gnu

bash scripts/build-rk3588.sh
```

产物：`install/rk3588_linux_aarch64/act_infer_rk3588/`，自带二进制、
`lib/librknnrt.so`、`model/`、板上启动器。二进制以 `$ORIGIN/lib` 为 rpath，
运行时自动找到 `librknnrt.so`。

### 6.2 部署到板 rootfs

StarryOS（RK3588）与板上 Linux 共用 rootfs。把 install 目录放到
`/act_infer_rk3588`：

```bash
BOARD_IP=10.3.10.24 BOARD_USER=orangepi bash scripts/deploy-to-board.sh
```

部署后板上应存在：

```
/act_infer_rk3588/act-infer-golden-rknn
/act_infer_rk3588/act-infer-review-rknn
/act_infer_rk3588/lib/librknnrt.so
/act_infer_rk3588/model/model.rknn
/act_infer_rk3588/model/{stats.json,golden.json,input.jpg,input_state.bin,review_left.jpg,review_right.jpg}
/act_infer_rk3588/run-golden.sh
/act_infer_rk3588/run-review.sh
```

---

## 7. 执行说明

### 7.1 板上 Linux 烟雾测试（部署后先验证驱动可用）

```bash
ssh orangepi@$BOARD_IP 'sudo /act_infer_rk3588/run-review.sh left'
# 期望末尾: ACT_REVIEW_DIRECTION=left 与 ACT_REVIEW_DONE
```

### 7.2 StarryOS 板测

把 `board/` 下配置接入 Starry 板测流程（与 `apps/starry/*-rknn` 同形）：

- review（方向判定）：`board/board-orangepi-5-plus.toml`，成功正则
  `^ACT_REVIEW_DIRECTION=left$`；
- golden（与基准对照）：`board/board-orangepi-5-plus-golden.toml`，成功正则
  `^ACT_INFER_OK$`。

`shell_init_cmd` 调用 `/act_infer_rk3588/run-review.sh`（或 `run-golden.sh`），
两脚本会自动设置 `LD_LIBRARY_PATH` 并以绝对路径传参。

### 7.3 直接调用二进制

```bash
/act_infer_rk3588/act-infer-review-rknn \
  --model /act_infer_rk3588/model/model.rknn \
  --image /act_infer_rk3588/model/review_left.jpg \
  --normalize /act_infer_rk3588/model/stats.json \
  --state /act_infer_rk3588/model/input_state.bin \
  --repeat 10 --core-mask auto \
  --output /tmp/act_review_result.json
```

参数：`--repeat N` 多次推理取稳定计时；`--core-mask auto|012` 选 NPU 核；
golden 额外支持 `--golden <json> --atol <float>`。

### 7.4 输出 JSON 字段（review）

```jsonc
{
  "backend": "rknn-npu",
  "left_wheel": -0.00160, "right_wheel": 0.00723,
  "speed_diff": 0.00883, "direction": "left",
  "output_action_norm": [...], "output_action_denorm": [...],
  "timing_ms": { "run_count": 10, "infer_single_ms": ?, "infer_total_ms": ?, "model_load_ms": ? },
  "peak_rss_kb": ?     // 程序内读取；板测实测
}
```

---

## 8. 运行时内存占用 / 推理时长测量方案

赛题文档要求记录「运行时内存占用」与「执行推理时长」。本交付提供两条互补路径，
**数值需由你在 StarryOS 上实测**（不保证 Starry 现有内存监控工具可用）：

1. **程序内（首选，无需外部工具）**：
   - 时长：`timing_ms`，区分一次性加载（`model_load_ms`）与纯推理
     （`infer_single_ms` = N 次均值，`infer_total_ms`）。用 `--repeat` 提稳。
   - 内存：`peak_rss_kb`，从 `/proc/self/status` 读 `VmHWM`，并按
     `VmRSS→VmPeak→VmSize` 多级回退，兼容 procfs 字段不全的内核。
2. **外部采样（兜底）**：`scripts/on-board-mem-sample.sh` 在进程运行期间轮询
   RSS 取峰值，依次尝试 `/proc/<pid>/status`、`/proc/<pid>/statm`、`ps`：
   ```bash
   /act_infer_rk3588/.../on-board-mem-sample.sh /act_infer_rk3588/run-review.sh left
   # 末尾打印 ACT_PEAK_RSS_KB=<peak>
   ```

> 若 StarryOS 两条路径都拿不到内存，请在板测时记录现象，我再据此补一个
> 与 Starry 内存子系统对接的方案。

---

## 9. 验证状态

- ✅ 模型已实际转换：`assets/prepare/model.rknn`（FP16，rk3588）。
- ✅ ONNX↔RKNN 一致性（PC 模拟器）：默认输入 `max_abs_diff≈6.7e-5`，方向 left；
  `review_right.jpg` 方向 right、`review_left.jpg` 方向 left，与 ONNX 完全一致。
- ✅ 交叉编译通过，`cargo clippy` 无告警，`cargo fmt` 已应用。
- ✅ aarch64 二进制在 qemu-user 下正确链接 `librknnrt.so` 并进入 `rknn_init`；
  因仿真无 rknpu 内核驱动而按预期报错退出（NPU 实跑在真实板）。
- ⏳ 待板测：真实 NPU 推理输出、`timing_ms`、`peak_rss_kb`。

---

## 10. AI 使用说明

- 本目录的工程脚手架、Rust 代码、转换/校验脚本、文档由 AI 编码助手在本人
  指导与决策下生成与调试；模型转换、ONNX/RKNN 一致性校验、交叉编译、qemu-user
  冒烟均由助手在本环境实际执行并验证。
- **借鉴**：
  - 模型导出与预处理逻辑沿用本仓库任务三 `proj57/qemu`
    （`export_onnx.py` / 预处理 / golden 流程）。
  - RK3588 上 RKNN 的交叉编译与部署形态参考本仓库
    `apps/starry/orangepi-5-plus-uvc-rknn`（aarch64-gnu + `librknnrt.so` +
    `$ORIGIN/lib` rpath + rootfs 安装）。
  - RKNPU2 C API 来自 Rockchip SDK 头文件 `rknn_api.h`（FFI 严格对齐）。
- **人工决策**：量化策略（FP16 非量化）、目标三元组（gnu）、左右转判定约定、
  交付边界（仅用户态推理与资产、不含板测执行）。
- **AI 参与**：上述实现、调试（如 `onnx.mapping` 版本兼容、RKNN 模拟器
  NCHW data_format、`pkg_resources` 依赖）、文档撰写。
