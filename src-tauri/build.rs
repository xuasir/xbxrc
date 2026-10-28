use sha2::{Digest, Sha256};
use std::path::PathBuf;
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
    let Some(path) = clang_runtime_library_path("libclang_rt.osx.a") else {
        return;
    };
    println!("cargo:rustc-link-arg=-Wl,-force_load,{}", path.display());
}

fn clang_runtime_library_path(lib: &str) -> Option<PathBuf> {
    let output = Command::new("clang")
        .arg(format!("--print-file-name={lib}"))
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let path = String::from_utf8(output.stdout).ok()?;
    let path = path.trim();
    if path.is_empty() || path == lib {
        return None;
    }
    let path = PathBuf::from(path);
    path.exists().then_some(path)
}

fn emit_build_metadata() {
    println!(
        "cargo:rustc-env=XBX_BUILD_SOURCE_SHA256={}",
        source_fingerprint().unwrap_or_else(|| "unknown".to_owned())
    );
    println!(
        "cargo:rustc-env=XBX_BUILD_GIT_COMMIT_SHORT={}",
        git_output(&["rev-parse", "--short", "HEAD"]).unwrap_or_else(|| "unknown".to_string())
    );
    println!(
        "cargo:rustc-env=XBX_BUILD_WORKSPACE_DIRTY={}",
        git_output(&["status", "--porcelain", "--untracked-files=normal"])
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

/// 把未提交及新增源码纳入构建身份，供 fresh trace 对应用户实际测试的源码。
fn source_fingerprint() -> Option<String> {
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").ok()?)
        .parent()?
        .to_path_buf();
    let inputs = [
        "src",
        "src-tauri/src",
        "src-tauri/build.rs",
        "src-tauri/tauri.conf.json",
        "src-tauri/Cargo.toml",
        "crates",
        "Cargo.toml",
        "Cargo.lock",
        "rust-toolchain.toml",
        "package.json",
        "pnpm-lock.yaml",
        "vite.config.ts",
        "tsconfig.json",
        "tsconfig.node.json",
        "tsconfig.app.json",
    ];
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(&root)
        .args([
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
            "--",
        ])
        .args(inputs);
    let output = command.output().ok()?;
    if !output.status.success() {
        return None;
    }
    let paths = String::from_utf8(output.stdout).ok()?;
    let mut paths: Vec<_> = paths.split('\0').filter(|path| !path.is_empty()).collect();
    paths.sort_unstable();
    paths.dedup();
    let mut hash = Sha256::new();
    for relative in paths {
        let path = root.join(relative);
        println!("cargo:rerun-if-changed={}", path.display());
        hash.update((relative.len() as u64).to_le_bytes());
        hash.update(relative.as_bytes());
        match std::fs::read(&path) {
            Ok(bytes) => {
                hash.update([1]);
                hash.update((bytes.len() as u64).to_le_bytes());
                hash.update(bytes);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => hash.update([0]),
            Err(_) => return None,
        }
    }
    // 目录监听覆盖下次构建前新增的源文件。
    for relative in ["src", "src-tauri/src", "crates"] {
        println!("cargo:rerun-if-changed={}", root.join(relative).display());
    }
    Some(format!("{:x}", hash.finalize()))
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
