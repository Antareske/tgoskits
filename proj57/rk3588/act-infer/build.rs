use std::{env, path::PathBuf};

fn main() {
    // RKNN 运行时库 `librknnrt.so` 由 RKNPU2 SDK 提供。
    // 允许通过 `RKNN_LIB_DIR` 覆盖；否则默认使用仓库中随程序打包的
    // `assets/sdk/aarch64` 目录。
    let lib_dir = env::var("RKNN_LIB_DIR").unwrap_or_else(|_| {
        let manifest_dir = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
        PathBuf::from(manifest_dir)
            .join("../assets/sdk/aarch64")
            .canonicalize()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| "../assets/sdk/aarch64".to_string())
    });

    println!("cargo:rustc-link-search=native={lib_dir}");
    println!("cargo:rustc-link-lib=dylib=rknnrt");
    // 嵌入 `$ORIGIN/lib` 的 rpath，这样二进制在 `./lib` 下放置 SDK 库时就能找到它。
    println!("cargo:rustc-link-arg=-Wl,-rpath,$ORIGIN/lib");
    println!("cargo:rustc-link-arg=-Wl,-rpath,$ORIGIN");
    println!("cargo:rerun-if-env-changed=RKNN_LIB_DIR");
}
