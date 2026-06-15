use serde::{Deserialize, Serialize};

/// 图像张量的元素总数：3 通道 × 224 高 × 224 宽（NCHW 布局）。
pub const IMAGE_LEN: usize = 3 * 224 * 224;
/// 状态向量长度（如左右轮速等 2 维观测量）。
pub const STATE_LEN: usize = 2;
/// 输入图像高度。
pub const IMAGE_H: usize = 224;
/// 输入图像宽度。
pub const IMAGE_W: usize = 224;

/// 归一化统计文件（stats.json）的反序列化结构。
///
/// 记录训练时统计得到的分位数，用于对状态做归一化、对动作做反归一化。
#[derive(Debug, Deserialize)]
pub struct StatsFile {
    /// 观测状态的分位数统计；为 `None` 时表示状态不做归一化。
    #[serde(rename = "observation.state")]
    pub observation_state: Option<QuantileStats>,
    /// 动作的分位数统计，用于反归一化（必有）。
    pub action: QuantileStats,
}

/// 分位数统计：1% 与 99% 分位，逐维度排列。
#[derive(Debug, Deserialize)]
pub struct QuantileStats {
    /// 1% 分位（每个维度一个值）。
    pub q01: Vec<f32>,
    /// 99% 分位（每个维度一个值）。
    pub q99: Vec<f32>,
}

/// 基准（golden）文件结构，保存离线计算好的参考动作输出，用于精度比对。
#[derive(Debug, Deserialize)]
pub struct GoldenFile {
    /// 反归一化后的参考动作向量。
    pub action_denorm: Vec<f32>,
}

/// 推理耗时指标，随结果 JSON 一起输出，便于性能评估。
#[derive(Debug, Serialize)]
pub struct TimingMetrics {
    /// 计时统计的推理次数。
    pub run_count: usize,
    /// 单次 `rknn_run` + 取输出的平均耗时（毫秒）。
    pub infer_single_ms: f64,
    /// 所有计时推理的总耗时（毫秒）。
    pub infer_total_ms: f64,
    /// 一次性的模型加载 + 上下文初始化耗时（毫秒）。
    pub model_load_ms: f64,
}

/// golden（基准比对）模式的输出结构，序列化为结果 JSON。
#[derive(Debug, Serialize)]
pub struct GoldenOutput {
    /// 运行模式标识。
    pub mode: &'static str,
    /// 推理后端标识。
    pub backend: &'static str,
    /// 模型文件路径。
    pub model_path: String,
    /// 输入图像路径。
    pub image_path: String,
    /// 归一化统计文件路径。
    pub normalize_path: String,
    /// 状态文件路径（可选）。
    pub state_path: Option<String>,
    /// 基准文件路径。
    pub golden_path: String,
    /// 动作维度。
    pub action_dim: usize,
    /// 动作分块（chunk）步数 = 输出长度 / 动作维度。
    pub chunk_steps: usize,
    /// 模型原始输出（归一化后的动作）。
    pub output_action_norm: Vec<f32>,
    /// 反归一化后的动作。
    pub output_action_denorm: Vec<f32>,
    /// 与基准的最大绝对误差。
    pub max_abs_diff: f32,
    /// 是否通过精度校验（最大误差不超过容差）。
    pub passed: bool,
    /// 耗时指标。
    pub timing_ms: TimingMetrics,
    /// 进程峰值常驻内存（KB），可能不可用。
    pub peak_rss_kb: Option<u64>,
}

/// review（人工查看）模式的输出结构，序列化为结果 JSON。
#[derive(Debug, Serialize)]
pub struct ReviewOutput {
    /// 运行模式标识。
    pub mode: &'static str,
    /// 推理后端标识。
    pub backend: &'static str,
    /// 模型文件路径。
    pub model_path: String,
    /// 输入图像路径。
    pub image_path: String,
    /// 归一化统计文件路径。
    pub normalize_path: String,
    /// 状态文件路径（可选）。
    pub state_path: Option<String>,
    /// 动作维度。
    pub action_dim: usize,
    /// 动作分块（chunk）步数。
    pub chunk_steps: usize,
    /// 左轮速度。
    pub left_wheel: f32,
    /// 右轮速度。
    pub right_wheel: f32,
    /// 右轮减左轮速度差。负值 => 左转，正值 => 右转。
    pub speed_diff: f32,
    /// 运动方向（"left"/"right"/"straight"）。
    pub direction: &'static str,
    /// 模型原始输出（归一化后的动作）。
    pub output_action_norm: Vec<f32>,
    /// 反归一化后的动作。
    pub output_action_denorm: Vec<f32>,
    /// 耗时指标。
    pub timing_ms: TimingMetrics,
    /// 进程峰值常驻内存（KB），可能不可用。
    pub peak_rss_kb: Option<u64>,
}
