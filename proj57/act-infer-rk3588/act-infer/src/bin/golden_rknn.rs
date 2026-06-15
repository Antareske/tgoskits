use std::{env, process};

use act_infer_rk3588::{
    cli::{parse_golden_args, print_golden_usage, write_json_if_requested},
    infer_rknn::run_model_timed,
    meminfo::peak_rss_kb,
    preprocess::{
        denormalize_action, normalize_state, preprocess_image_file, read_golden, read_state_file,
        read_stats,
    },
    schema::{GoldenOutput, STATE_LEN},
};
use anyhow::Result;

fn main() {
    // 入口只负责统一错误处理。
    if let Err(err) = run() {
        eprintln!("ACT_INFER_FAILED: {err:#}");
        process::exit(1);
    }
}

fn run() -> Result<()> {
    // 参数必须成对出现，否则直接打印用法并退出。
    let args = env::args().skip(1).collect::<Vec<_>>();
    if args.is_empty() || args.len() % 2 != 0 {
        print_golden_usage("act-infer-golden-rknn");
        process::exit(2);
    }
    let parsed = parse_golden_args(&args)?;
    let common = &parsed.common;

    let stats = read_stats(&common.normalize_path)?;
    let state_raw = if let Some(path) = common.state_path.as_ref() {
        read_state_file(path)?
    } else {
        [0.0_f32; STATE_LEN]
    };
    let state = normalize_state(state_raw, &stats)?;
    let image = preprocess_image_file(&common.image_path)?;

    let (raw_action, timing_ms) = run_model_timed(
        &common.model_path,
        &image,
        &state,
        common.repeat,
        common.core_mask,
    )?;
    let action_denorm = denormalize_action(&raw_action, &stats)?;

    let golden = read_golden(&parsed.golden_path)?;
    // 只比较两者共有的长度，避免基准和当前输出长度不一致时越界。
    let compare_len = action_denorm.len().min(golden.len());
    let mut max_abs_diff = 0.0_f32;
    for i in 0..compare_len {
        let diff = (action_denorm[i] - golden[i]).abs();
        if diff > max_abs_diff {
            max_abs_diff = diff;
        }
    }
    let passed = compare_len > 0 && max_abs_diff <= parsed.atol;

    let action_dim = stats.action.q01.len();
    let chunk_steps = action_denorm.len().checked_div(action_dim).unwrap_or(0);

    let result = GoldenOutput {
        mode: "golden-rknn",
        backend: "rknn-npu",
        model_path: common.model_path.display().to_string(),
        image_path: common.image_path.display().to_string(),
        normalize_path: common.normalize_path.display().to_string(),
        state_path: common.state_path.as_ref().map(|p| p.display().to_string()),
        golden_path: parsed.golden_path.display().to_string(),
        action_dim,
        chunk_steps,
        output_action_norm: raw_action,
        output_action_denorm: action_denorm,
        max_abs_diff,
        passed,
        timing_ms,
        peak_rss_kb: peak_rss_kb(),
    };

    let json = serde_json::to_string_pretty(&result)?;
    println!("ACT_GOLDEN_RESULT");
    println!("{json}");
    write_json_if_requested(common.output_path.as_deref(), &json)?;

    if !passed {
        eprintln!(
            "ACT_INFER_FAILED: golden mismatch max_abs_diff={max_abs_diff} > atol={}",
            parsed.atol
        );
        process::exit(1);
    }
    Ok(())
}
