use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Result, bail};

/// 两种运行模式（review/golden）通用的命令行参数。
pub struct CommonArgs {
    /// 模型文件路径（.rknn）。
    pub model_path: PathBuf,
    /// 输入图像路径（.jpg/.jpeg）。
    pub image_path: PathBuf,
    /// 归一化统计文件路径（stats.json）。
    pub normalize_path: PathBuf,
    /// 状态文件路径（可选，二进制 f32）。
    pub state_path: Option<PathBuf>,
    /// 结果 JSON 输出路径（可选）。
    pub output_path: Option<PathBuf>,
    /// 计时推理的重复次数（默认 1），用于获得更稳定的耗时统计。
    pub repeat: usize,
    /// NPU 核心选择掩码："auto" 或 "012"（同时使用三个核心）。
    pub core_mask: CoreMask,
}

/// NPU 核心掩码选择。
#[derive(Clone, Copy)]
pub enum CoreMask {
    /// 自动调度（由运行时决定使用哪个核心）。
    Auto,
    /// 同时使用 0/1/2 三个核心。
    All012,
}

/// golden 模式专用参数（在通用参数基础上扩展）。
pub struct GoldenArgs {
    /// 通用参数。
    pub common: CommonArgs,
    /// 基准文件路径。
    pub golden_path: PathBuf,
    /// 精度比对的绝对容差。
    pub atol: f32,
}

/// 解析通用命令行参数。参数按 `--key value` 成对出现。
pub fn parse_common_args(args: &[String]) -> Result<CommonArgs> {
    let mut model_path = None;
    let mut image_path = None;
    let mut normalize_path = None;
    let mut state_path = None;
    let mut output_path = None;
    let mut repeat = 1usize;
    let mut core_mask = CoreMask::Auto;

    let mut idx = 0;
    while idx < args.len() {
        let key = &args[idx];
        let Some(value) = args.get(idx + 1) else {
            bail!("missing value for argument: {key}");
        };
        match key.as_str() {
            "--model" => model_path = Some(require_absolute("--model", value)?),
            "--image" => image_path = Some(require_absolute("--image", value)?),
            "--normalize" => normalize_path = Some(require_absolute("--normalize", value)?),
            "--state" => state_path = Some(require_absolute("--state", value)?),
            "--output" => output_path = Some(require_absolute("--output", value)?),
            "--repeat" => {
                repeat = value
                    .parse::<usize>()
                    .map_err(|e| anyhow::anyhow!("invalid --repeat value: {e}"))?
                    .max(1);
            }
            "--core-mask" => {
                core_mask = match value.as_str() {
                    "auto" => CoreMask::Auto,
                    "012" | "all" => CoreMask::All012,
                    other => bail!("invalid --core-mask value: {other} (expected auto|012)"),
                }
            }
            _ => bail!("unsupported argument: {key}"),
        }
        idx += 2;
    }

    let parsed = CommonArgs {
        model_path: require_present("--model", model_path)?,
        image_path: require_present("--image", image_path)?,
        normalize_path: require_present("--normalize", normalize_path)?,
        state_path,
        output_path,
        repeat,
        core_mask,
    };
    validate_common_args(&parsed)?;
    Ok(parsed)
}

/// 解析 golden 模式参数：先抽取 `--golden`/`--atol`，其余参数透传给
/// `parse_common_args` 复用通用解析逻辑。
pub fn parse_golden_args(args: &[String]) -> Result<GoldenArgs> {
    // 保留下来、需要交给通用解析器处理的参数。
    let mut kept = Vec::with_capacity(args.len());
    let mut golden_path = None;
    // 精度比对的默认绝对容差。
    let mut atol = 5e-2_f32;

    let mut idx = 0;
    while idx < args.len() {
        let key = &args[idx];
        let Some(value) = args.get(idx + 1) else {
            bail!("missing value for argument: {key}");
        };
        match key.as_str() {
            "--golden" => golden_path = Some(require_absolute("--golden", value)?),
            "--atol" => {
                atol = value
                    .parse::<f32>()
                    .map_err(|e| anyhow::anyhow!("invalid --atol value: {e}"))?
            }
            _ => {
                kept.push(key.clone());
                kept.push(value.clone());
            }
        }
        idx += 2;
    }

    GoldenArgs {
        common: parse_common_args(&kept)?,
        golden_path: require_present("--golden", golden_path)?,
        atol,
    }
    .validated()
}

/// 打印通用用法说明。
pub fn print_common_usage(bin_name: &str) {
    eprintln!(
        "usage:\n  {bin_name} --model ABS_RKNN --image ABS_JPG --normalize ABS_STATS_JSON \
         [--state ABS_STATE_BIN] [--output ABS_RESULT_JSON] [--repeat N] [--core-mask auto|012]"
    );
}

/// 打印 golden 模式用法说明。
pub fn print_golden_usage(bin_name: &str) {
    eprintln!(
        "usage:\n  {bin_name} --model ABS_RKNN --image ABS_JPG --normalize ABS_STATS_JSON \
         --golden ABS_GOLDEN_JSON [--state ABS_STATE_BIN] [--output ABS_RESULT_JSON] [--repeat N] \
         [--core-mask auto|012] [--atol 0.05]"
    );
}

/// 当指定了输出路径时，把结果 JSON 写入该文件；否则不做任何事。
pub fn write_json_if_requested(path: Option<&Path>, content: &str) -> Result<()> {
    if let Some(path) = path {
        fs::write(path, content)?;
    }
    Ok(())
}

/// 要求参数值为绝对路径，否则报错（本工具统一使用绝对路径以避免歧义）。
fn require_absolute(name: &str, value: &str) -> Result<PathBuf> {
    let path = PathBuf::from(value);
    if !path.is_absolute() {
        bail!("{name} must be an absolute path: {value}");
    }
    Ok(path)
}

/// 校验通用参数：必需文件存在、模型为 .rknn、图像为 .jpg/.jpeg。
fn validate_common_args(args: &CommonArgs) -> Result<()> {
    require_file("--model", &args.model_path)?;
    require_file("--image", &args.image_path)?;
    require_file("--normalize", &args.normalize_path)?;
    if let Some(path) = args.state_path.as_ref() {
        require_file("--state", path)?;
    }
    let model_ext = args
        .model_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if model_ext != "rknn" {
        bail!("--model must be .rknn: {}", args.model_path.display());
    }

    let image_ext = args
        .image_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if image_ext != "jpg" && image_ext != "jpeg" {
        bail!("--image must be .jpg/.jpeg: {}", args.image_path.display());
    }
    Ok(())
}

/// 确认路径指向一个存在的文件，否则报错。
fn require_file(name: &str, path: &Path) -> Result<()> {
    if !path.is_file() {
        bail!("{name} path is not a file: {}", path.display());
    }
    Ok(())
}

impl GoldenArgs {
    /// 校验 golden 模式特有的参数（基准文件存在性）。
    fn validated(self) -> Result<Self> {
        require_file("--golden", &self.golden_path)?;
        Ok(self)
    }
}

fn require_present(name: &str, value: Option<PathBuf>) -> Result<PathBuf> {
    value.ok_or_else(|| anyhow::anyhow!("missing required argument: {name}"))
}
