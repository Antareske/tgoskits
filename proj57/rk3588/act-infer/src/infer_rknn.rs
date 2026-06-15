use std::{ffi::CStr, fs, path::Path, ptr, time::Instant};

use anyhow::{Context, Result, bail};

use crate::{
    cli::CoreMask,
    rknn_sys::*,
    schema::{IMAGE_LEN, STATE_LEN, StageTiming, TimingMetrics},
};

/// `rknn_context` 的 RAII 包装，负责在离开作用域时释放上下文。
struct RknnContext(rknn_context);

impl Drop for RknnContext {
    fn drop(&mut self) {
        if self.0 != 0 {
            unsafe {
                rknn_destroy(self.0);
            }
        }
    }
}

fn attr_name(attr: &rknn_tensor_attr) -> String {
    // `name` 是固定长度数组中的 NUL 结尾 C 字符串。
    let bytes = unsafe { CStr::from_ptr(attr.name.as_ptr()) };
    bytes.to_string_lossy().into_owned()
}

/// 样本均值（毫秒）。空切片返回 0。
fn average_ms(samples: &[f64]) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    samples.iter().sum::<f64>() / samples.len() as f64
}

/// 线性插值分位数（毫秒）。`percentile` 取值 `[0,1]`。空切片返回 0。
fn percentile_ms(samples: &[f64], percentile: f64) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    let mut sorted = samples.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let rank = percentile * (sorted.len() - 1) as f64;
    let low = rank.floor() as usize;
    let high = (low + 1).min(sorted.len() - 1);
    let frac = rank - low as f64;
    sorted[low] * (1.0 - frac) + sorted[high] * frac
}

/// 把一个阶段的样本切片汇总成 avg/p50/p95 统计。
fn stage_timing(samples: &[f64]) -> StageTiming {
    StageTiming {
        avg_ms: average_ms(samples),
        p50_ms: percentile_ms(samples, 0.50),
        p95_ms: percentile_ms(samples, 0.95),
    }
}

/// 运行 ACT 的 RKNN 模型 1 次或多次，返回最后一次的原始归一化动作和耗时。
///
/// 模型期望有两个 float 输入（图像 NCHW `[1,3,224,224]`、状态 `[1,2]`）
/// 和一个 float 输出（`[1,chunk,action_dim]`）。
/// 输入会按元素数量映射到模型输入张量，因此导出图中的输入顺序变化也能兼容。
///
/// 计时按阶段拆分：`rknn_inputs_set` / `rknn_run` / `rknn_outputs_get` /
/// `rknn_outputs_release` 各自单独计时并按 `repeat` 收集样本，统计 avg/p50/p95；
/// 另外查询 `rknn_perf_run` 得到 NPU 硬件真实执行时间。`preprocess_ms`、
/// `normalize_state_ms`、`denormalize_ms` 由调用方（bin 层）测量后传入并回填，
/// 以便在同一份 `TimingMetrics` 中体现完整流水线开销。
pub fn run_model_timed(
    model_path: &Path,
    image: &[f32],
    state: &[f32],
    repeat: usize,
    core_mask: CoreMask,
) -> Result<(Vec<f32>, TimingMetrics)> {
    if image.len() != IMAGE_LEN {
        bail!("image buffer has {} f32, expected {IMAGE_LEN}", image.len());
    }
    if state.len() != STATE_LEN {
        bail!("state buffer has {} f32, expected {STATE_LEN}", state.len());
    }

    let model_bytes =
        fs::read(model_path).with_context(|| format!("failed to read {}", model_path.display()))?;

    let load_start = Instant::now();
    let mut raw_ctx: rknn_context = 0;
    let ret = unsafe {
        rknn_init(
            &mut raw_ctx as *mut rknn_context,
            model_bytes.as_ptr() as *mut _,
            model_bytes.len() as u32,
            0,
            ptr::null_mut(),
        )
    };
    if ret != RKNN_SUCC {
        bail!("rknn_init failed: ret={ret}");
    }
    let ctx = RknnContext(raw_ctx);
    let model_load_ms = load_start.elapsed().as_secs_f64() * 1000.0;

    // 打印 SDK / 驱动版本，便于定位板端环境差异。
    let mut sdk = rknn_sdk_version::default();
    let ret = unsafe {
        rknn_query(
            ctx.0,
            RKNN_QUERY_SDK_VERSION,
            &mut sdk as *mut _ as *mut _,
            std::mem::size_of::<rknn_sdk_version>() as u32,
        )
    };
    if ret == RKNN_SUCC {
        let api = unsafe { CStr::from_ptr(sdk.api_version.as_ptr()) }.to_string_lossy();
        let drv = unsafe { CStr::from_ptr(sdk.drv_version.as_ptr()) }.to_string_lossy();
        eprintln!("RKNN_SDK api={api} driver={drv}");
    }

    // 选择使用的 NPU 核心。
    let mask = match core_mask {
        CoreMask::Auto => RKNN_NPU_CORE_AUTO,
        CoreMask::All012 => RKNN_NPU_CORE_0_1_2,
    };
    let ret = unsafe { rknn_set_core_mask(ctx.0, mask) };
    if ret != RKNN_SUCC {
        // 非致命错误：AUTO 在单核/多核芯片上通常仍可继续工作。
        eprintln!("rknn_set_core_mask warning: ret={ret} (continuing with default)");
    }

    // 查询输入/输出数量。
    let mut io_num = rknn_input_output_num {
        n_input: 0,
        n_output: 0,
    };
    let ret = unsafe {
        rknn_query(
            ctx.0,
            RKNN_QUERY_IN_OUT_NUM,
            &mut io_num as *mut _ as *mut _,
            std::mem::size_of::<rknn_input_output_num>() as u32,
        )
    };
    if ret != RKNN_SUCC {
        bail!("rknn_query IN_OUT_NUM failed: ret={ret}");
    }
    if io_num.n_input < 1 || io_num.n_input > 3 {
        bail!("unexpected model input count: {}", io_num.n_input);
    }
    if io_num.n_output < 1 {
        bail!("model has no outputs");
    }

    // 查询输入属性，并按元素数量把 image/state 绑定到正确的输入张量。
    let mut input_attrs = vec![rknn_tensor_attr::default(); io_num.n_input as usize];
    for (i, attr) in input_attrs.iter_mut().enumerate() {
        attr.index = i as u32;
        let ret = unsafe {
            rknn_query(
                ctx.0,
                RKNN_QUERY_INPUT_ATTR,
                attr as *mut _ as *mut _,
                std::mem::size_of::<rknn_tensor_attr>() as u32,
            )
        };
        if ret != RKNN_SUCC {
            bail!("rknn_query INPUT_ATTR[{i}] failed: ret={ret}");
        }
        eprintln!(
            "RKNN_INPUT[{i}] name={} n_elems={} fmt={} type={}",
            attr_name(attr),
            attr.n_elems,
            attr.fmt,
            attr.type_
        );
    }

    let mut output_attrs = vec![rknn_tensor_attr::default(); io_num.n_output as usize];
    for (i, attr) in output_attrs.iter_mut().enumerate() {
        attr.index = i as u32;
        let ret = unsafe {
            rknn_query(
                ctx.0,
                RKNN_QUERY_OUTPUT_ATTR,
                attr as *mut _ as *mut _,
                std::mem::size_of::<rknn_tensor_attr>() as u32,
            )
        };
        if ret != RKNN_SUCC {
            bail!("rknn_query OUTPUT_ATTR[{i}] failed: ret={ret}");
        }
        eprintln!(
            "RKNN_OUTPUT[{i}] name={} n_elems={} size={}",
            attr_name(attr),
            attr.n_elems,
            attr.size
        );
    }

    // 按模型期望的输入顺序构造输入缓冲区。
    let mut inputs: Vec<rknn_input> = Vec::with_capacity(io_num.n_input as usize);
    // 在 rknn_inputs_set 调用期间保持所有权，避免裸指针悬空。
    let image_vec = image.to_vec();
    let state_vec = state.to_vec();
    for attr in &input_attrs {
        let (buf_ptr, byte_size, fmt) = match attr.n_elems as usize {
            IMAGE_LEN => (
                image_vec.as_ptr() as *mut _,
                (IMAGE_LEN * 4) as u32,
                RKNN_TENSOR_NCHW,
            ),
            STATE_LEN => (
                state_vec.as_ptr() as *mut _,
                (STATE_LEN * 4) as u32,
                RKNN_TENSOR_UNDEFINED,
            ),
            other => bail!(
                "input tensor '{}' has unexpected n_elems={other}",
                attr_name(attr)
            ),
        };
        inputs.push(rknn_input {
            index: attr.index,
            buf: buf_ptr,
            size: byte_size,
            pass_through: 0,
            type_: RKNN_TENSOR_FLOAT32,
            fmt,
        });
    }

    // 进入计时推理循环，每个阶段单独收集样本。
    let mut last_action: Vec<f32> = Vec::new();
    let mut inputs_set_samples: Vec<f64> = Vec::with_capacity(repeat);
    let mut run_samples: Vec<f64> = Vec::with_capacity(repeat);
    let mut outputs_get_samples: Vec<f64> = Vec::with_capacity(repeat);
    let mut outputs_release_samples: Vec<f64> = Vec::with_capacity(repeat);
    let mut npu_run_samples: Vec<f64> = Vec::with_capacity(repeat);
    let mut total_ms = 0.0_f64;
    let mut first_run_ms = 0.0_f64;
    for iter in 0..repeat {
        let iter_start = Instant::now();

        let set_start = Instant::now();
        let ret = unsafe { rknn_inputs_set(ctx.0, io_num.n_input, inputs.as_mut_ptr()) };
        if ret != RKNN_SUCC {
            bail!("rknn_inputs_set failed: ret={ret}");
        }
        let inputs_set_ms = set_start.elapsed().as_secs_f64() * 1000.0;

        let run_start = Instant::now();
        let ret = unsafe { rknn_run(ctx.0, ptr::null_mut()) };
        if ret != RKNN_SUCC {
            bail!("rknn_run failed: ret={ret}");
        }
        let run_ms = run_start.elapsed().as_secs_f64() * 1000.0;

        // 查询 NPU 硬件真实执行时间（单位微秒），失败时跳过该样本。
        let mut perf_run = rknn_perf_run::default();
        let perf_ret = unsafe {
            rknn_query(
                ctx.0,
                RKNN_QUERY_PERF_RUN,
                &mut perf_run as *mut _ as *mut _,
                std::mem::size_of::<rknn_perf_run>() as u32,
            )
        };
        if perf_ret == RKNN_SUCC && perf_run.run_duration >= 0 {
            npu_run_samples.push(perf_run.run_duration as f64 / 1000.0);
        }

        let mut outputs = vec![rknn_output::default(); io_num.n_output as usize];
        for (i, out) in outputs.iter_mut().enumerate() {
            out.index = i as u32;
            out.want_float = 1;
            out.is_prealloc = 0;
        }
        let get_start = Instant::now();
        let ret = unsafe {
            rknn_outputs_get(
                ctx.0,
                io_num.n_output,
                outputs.as_mut_ptr(),
                ptr::null_mut(),
            )
        };
        if ret != RKNN_SUCC {
            bail!("rknn_outputs_get failed: ret={ret}");
        }
        let outputs_get_ms = get_start.elapsed().as_secs_f64() * 1000.0;

        // 在释放输出前，把第一个输出（动作张量）拷贝出来。
        let out0 = &outputs[0];
        let n = (out0.size as usize) / 4;
        let mut action = vec![0.0_f32; n];
        if !out0.buf.is_null() && n > 0 {
            unsafe {
                ptr::copy_nonoverlapping(out0.buf as *const f32, action.as_mut_ptr(), n);
            }
        }
        last_action = action;

        let release_start = Instant::now();
        unsafe {
            rknn_outputs_release(ctx.0, io_num.n_output, outputs.as_mut_ptr());
        }
        let outputs_release_ms = release_start.elapsed().as_secs_f64() * 1000.0;

        let iter_ms = iter_start.elapsed().as_secs_f64() * 1000.0;
        total_ms += iter_ms;

        // 首次推理含 NPU warmup，单独记录并从稳态分位统计中剔除。
        if iter == 0 {
            first_run_ms = iter_ms;
            if repeat > 1 {
                continue;
            }
        }
        inputs_set_samples.push(inputs_set_ms);
        run_samples.push(run_ms);
        outputs_get_samples.push(outputs_get_ms);
        outputs_release_samples.push(outputs_release_ms);
    }

    if last_action.is_empty() {
        bail!("model produced an empty action output");
    }

    let timing = TimingMetrics {
        run_count: repeat,
        model_load_ms,
        // 一次性 CPU 阶段由 bin 层测量后回填。
        preprocess_ms: 0.0,
        normalize_state_ms: 0.0,
        inputs_set: stage_timing(&inputs_set_samples),
        run: stage_timing(&run_samples),
        outputs_get: stage_timing(&outputs_get_samples),
        outputs_release: stage_timing(&outputs_release_samples),
        npu_run: if npu_run_samples.is_empty() {
            None
        } else {
            Some(stage_timing(&npu_run_samples))
        },
        first_run_ms,
        denormalize_ms: 0.0,
        infer_single_ms: total_ms / repeat as f64,
        infer_total_ms: total_ms,
    };

    // `ctx` 在这里析构 -> 调用 `rknn_destroy`。
    Ok((last_action, timing))
}
