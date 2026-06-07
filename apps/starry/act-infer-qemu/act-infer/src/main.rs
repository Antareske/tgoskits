use std::{env, fs, path::Path};

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use tract_onnx::prelude::*;

const IMAGE_LEN: usize = 3 * 224 * 224;
const STATE_LEN: usize = 2;
const DEFAULT_ASSET_DIR: &str = "/opt/act";

#[derive(Debug, Deserialize)]
struct StatsFile {
    action: QuantileStats,
}

#[derive(Debug, Deserialize)]
struct QuantileStats {
    q01: Vec<f32>,
    q99: Vec<f32>,
}

#[derive(Debug, Deserialize)]
struct GoldenFile {
    action_denorm: Vec<f32>,
}

fn main() -> Result<()> {
    let asset_dir = env::args()
        .nth(1)
        .unwrap_or_else(|| DEFAULT_ASSET_DIR.to_string());
    let asset_dir = Path::new(&asset_dir);

    let image = read_f32_file(&asset_dir.join("input_image.bin"), IMAGE_LEN)?;
    let state = read_f32_file(&asset_dir.join("input_state.bin"), STATE_LEN)?;
    let golden = read_golden(&asset_dir.join("golden.json"))?;
    let stats = read_stats(&asset_dir.join("stats.json"))?;

    let model = tract_onnx::onnx()
        .model_for_path(asset_dir.join("model.onnx"))?
        .with_input_fact(0, f32::fact([1, 1, 3, 224, 224]).into())?
        .with_input_fact(1, f32::fact([1, 2]).into())?
        .into_optimized()?
        .into_runnable()?;

    let image_tensor = Tensor::from_shape(&[1, 1, 3, 224, 224], &image)?;
    let state_tensor = Tensor::from_shape(&[1, 2], &state)?;
    let outputs = model.run(tvec!(image_tensor.into(), state_tensor.into()))?;
    let raw_action = outputs[0].to_array_view::<f32>()?;
    let raw_action = raw_action
        .as_slice()
        .context("ACT output is not contiguous")?;

    let denormalized = denormalize_action(raw_action, &stats)?;
    compare(
        "denormalized_action",
        &denormalized,
        &golden.action_denorm,
        1e-2,
    )?;

    println!("ACT_ACTION={denormalized:?}");
    Ok(())
}

fn read_f32_file(path: &Path, expected_len: usize) -> Result<Vec<f32>> {
    let bytes = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    if bytes.len() % 4 != 0 {
        bail!("{} size is not a multiple of f32", path.display());
    }
    let values = bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect::<Vec<_>>();
    if values.len() != expected_len {
        bail!(
            "{} has {} f32 values, expected {}",
            path.display(),
            values.len(),
            expected_len
        );
    }
    Ok(values)
}

fn read_stats(path: &Path) -> Result<StatsFile> {
    let content =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    serde_json::from_str(&content).with_context(|| format!("failed to parse {}", path.display()))
}

fn read_golden(path: &Path) -> Result<GoldenFile> {
    let content =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    serde_json::from_str(&content).with_context(|| format!("failed to parse {}", path.display()))
}

fn denormalize_action(action: &[f32], stats: &StatsFile) -> Result<Vec<f32>> {
    if stats.action.q01.len() < action.len() || stats.action.q99.len() < action.len() {
        bail!("stats action quantiles are shorter than action output");
    }
    Ok(action
        .iter()
        .enumerate()
        .map(|(idx, value)| {
            let q01 = stats.action.q01[idx];
            let q99 = stats.action.q99[idx];
            (value + 1.0) * 0.5 * (q99 - q01) + q01
        })
        .collect())
}

fn compare(name: &str, actual: &[f32], expected: &[f32], atol: f32) -> Result<()> {
    if actual.len() != expected.len() {
        bail!(
            "{name} length mismatch: actual {}, expected {}",
            actual.len(),
            expected.len()
        );
    }
    let max_diff = actual
        .iter()
        .zip(expected.iter())
        .map(|(left, right)| (left - right).abs())
        .fold(0.0_f32, f32::max);
    if max_diff > atol {
        bail!("{name} mismatch: max_abs_diff {max_diff} > {atol}");
    }
    Ok(())
}
