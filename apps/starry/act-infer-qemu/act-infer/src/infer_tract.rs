use std::{path::Path, time::Instant};

use anyhow::{Context, Result, bail};
use tract_onnx::prelude::*;

use crate::schema::{
    MODEL_INPUT_IMAGE_SHAPE_4D, MODEL_INPUT_IMAGE_SHAPE_5D, MODEL_INPUT_LATENT_SHAPE,
    MODEL_INPUT_STATE_SHAPE, TimingMetrics,
};

type Plan = RunnableModel<TypedFact, Box<dyn TypedOp>, Graph<TypedFact, Box<dyn TypedOp>>>;

pub fn run_model_timed(
    model_path: &Path,
    image: &[f32],
    state: &[f32],
) -> Result<(Vec<f32>, TimingMetrics)> {
    // tract 路径保留原有兼容逻辑：支持导出模型可能存在的 2 输入或 3 输入形式。
    let model = tract_onnx::onnx().model_for_path(model_path)?;
    let input_count = model.input_outlets()?.len();
    if input_count != 2 && input_count != 3 {
        bail!("unsupported input count {input_count}, expected 2 or 3");
    }

    let mut model = model.with_input_fact(1, f32::fact(MODEL_INPUT_STATE_SHAPE).into())?;
    if input_count == 3 {
        model = model.with_input_fact(2, f32::fact(MODEL_INPUT_LATENT_SHAPE).into())?;
    }
    let model = model.into_optimized()?.into_runnable()?;

    let state_tensor = Tensor::from_shape(&MODEL_INPUT_STATE_SHAPE, state)?;
    // 旧导出模型可能保留 latent 输入；当前推理固定填 0，实际新模型通常只有 image/state。
    let latent = [0.0_f32; 32];
    let latent_tensor = Tensor::from_shape(&MODEL_INPUT_LATENT_SHAPE, &latent)?;

    let start = Instant::now();
    let outputs =
        run_with_adaptive_image_shape(&model, input_count, image, &state_tensor, &latent_tensor)
            .context("failed to run model with 4D/5D image input layouts")?;
    let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;

    let raw_action = outputs[0].to_array_view::<f32>()?;
    let action = raw_action
        .as_slice()
        .map(|s| s.to_vec())
        .context("ACT output is not contiguous")?;

    Ok((
        action,
        TimingMetrics {
            run_count: 1,
            infer_single_ms: elapsed_ms,
            infer_total_ms: elapsed_ms,
        },
    ))
}

fn run_with_adaptive_image_shape(
    model: &Plan,
    input_count: usize,
    image: &[f32],
    state: &Tensor,
    latent: &Tensor,
) -> TractResult<TVec<TValue>> {
    // 优先按当前导出约定使用 4D 图像输入；失败时回退到旧模型可能使用的 5D 布局。
    let image_4d = Tensor::from_shape(&MODEL_INPUT_IMAGE_SHAPE_4D, image)?;
    let first = run_once(model, input_count, image_4d, state, latent);
    if first.is_ok() {
        return first;
    }

    let image_5d = Tensor::from_shape(&MODEL_INPUT_IMAGE_SHAPE_5D, image)?;
    run_once(model, input_count, image_5d, state, latent)
}

fn run_once(
    model: &Plan,
    input_count: usize,
    image: Tensor,
    state: &Tensor,
    latent: &Tensor,
) -> TractResult<TVec<TValue>> {
    if input_count == 3 {
        model.run(tvec!(
            image.into(),
            state.clone().into(),
            latent.clone().into()
        ))
    } else {
        model.run(tvec!(image.into(), state.clone().into()))
    }
}
