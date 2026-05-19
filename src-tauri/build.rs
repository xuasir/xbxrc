use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn main() {
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=build.rs");
    emit_build_metadata();
    link_macos_availability_runtime();
    tauri_build::build()
}

/// SDL3 静态库中的 ObjC `@available` 会引用 `___isPlatformVersionAtLeast`。
fn link_macos_availability_runtime() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    println!("cargo:rustc-link-lib=clang_rt.osx");
}

fn emit_build_metadata() {
    println!(
        "cargo:rustc-env=XBX_BUILD_GIT_COMMIT_SHORT={}",
        git_output(&["rev-parse", "--short", "HEAD"]).unwrap_or_else(|| "unknown".to_string())
    );
    println!(
        "cargo:rustc-env=XBX_BUILD_WORKSPACE_DIRTY={}",
        git_output(&["status", "--porcelain", "--untracked-files=no"])
            .map(|output| (!output.trim().is_empty()).to_string())
            .unwrap_or_else(|| "false".to_string())
    );
    println!(
        "cargo:rustc-env=XBX_BUILD_TIMESTAMP_UNIX_MS={}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis().to_string())
            .unwrap_or_else(|_| "0".to_string())
    );
    println!(
        "cargo:rustc-env=XBX_BUILD_CARGO_PROFILE={}",
        std::env::var("PROFILE").unwrap_or_else(|_| "unknown".to_string())
    );
}

fn git_output(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout)
        .ok()
        .map(|value| value.trim().to_string())
}
