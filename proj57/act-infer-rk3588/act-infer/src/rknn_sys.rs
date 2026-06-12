//! Minimal FFI bindings for the RKNPU2 runtime (`librknnrt.so`).
//!
//! Only the subset of `rknn_api.h` required for a single-input or
//! multi-input float model with float outputs is bound here. The struct
//! layouts mirror `rknn_api.h` from RKNPU2 SDK 2.4.2 exactly; keep them in
//! sync with `assets/sdk/include/rknn_api.h`.
#![allow(non_camel_case_types, dead_code)]

use std::os::raw::{c_char, c_int, c_void};

pub type rknn_context = u64;

pub const RKNN_SUCC: c_int = 0;

// rknn_query_cmd
pub const RKNN_QUERY_IN_OUT_NUM: c_int = 0;
pub const RKNN_QUERY_INPUT_ATTR: c_int = 1;
pub const RKNN_QUERY_OUTPUT_ATTR: c_int = 2;
pub const RKNN_QUERY_SDK_VERSION: c_int = 5;

// rknn_tensor_type
pub const RKNN_TENSOR_FLOAT32: c_int = 0;
pub const RKNN_TENSOR_FLOAT16: c_int = 1;
pub const RKNN_TENSOR_INT8: c_int = 2;
pub const RKNN_TENSOR_UINT8: c_int = 3;

// rknn_tensor_format
pub const RKNN_TENSOR_NCHW: c_int = 0;
pub const RKNN_TENSOR_NHWC: c_int = 1;
pub const RKNN_TENSOR_UNDEFINED: c_int = 3;

// rknn_core_mask
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
    pub index: u32,
    pub n_dims: u32,
    pub dims: [u32; RKNN_MAX_DIMS],
    pub name: [c_char; RKNN_MAX_NAME_LEN],
    pub n_elems: u32,
    pub size: u32,
    pub fmt: c_int,
    pub type_: c_int,
    pub qnt_type: c_int,
    pub fl: i8,
    pub zp: i32,
    pub scale: f32,
    pub w_stride: u32,
    pub size_with_stride: u32,
    pub pass_through: u8,
    pub h_stride: u32,
}

impl Default for rknn_tensor_attr {
    fn default() -> Self {
        // SAFETY: the struct is a plain-old-data type made only of integers,
        // floats and fixed-size arrays, so an all-zero bit pattern is valid.
        unsafe { std::mem::zeroed() }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct rknn_sdk_version {
    pub api_version: [c_char; 256],
    pub drv_version: [c_char; 256],
}

impl Default for rknn_sdk_version {
    fn default() -> Self {
        unsafe { std::mem::zeroed() }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct rknn_input {
    pub index: u32,
    pub buf: *mut c_void,
    pub size: u32,
    pub pass_through: u8,
    pub type_: c_int,
    pub fmt: c_int,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct rknn_output {
    pub want_float: u8,
    pub is_prealloc: u8,
    pub index: u32,
    pub buf: *mut c_void,
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
