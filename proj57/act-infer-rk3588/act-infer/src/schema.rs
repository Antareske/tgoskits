use serde::{Deserialize, Serialize};

pub const IMAGE_LEN: usize = 3 * 224 * 224;
pub const STATE_LEN: usize = 2;
pub const IMAGE_H: usize = 224;
pub const IMAGE_W: usize = 224;

#[derive(Debug, Deserialize)]
pub struct StatsFile {
    #[serde(rename = "observation.state")]
    pub observation_state: Option<QuantileStats>,
    pub action: QuantileStats,
}

#[derive(Debug, Deserialize)]
pub struct QuantileStats {
    pub q01: Vec<f32>,
    pub q99: Vec<f32>,
}

#[derive(Debug, Deserialize)]
pub struct GoldenFile {
    pub action_denorm: Vec<f32>,
}

#[derive(Debug, Serialize)]
pub struct TimingMetrics {
    /// Number of inference runs that were timed.
    pub run_count: usize,
    /// Average time of a single `rknn_run` + output fetch, in milliseconds.
    pub infer_single_ms: f64,
    /// Total inference time across all timed runs, in milliseconds.
    pub infer_total_ms: f64,
    /// One-time model load + context init time, in milliseconds.
    pub model_load_ms: f64,
}

#[derive(Debug, Serialize)]
pub struct GoldenOutput {
    pub mode: &'static str,
    pub backend: &'static str,
    pub model_path: String,
    pub image_path: String,
    pub normalize_path: String,
    pub state_path: Option<String>,
    pub golden_path: String,
    pub action_dim: usize,
    pub chunk_steps: usize,
    pub output_action_norm: Vec<f32>,
    pub output_action_denorm: Vec<f32>,
    pub max_abs_diff: f32,
    pub passed: bool,
    pub timing_ms: TimingMetrics,
    pub peak_rss_kb: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct ReviewOutput {
    pub mode: &'static str,
    pub backend: &'static str,
    pub model_path: String,
    pub image_path: String,
    pub normalize_path: String,
    pub state_path: Option<String>,
    pub action_dim: usize,
    pub chunk_steps: usize,
    pub left_wheel: f32,
    pub right_wheel: f32,
    /// right_wheel - left_wheel. Negative => turning left, positive => right.
    pub speed_diff: f32,
    pub direction: &'static str,
    pub output_action_norm: Vec<f32>,
    pub output_action_denorm: Vec<f32>,
    pub timing_ms: TimingMetrics,
    pub peak_rss_kb: Option<u64>,
}
