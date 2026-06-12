use std::{env, path::PathBuf};

fn main() {
    // The RKNN runtime library (librknnrt.so) is provided by the RKNPU2 SDK.
    // Allow overriding via RKNN_LIB_DIR; otherwise default to the staged SDK
    // directory shipped alongside this app (assets/sdk/aarch64).
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
    // Embed an rpath of $ORIGIN/lib so the binary finds librknnrt.so when the
    // SDK library is deployed next to it under a ./lib subdirectory.
    println!("cargo:rustc-link-arg=-Wl,-rpath,$ORIGIN/lib");
    println!("cargo:rustc-link-arg=-Wl,-rpath,$ORIGIN");
    println!("cargo:rerun-if-env-changed=RKNN_LIB_DIR");
}
