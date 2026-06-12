use std::{ffi::CStr, fs, path::Path, ptr, time::Instant};

use anyhow::{Context, Result, bail};

use crate::{
    cli::CoreMask,
    rknn_sys::*,
    schema::{IMAGE_LEN, STATE_LEN, TimingMetrics},
};

/// RAII wrapper around an `rknn_context`.
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
    // name is a NUL-terminated C string inside a fixed array.
    let bytes = unsafe { CStr::from_ptr(attr.name.as_ptr()) };
    bytes.to_string_lossy().into_owned()
}

/// Run the ACT RKNN model once-or-N-times and return the (last) raw normalized
/// action vector along with timing metrics.
///
/// The model is expected to have two float inputs (image NCHW `[1,3,224,224]`
/// and state `[1,2]`) and a single float output (`[1,chunk,action_dim]`).
/// Inputs are mapped to the model's input tensors by element count so the
/// binding is robust to input ordering changes in the exported graph.
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

    // Report SDK / driver version for diagnostics.
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

    // Select NPU core(s).
    let mask = match core_mask {
        CoreMask::Auto => RKNN_NPU_CORE_AUTO,
        CoreMask::All012 => RKNN_NPU_CORE_0_1_2,
    };
    let ret = unsafe { rknn_set_core_mask(ctx.0, mask) };
    if ret != RKNN_SUCC {
        // Non-fatal: AUTO still works on single/multi core silicon.
        eprintln!("rknn_set_core_mask warning: ret={ret} (continuing with default)");
    }

    // Query input/output counts.
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

    // Query input attributes and map our buffers to input indices by element
    // count (image has IMAGE_LEN elems, state has STATE_LEN elems).
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

    // Build the input buffers in the order the model expects.
    let mut inputs: Vec<rknn_input> = Vec::with_capacity(io_num.n_input as usize);
    // Keep owned copies alive for the duration of rknn_inputs_set.
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

    // Timed inference loop.
    let mut last_action: Vec<f32> = Vec::new();
    let mut total_ms = 0.0_f64;
    for _ in 0..repeat {
        let ret = unsafe { rknn_inputs_set(ctx.0, io_num.n_input, inputs.as_mut_ptr()) };
        if ret != RKNN_SUCC {
            bail!("rknn_inputs_set failed: ret={ret}");
        }

        let run_start = Instant::now();
        let ret = unsafe { rknn_run(ctx.0, ptr::null_mut()) };
        if ret != RKNN_SUCC {
            bail!("rknn_run failed: ret={ret}");
        }

        let mut outputs = vec![rknn_output::default(); io_num.n_output as usize];
        for (i, out) in outputs.iter_mut().enumerate() {
            out.index = i as u32;
            out.want_float = 1;
            out.is_prealloc = 0;
        }
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
        let elapsed_ms = run_start.elapsed().as_secs_f64() * 1000.0;
        total_ms += elapsed_ms;

        // Copy the first output (action tensor) out before releasing.
        let out0 = &outputs[0];
        let n = (out0.size as usize) / 4;
        let mut action = vec![0.0_f32; n];
        if !out0.buf.is_null() && n > 0 {
            unsafe {
                ptr::copy_nonoverlapping(out0.buf as *const f32, action.as_mut_ptr(), n);
            }
        }
        last_action = action;

        unsafe {
            rknn_outputs_release(ctx.0, io_num.n_output, outputs.as_mut_ptr());
        }
    }

    if last_action.is_empty() {
        bail!("model produced an empty action output");
    }

    let timing = TimingMetrics {
        run_count: repeat,
        infer_single_ms: total_ms / repeat as f64,
        infer_total_ms: total_ms,
        model_load_ms,
    };

    // ctx dropped here -> rknn_destroy.
    Ok((last_action, timing))
}
