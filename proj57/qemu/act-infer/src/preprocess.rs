use std::{fs, path::Path};

use anyhow::{Context, Result, bail};
use image::imageops::FilterType;

use crate::schema::{IMAGE_LEN, STATE_LEN, StatsFile};

pub fn preprocess_image_file(path: &Path) -> Result<Vec<f32>> {
    let img = image::open(path)
        .with_context(|| format!("failed to open image {}", path.display()))?
        .to_rgb8();
    let img = image::imageops::resize(&img, 224, 224, FilterType::Triangle);
    let mut out = vec![0.0_f32; IMAGE_LEN];
    let mean = [0.485_f32, 0.456_f32, 0.406_f32];
    let std = [0.229_f32, 0.224_f32, 0.225_f32];
    for y in 0..224 {
        for x in 0..224 {
            let p = img.get_pixel(x, y).0;
            for c in 0..3 {
                let idx = c * 224 * 224 + y as usize * 224 + x as usize;
                let v = (p[c] as f32) / 255.0;
                out[idx] = (v - mean[c]) / std[c];
            }
        }
    }
    Ok(out)
}

pub fn read_state_file(path: &Path) -> Result<[f32; 2]> {
    let values = read_f32_file(path, STATE_LEN)?;
    Ok([values[0], values[1]])
}

pub fn normalize_state(raw: [f32; 2], stats: &StatsFile) -> Result<Vec<f32>> {
    let Some(ob_state) = stats.observation_state.as_ref() else {
        return Ok(raw.to_vec());
    };
    if ob_state.q01.len() < STATE_LEN || ob_state.q99.len() < STATE_LEN {
        bail!("observation.state quantiles shorter than state dimension");
    }
    let mut normalized = Vec::with_capacity(STATE_LEN);
    for (idx, value) in raw.iter().enumerate() {
        let q01 = ob_state.q01[idx];
        let q99 = ob_state.q99[idx];
        let denom = if (q99 - q01).abs() < f32::EPSILON {
            1e-8
        } else {
            q99 - q01
        };
        normalized.push(2.0 * (value - q01) / denom - 1.0);
    }
    Ok(normalized)
}

pub fn denormalize_action(action: &[f32], stats: &StatsFile) -> Result<Vec<f32>> {
    if stats.action.q01.is_empty() || stats.action.q99.is_empty() {
        bail!("stats action quantiles are empty");
    }
    if stats.action.q01.len() != stats.action.q99.len() {
        bail!("stats action quantiles length mismatch");
    }
    let dim = stats.action.q01.len();
    Ok(action
        .iter()
        .enumerate()
        .map(|(idx, value)| {
            let q01 = stats.action.q01[idx % dim];
            let q99 = stats.action.q99[idx % dim];
            (value + 1.0) * 0.5 * (q99 - q01) + q01
        })
        .collect())
}

pub fn read_stats(path: &Path) -> Result<StatsFile> {
    let content =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    serde_json::from_str(&content).with_context(|| format!("failed to parse {}", path.display()))
}

pub fn read_golden(path: &Path) -> Result<Vec<f32>> {
    let content =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    let parsed: crate::schema::GoldenFile = serde_json::from_str(&content)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    Ok(parsed.action_denorm)
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
