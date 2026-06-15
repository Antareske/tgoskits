use std::{env, process};

use act_infer::{
    cli::{parse_common_args, print_common_usage, write_json_if_requested},
    infer_ort::run_model_timed,
    preprocess::{
        denormalize_action, normalize_state, preprocess_image_file, read_state_file, read_stats,
    },
    schema::ReviewOutput,
};
use anyhow::Result;

fn main() {
    if let Err(err) = run() {
        eprintln!("ACT_INFER_FAILED: {err:#}");
        process::exit(1);
    }
}

fn run() -> Result<()> {
    let args = env::args().skip(1).collect::<Vec<_>>();
    if args.is_empty() || args.len() % 2 != 0 {
        print_common_usage("act-infer-review-ort");
        process::exit(2);
    }
    let parsed = parse_common_args(&args)?;

    let stats = read_stats(&parsed.normalize_path)?;
    let state_raw = if let Some(path) = parsed.state_path.as_ref() {
        read_state_file(path)?
    } else {
        [0.0, 0.0]
    };
    let state = normalize_state(state_raw, &stats)?;
    let image = preprocess_image_file(&parsed.image_path)?;
    // ORT review 模式输出宿主机 ONNX Runtime 的动作结果，便于和 tract 输出横向比较。
    let (raw_action, timing_ms) = run_model_timed(&parsed.model_path, &image, &state)?;
    let action_denorm = denormalize_action(&raw_action, &stats)?;

    let action_dim = stats.action.q01.len();
    let chunk_steps = action_denorm.len().checked_div(action_dim).unwrap_or(0);
    let left_wheel = action_denorm.first().copied().unwrap_or_default();
    let right_wheel = action_denorm.get(1).copied().unwrap_or_default();

    let result = ReviewOutput {
        mode: "review-ort",
        model_path: parsed.model_path.display().to_string(),
        image_path: parsed.image_path.display().to_string(),
        normalize_path: parsed.normalize_path.display().to_string(),
        state_path: parsed.state_path.as_ref().map(|p| p.display().to_string()),
        action_dim,
        chunk_steps,
        left_wheel,
        right_wheel,
        speed_diff: right_wheel - left_wheel,
        output_action_norm: raw_action,
        output_action_denorm: action_denorm,
        timing_ms,
    };

    let json = serde_json::to_string_pretty(&result)?;
    println!("ACT_REVIEW_RESULT");
    println!("{json}");
    write_json_if_requested(parsed.output_path.as_deref(), &json)?;
    Ok(())
}
