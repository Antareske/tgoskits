use std::{path::Path, time::Instant};

use anyhow::Result;
use ort::{
    inputs,
    session::{Session, builder::GraphOptimizationLevel},
    value::TensorRef,
};

use crate::schema::{MODEL_INPUT_IMAGE_SHAPE_4D, MODEL_INPUT_STATE_SHAPE, TimingMetrics};

pub fn run_model_timed(
    model_path: &Path,
    image: &[f32],
    state: &[f32],
) -> Result<(Vec<f32>, TimingMetrics)> {
    // ORT 路径用于宿主机对照验证，只统计一次完整 session 推理耗时。
    let start = Instant::now();
    let action = run_model(model_path, image, state)?;
    let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;

    Ok((
        action,
        TimingMetrics {
            run_count: 1,
            infer_single_ms: elapsed_ms,
            infer_total_ms: elapsed_ms,
        },
    ))
}

fn run_model(model_path: &Path, image: &[f32], state: &[f32]) -> Result<Vec<f32>> {
    // 多次初始化时 ORT 会复用全局环境；这里忽略重复初始化返回值。
    let _ = ort::init().with_name("act-infer-ort").commit();
    let mut session = Session::builder()
        .map_err(|err| anyhow::anyhow!("failed to create ONNX Runtime session builder: {err:?}"))?
        .with_optimization_level(GraphOptimizationLevel::Level1)
        .map_err(|err| anyhow::anyhow!("failed to set ONNX Runtime optimization level: {err:?}"))?
        .with_intra_threads(1)
        .map_err(|err| anyhow::anyhow!("failed to set ONNX Runtime thread count: {err:?}"))?
        .commit_from_file(model_path)
        .map_err(|err| {
            anyhow::anyhow!(
                "failed to load ONNX model {}: {err:?}",
                model_path.display()
            )
        })?;

    // 新导出模型固定为两个输入：归一化后的图像和机器人状态。
    let outputs = session
        .run(inputs![
            TensorRef::from_array_view((MODEL_INPUT_IMAGE_SHAPE_4D, image))
                .map_err(|err| anyhow::anyhow!("failed to build image tensor: {err:?}"))?,
            TensorRef::from_array_view((MODEL_INPUT_STATE_SHAPE, state))
                .map_err(|err| anyhow::anyhow!("failed to build state tensor: {err:?}"))?,
        ])
        .map_err(|err| anyhow::anyhow!("failed to run ONNX Runtime session: {err:?}"))?;

    let action = outputs[0].try_extract_tensor::<f32>()?.1.to_vec();
    Ok(action)
}
