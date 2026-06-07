use std::{env, process};

use act_infer::{
    cli::{parse_golden_args, print_golden_usage, write_json_if_requested},
    infer_ort::run_model_timed,
    preprocess::{
        denormalize_action, normalize_state, preprocess_image_file, read_golden, read_state_file,
        read_stats,
    },
    schema::GoldenOutput,
};
use anyhow::{Result, bail};

fn main() {
    if let Err(err) = run() {
        eprintln!("ACT_INFER_FAILED: {err:#}");
        process::exit(1);
    }
}

fn run() -> Result<()> {
    let args = env::args().skip(1).collect::<Vec<_>>();
    if args.is_empty() || args.len() % 2 != 0 {
        print_golden_usage("act-infer-golden-ort");
        process::exit(2);
    }
    let parsed = parse_golden_args(&args)?;

    let stats = read_stats(&parsed.common.normalize_path)?;
    let state_raw = if let Some(path) = parsed.common.state_path.as_ref() {
        read_state_file(path)?
    } else {
        [0.0, 0.0]
    };
    let state = normalize_state(state_raw, &stats)?;
    let image = preprocess_image_file(&parsed.common.image_path)?;
    // ORT golden 模式用于和 tract 路径并行校验同一 ONNX 模型的输出。
    let (raw_action, timing_ms) = run_model_timed(&parsed.common.model_path, &image, &state)?;
    let action_denorm = denormalize_action(&raw_action, &stats)?;
    let golden = read_golden(&parsed.golden_path)?;

    if action_denorm.len() != golden.len() {
        bail!(
            "action length mismatch: actual {}, expected {}",
            action_denorm.len(),
            golden.len()
        );
    }
    let max_abs_diff = action_denorm
        .iter()
        .zip(golden.iter())
        .map(|(a, b)| (a - b).abs())
        .fold(0.0_f32, f32::max);
    let passed = max_abs_diff <= parsed.atol;

    let action_dim = stats.action.q01.len();
    let chunk_steps = action_denorm.len().checked_div(action_dim).unwrap_or(0);
    let result = GoldenOutput {
        mode: "golden-ort",
        model_path: parsed.common.model_path.display().to_string(),
        image_path: parsed.common.image_path.display().to_string(),
        normalize_path: parsed.common.normalize_path.display().to_string(),
        state_path: parsed
            .common
            .state_path
            .as_ref()
            .map(|p| p.display().to_string()),
        golden_path: parsed.golden_path.display().to_string(),
        action_dim,
        chunk_steps,
        output_action_norm: raw_action,
        output_action_denorm: action_denorm,
        max_abs_diff,
        passed,
        timing_ms,
    };

    let json = serde_json::to_string_pretty(&result)?;
    println!("ACT_GOLDEN_RESULT");
    println!("{json}");
    write_json_if_requested(parsed.common.output_path.as_deref(), &json)?;
    if !passed {
        bail!(
            "golden compare failed: max_abs_diff {} > atol {}",
            max_abs_diff,
            parsed.atol
        );
    }
    Ok(())
}
