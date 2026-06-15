use serde::{Deserialize, Serialize};

pub const IMAGE_LEN: usize = 3 * 224 * 224;
pub const STATE_LEN: usize = 2;
pub const LATENT_LEN: usize = 32;
pub const MODEL_INPUT_STATE_SHAPE: [usize; 2] = [1, 2];
pub const MODEL_INPUT_IMAGE_SHAPE_4D: [usize; 4] = [1, 3, 224, 224];
pub const MODEL_INPUT_IMAGE_SHAPE_5D: [usize; 5] = [1, 1, 3, 224, 224];
pub const MODEL_INPUT_LATENT_SHAPE: [usize; 2] = [1, 32];

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
    pub run_count: usize,
    pub infer_single_ms: f64,
    pub infer_total_ms: f64,
}

#[derive(Debug, Serialize)]
pub struct GoldenOutput {
    pub mode: &'static str,
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
}

#[derive(Debug, Serialize)]
pub struct ReviewOutput {
    pub mode: &'static str,
    pub model_path: String,
    pub image_path: String,
    pub normalize_path: String,
    pub state_path: Option<String>,
    pub action_dim: usize,
    pub chunk_steps: usize,
    pub left_wheel: f32,
    pub right_wheel: f32,
    pub speed_diff: f32,
    pub output_action_norm: Vec<f32>,
    pub output_action_denorm: Vec<f32>,
    pub timing_ms: TimingMetrics,
}
