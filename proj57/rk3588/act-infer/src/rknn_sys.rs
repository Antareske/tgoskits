//! RKNPU2 运行时（`librknnrt.so`）的最小 FFI 绑定。
//!
//! 这里只绑定当前程序需要的那一小部分 `rknn_api.h`：
//! 支持单输入或多输入的 float 模型，以及 float 输出。
//! 结构体布局必须与 RKNPU2 SDK 2.4.2 的 `rknn_api.h` 保持一致，
//! 如 SDK 升级请同步检查 `assets/sdk/include/rknn_api.h`。
#![allow(non_camel_case_types, dead_code)]

use std::os::raw::{c_char, c_int, c_void};

pub type rknn_context = u64;

/// RKNN 接口的成功返回码。
pub const RKNN_SUCC: c_int = 0;

/// `rknn_query` 的查询类型。
pub const RKNN_QUERY_IN_OUT_NUM: c_int = 0;
pub const RKNN_QUERY_INPUT_ATTR: c_int = 1;
pub const RKNN_QUERY_OUTPUT_ATTR: c_int = 2;
/// 查询上一次 `rknn_run` 在 NPU 上的真实执行时间（微秒）。
pub const RKNN_QUERY_PERF_RUN: c_int = 4;
pub const RKNN_QUERY_SDK_VERSION: c_int = 5;

/// 张量数据类型。
pub const RKNN_TENSOR_FLOAT32: c_int = 0;
pub const RKNN_TENSOR_FLOAT16: c_int = 1;
pub const RKNN_TENSOR_INT8: c_int = 2;
pub const RKNN_TENSOR_UINT8: c_int = 3;

/// 张量布局格式。
pub const RKNN_TENSOR_NCHW: c_int = 0;
pub const RKNN_TENSOR_NHWC: c_int = 1;
pub const RKNN_TENSOR_UNDEFINED: c_int = 3;

/// NPU 核心掩码。
pub const RKNN_NPU_CORE_AUTO: c_int = 0;
pub const RKNN_NPU_CORE_0_1_2: c_int = 7;

pub const RKNN_MAX_DIMS: usize = 16;
pub const RKNN_MAX_NAME_LEN: usize = 256;

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct rknn_input_output_num {
    pub n_input: u32,
    pub n_output: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct rknn_tensor_attr {
    /// 张量索引。
    pub index: u32,
    /// 张量维度数。
    pub n_dims: u32,
    /// 各维度大小。
    pub dims: [u32; RKNN_MAX_DIMS],
    /// 张量名称（C 字符串）。
    pub name: [c_char; RKNN_MAX_NAME_LEN],
    /// 元素总数。
    pub n_elems: u32,
    /// 张量字节大小。
    pub size: u32,
    /// 张量布局格式。
    pub fmt: c_int,
    /// 张量类型。
    pub type_: c_int,
    /// 量化类型。
    pub qnt_type: c_int,
    /// 量化参数位宽。
    pub fl: i8,
    /// 零点。
    pub zp: i32,
    /// 量化缩放因子。
    pub scale: f32,
    /// 宽度 stride。
    pub w_stride: u32,
    /// 带 stride 的总大小。
    pub size_with_stride: u32,
    /// 是否透传。
    pub pass_through: u8,
    /// 高度 stride。
    pub h_stride: u32,
}

impl Default for rknn_tensor_attr {
    fn default() -> Self {
        // SAFETY: 该结构体只包含整数、浮点数和固定长度数组，
        // 因此全 0 位模式是合法值。
        unsafe { std::mem::zeroed() }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct rknn_sdk_version {
    /// API 版本字符串。
    pub api_version: [c_char; 256],
    /// 驱动版本字符串。
    pub drv_version: [c_char; 256],
}

impl Default for rknn_sdk_version {
    fn default() -> Self {
        unsafe { std::mem::zeroed() }
    }
}

/// `RKNN_QUERY_PERF_RUN` 的返回结构：上一次推理在 NPU 上的真实执行时间。
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct rknn_perf_run {
    /// NPU 真实推理耗时（微秒）。
    pub run_duration: i64,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct rknn_input {
    /// 输入张量索引。
    pub index: u32,
    /// 输入缓冲区指针。
    pub buf: *mut c_void,
    /// 输入数据大小（字节）。
    pub size: u32,
    /// 是否透传。
    pub pass_through: u8,
    /// 输入张量类型。
    pub type_: c_int,
    /// 输入张量布局。
    pub fmt: c_int,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct rknn_output {
    /// 是否要求返回 float 数据。
    pub want_float: u8,
    /// 是否由调用方预分配缓冲区。
    pub is_prealloc: u8,
    /// 输出张量索引。
    pub index: u32,
    /// 输出缓冲区指针。
    pub buf: *mut c_void,
    /// 输出数据大小（字节）。
    pub size: u32,
}

impl Default for rknn_output {
    fn default() -> Self {
        unsafe { std::mem::zeroed() }
    }
}

unsafe extern "C" {
    pub fn rknn_init(
        context: *mut rknn_context,
        model: *mut c_void,
        size: u32,
        flag: u32,
        extend: *mut c_void,
    ) -> c_int;

    pub fn rknn_destroy(context: rknn_context) -> c_int;

    pub fn rknn_query(context: rknn_context, cmd: c_int, info: *mut c_void, size: u32) -> c_int;

    pub fn rknn_inputs_set(context: rknn_context, n_inputs: u32, inputs: *mut rknn_input) -> c_int;

    pub fn rknn_set_core_mask(context: rknn_context, core_mask: c_int) -> c_int;

    pub fn rknn_run(context: rknn_context, extend: *mut c_void) -> c_int;

    pub fn rknn_outputs_get(
        context: rknn_context,
        n_outputs: u32,
        outputs: *mut rknn_output,
        extend: *mut c_void,
    ) -> c_int;

    pub fn rknn_outputs_release(
        context: rknn_context,
        n_outputs: u32,
        outputs: *mut rknn_output,
    ) -> c_int;
}
