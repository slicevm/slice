fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if target_os == "windows" && target_env == "msvc" {
        println!("cargo:rustc-link-arg=/STACK:16777216");
    } else if target_os == "linux" {
        println!("cargo:rustc-link-arg-bins=-Wl,-z,stack-size=16777216");
    }
}
