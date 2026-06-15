use std::{env, process, time::Instant};

use act_infer_rk3588::{
    cli::{parse_common_args, print_common_usage, write_json_if_requested},
    infer_rknn::run_model_timed,
    meminfo::peak_rss_kb,
    preprocess::{
        denormalize_action, normalize_state, preprocess_image_file, read_state_file, read_stats,
    },
    schema::{ReviewOutput, STATE_LEN},
};
use anyhow::Result;

fn main() {
    // 入口只负责打印统一错误并返回退出码。
    if let Err(err) = run() {
        eprintln!("ACT_INFER_FAILED: {err:#}");
        process::exit(1);
    }
}

fn run() -> Result<()> {
    // 参数必须成对出现，否则直接打印用法并退出。
    let args = env::args().skip(1).collect::<Vec<_>>();
    if args.is_empty() || args.len() % 2 != 0 {
        print_common_usage("act-infer-review-rknn");
        process::exit(2);
    }
    let parsed = parse_common_args(&args)?;

    let stats = read_stats(&parsed.normalize_path)?;
    let state_raw = if let Some(path) = parsed.state_path.as_ref() {
        read_state_file(path)?
    } else {
        [0.0_f32; STATE_LEN]
    };
    let normalize_state_start = Instant::now();
    let state = normalize_state(state_raw, &stats)?;
    let normalize_state_ms = normalize_state_start.elapsed().as_secs_f64() * 1000.0;

    let preprocess_start = Instant::now();
    let image = preprocess_image_file(&parsed.image_path)?;
    let preprocess_ms = preprocess_start.elapsed().as_secs_f64() * 1000.0;

    // review 模式只输出动作结果，供人工检查左右轮速度趋势和转向方向。
    let (raw_action, mut timing_ms) = run_model_timed(
        &parsed.model_path,
        &image,
        &state,
        parsed.repeat,
        parsed.core_mask,
    )?;

    let denormalize_start = Instant::now();
    let action_denorm = denormalize_action(&raw_action, &stats)?;
    let denormalize_ms = denormalize_start.elapsed().as_secs_f64() * 1000.0;

    // 回填 bin 层测量的一次性 CPU 阶段耗时。
    timing_ms.preprocess_ms = preprocess_ms;
    timing_ms.normalize_state_ms = normalize_state_ms;
    timing_ms.denormalize_ms = denormalize_ms;

    let action_dim = stats.action.q01.len();
    let chunk_steps = action_denorm.len().checked_div(action_dim).unwrap_or(0);
    let left_wheel = action_denorm.first().copied().unwrap_or_default();
    let right_wheel = action_denorm.get(1).copied().unwrap_or_default();
    let speed_diff = right_wheel - left_wheel;
    // 差速驱动约定：右轮比左轮更快，车辆会向左转。
    let direction = if speed_diff > 0.0 {
        "left"
    } else if speed_diff < 0.0 {
        "right"
    } else {
        "straight"
    };

    let result = ReviewOutput {
        mode: "review-rknn",
        backend: "rknn-npu",
        model_path: parsed.model_path.display().to_string(),
        image_path: parsed.image_path.display().to_string(),
        normalize_path: parsed.normalize_path.display().to_string(),
        state_path: parsed.state_path.as_ref().map(|p| p.display().to_string()),
        action_dim,
        chunk_steps,
        left_wheel,
        right_wheel,
        speed_diff,
        direction,
        output_action_norm: raw_action,
        output_action_denorm: action_denorm,
        timing_ms,
        peak_rss_kb: peak_rss_kb(),
    };

    let json = serde_json::to_string_pretty(&result)?;
    println!("ACT_REVIEW_RESULT");
    println!("{json}");
    println!("ACT_REVIEW_DIRECTION={direction}");
    write_json_if_requested(parsed.output_path.as_deref(), &json)?;
    Ok(())
}
