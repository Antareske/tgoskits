//! act-infer-rk3588：在 RK3588 NPU 上运行 ACT 模型推理的库 crate。
//!
//! 各模块职责：
//! - `cli`：命令行参数解析与校验。
//! - `infer_rknn`：通过 RKNPU2 运行时执行实际推理。
//! - `meminfo`：尽力而为地采集进程内存占用。
//! - `preprocess`：图像/状态预处理与动作反归一化。
//! - `rknn_sys`：RKNPU2 运行时（librknnrt.so）的 FFI 绑定。
//! - `schema`：输入输出的数据结构与常量定义。

pub mod cli;
pub mod infer_rknn;
pub mod meminfo;
pub mod preprocess;
pub mod rknn_sys;
pub mod schema;
