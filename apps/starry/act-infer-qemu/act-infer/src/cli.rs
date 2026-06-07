use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Result, bail};

pub struct CommonArgs {
    pub model_path: PathBuf,
    pub image_path: PathBuf,
    pub normalize_path: PathBuf,
    pub state_path: Option<PathBuf>,
    pub output_path: Option<PathBuf>,
}

pub struct GoldenArgs {
    pub common: CommonArgs,
    pub golden_path: PathBuf,
    pub atol: f32,
}

pub fn parse_common_args(args: &[String]) -> Result<CommonArgs> {
    let mut model_path = None;
    let mut image_path = None;
    let mut normalize_path = None;
    let mut state_path = None;
    let mut output_path = None;

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
    };
    validate_common_args(&parsed)?;
    Ok(parsed)
}

pub fn parse_golden_args(args: &[String]) -> Result<GoldenArgs> {
    let mut kept = Vec::with_capacity(args.len());
    let mut golden_path = None;
    let mut atol = 1e-2_f32;

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

pub fn print_common_usage(bin_name: &str) {
    eprintln!(
        "usage:\n  {bin_name} --model ABS_ONNX --image ABS_JPG --normalize ABS_STATS_JSON \
         [--state ABS_STATE_BIN] [--output ABS_RESULT_JSON]"
    );
}

pub fn print_golden_usage(bin_name: &str) {
    eprintln!(
        "usage:\n  {bin_name} --model ABS_ONNX --image ABS_JPG --normalize ABS_STATS_JSON \
         --golden ABS_GOLDEN_JSON [--state ABS_STATE_BIN] [--output ABS_RESULT_JSON] [--atol 0.01]"
    );
}

pub fn write_json_if_requested(path: Option<&Path>, content: &str) -> Result<()> {
    if let Some(path) = path {
        fs::write(path, content)?;
    }
    Ok(())
}

fn require_absolute(name: &str, value: &str) -> Result<PathBuf> {
    let path = PathBuf::from(value);
    if !path.is_absolute() {
        bail!("{name} must be an absolute path: {value}");
    }
    Ok(path)
}

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
    if model_ext != "onnx" {
        bail!("--model must be .onnx: {}", args.model_path.display());
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

fn require_file(name: &str, path: &Path) -> Result<()> {
    if !path.is_file() {
        bail!("{name} path is not a file: {}", path.display());
    }
    Ok(())
}

impl GoldenArgs {
    fn validated(self) -> Result<Self> {
        require_file("--golden", &self.golden_path)?;
        Ok(self)
    }
}

fn require_present(name: &str, value: Option<PathBuf>) -> Result<PathBuf> {
    value.ok_or_else(|| anyhow::anyhow!("missing required argument: {name}"))
}
