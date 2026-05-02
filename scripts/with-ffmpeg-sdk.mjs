#!/usr/bin/env node
import { spawn } from "node:child_process";
import { copyFileSync, existsSync, lstatSync, mkdirSync, readdirSync, readFileSync, readlinkSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { resolve } from "node:path";
import process from "node:process";

function resolveSdkRoot() {
  const workspaceRoot = resolve(import.meta.dirname, "..");
  const forcedSdkTarget = process.env.XBX_FFMPEG_SDK_TARGET;
  const platform = process.platform;
  const arch = process.arch;
  let sdkTarget = null;

  if (forcedSdkTarget) {
    sdkTarget = forcedSdkTarget;
  } else {
    if (platform === "win32" && arch === "x64") {
      sdkTarget = "windows-x64";
    } else if (platform === "darwin" && arch === "arm64") {
      sdkTarget = "macos-arm64";
    } else if (platform === "darwin" && arch === "x64") {
      sdkTarget = "macos-x64";
    } else {
      throw new Error(`Unsupported platform/arch: ${platform}/${arch}`);
    }
  }

  const sdkRoot = resolve(workspaceRoot, "src-tauri", "resources", "ffmpeg", sdkTarget);
  const includeDir = resolve(sdkRoot, "include");
  const libDir = resolve(sdkRoot, "lib");
  const binDir = resolve(sdkRoot, platform === "win32" ? "bin" : "lib");

  for (const dir of [sdkRoot, includeDir, libDir]) {
    if (!existsSync(dir)) {
      throw new Error(`FFmpeg SDK path missing: ${dir}`);
    }
  }

  return { sdkRoot, includeDir, libDir, binDir };
}

function run() {
  const splitIdx = process.argv.indexOf("--");
  if (splitIdx === -1 || splitIdx === process.argv.length - 1) {
    console.error("Usage: node scripts/with-ffmpeg-sdk.mjs -- <command> [args...]");
    process.exit(2);
  }

  const command = process.argv[splitIdx + 1];
  const args = process.argv.slice(splitIdx + 2);
  const { sdkRoot, includeDir, libDir, binDir } = resolveSdkRoot();
  const workspaceRoot = resolve(import.meta.dirname, "..");
  ensureLegacyRuntimeAlias({ sdkRoot });
  ensureRuntimeAssets({ workspaceRoot, libDir, binDir });
  const pkgConfigDir = resolve(libDir, "pkgconfig");
  const localPkgConfigDir = prepareLocalPkgConfig({
    sdkRoot,
    includeDir,
    libDir,
    pkgConfigDir,
  });
  const currentPath = process.env.PATH ?? "";
  const currentCmakeArgs = process.env.CMAKE_ARGS ?? "";
  const pathDelimiter = process.platform === "win32" ? ";" : ":";
  const cmakePolicyArg = "-DCMAKE_POLICY_VERSION_MINIMUM=3.5";
  const mergedCmakeArgs = currentCmakeArgs.includes(cmakePolicyArg)
    ? currentCmakeArgs
    : `${currentCmakeArgs} ${cmakePolicyArg}`.trim();
  const childEnv = {
    ...process.env,
    FFMPEG_DIR: sdkRoot,
    FFMPEG_INCLUDE_DIR: includeDir,
    FFMPEG_LIB_DIR: libDir,
    // 强制仅使用 vendored FFmpeg 的 pkg-config 元数据，避免误命中系统/Homebrew。
    PKG_CONFIG_PATH: localPkgConfigDir,
    PKG_CONFIG_LIBDIR: localPkgConfigDir,
    PKG_CONFIG_DIR: "",
    // 兼容新版 CMake（4.x）与部分旧依赖（如 audiopus_sys 内 vendored opus）的最低策略版本冲突。
    CMAKE_ARGS: mergedCmakeArgs,
    CMAKE_POLICY_VERSION_MINIMUM: "3.5",
    PATH: `${binDir}${pathDelimiter}${currentPath}`,
    // macOS dyld does not consult PATH for .dylib lookup.
    DYLD_LIBRARY_PATH: process.platform === "darwin"
      ? [libDir, process.env.DYLD_LIBRARY_PATH].filter(Boolean).join(":")
      : process.env.DYLD_LIBRARY_PATH,
    DYLD_FALLBACK_LIBRARY_PATH: process.platform === "darwin"
      ? [libDir, process.env.DYLD_FALLBACK_LIBRARY_PATH].filter(Boolean).join(":")
      : process.env.DYLD_FALLBACK_LIBRARY_PATH,
  };
  const isWindows = process.platform === "win32";
  const normalizedCommand = isWindows && command === "tauri" ? "tauri.cmd" : command;
  const child = isWindows
    ? spawn("cmd.exe", ["/d", "/s", "/c", normalizedCommand, ...args], {
      stdio: "inherit",
      shell: false,
      env: childEnv,
    })
    : spawn(normalizedCommand, args, {
      stdio: "inherit",
      shell: false,
      env: childEnv,
    });

  child.on("exit", (code, signal) => {
    if (signal) {
      process.kill(process.pid, signal);
      return;
    }
    process.exit(code ?? 1);
  });
}

function ensureLegacyRuntimeAlias({ sdkRoot }) {
  const platform = process.platform;
  const arch = process.arch;
  const ffmpegVersion = "ffmpeg-8.1";

  let runtimeSuffix = null;
  if (platform === "darwin" && arch === "arm64") {
    runtimeSuffix = "out-arm64";
  } else if (platform === "darwin" && arch === "x64") {
    runtimeSuffix = "out-x64";
  } else if (platform === "win32" && arch === "x64") {
    runtimeSuffix = "out-x64";
  } else {
    return;
  }

  // Keep compatibility with historical install_name entries that hardcode /tmp.
  const aliasPath = resolve("/tmp", ffmpegVersion, runtimeSuffix);
  const aliasParent = resolve(aliasPath, "..");
  mkdirSync(aliasParent, { recursive: true });

  if (existsSync(aliasPath)) {
    try {
      const stat = lstatSync(aliasPath);
      if (stat.isSymbolicLink()) {
        const linkedTo = resolve(aliasParent, readlinkSync(aliasPath));
        if (linkedTo === sdkRoot) {
          return;
        }
        rmSync(aliasPath, { recursive: true, force: true });
      } else {
        // Existing non-link path blocks alias creation. Replace it to keep runtime deterministic.
        rmSync(aliasPath, { recursive: true, force: true });
      }
    } catch {
      rmSync(aliasPath, { recursive: true, force: true });
    }
  }

  const symlinkType = process.platform === "win32" ? "junction" : "dir";
  symlinkSync(sdkRoot, aliasPath, symlinkType);
}

function ensureRuntimeAssets({ workspaceRoot, libDir, binDir }) {
  const platform = process.platform;
  if (platform !== "darwin" && platform !== "win32") {
    return;
  }

  const targetDirs = [
    resolve(workspaceRoot, "target", "debug"),
    resolve(workspaceRoot, "target", "release"),
  ];

  for (const targetDir of targetDirs) {
    mkdirSync(targetDir, { recursive: true });

    if (platform === "darwin") {
      const dylibEntries = readdirSync(libDir).filter(name => name.endsWith(".dylib"));
      for (const name of dylibEntries) {
        const src = resolve(libDir, name);
        const dst = resolve(targetDir, name);
        ensurePathLink({ src, dst });
      }
    } else if (platform === "win32") {
      const dllEntries = readdirSync(binDir).filter(name => name.toLowerCase().endsWith(".dll"));
      for (const name of dllEntries) {
        const src = resolve(binDir, name);
        const dst = resolve(targetDir, name);
        ensurePathCopy({ src, dst });
      }
    }
  }
}

function ensurePathLink({ src, dst }) {
  if (existsSync(dst)) {
    try {
      const stat = lstatSync(dst);
      if (stat.isSymbolicLink()) {
        const linkedTo = resolve(dst, "..", readlinkSync(dst));
        if (linkedTo === src) {
          return;
        }
      }
      rmSync(dst, { recursive: true, force: true });
    } catch {
      rmSync(dst, { recursive: true, force: true });
    }
  }
  const symlinkType = process.platform === "win32" ? "file" : "file";
  symlinkSync(src, dst, symlinkType);
}

function ensurePathCopy({ src, dst }) {
  if (existsSync(dst)) {
    rmSync(dst, { recursive: true, force: true });
  }
  copyFileSync(src, dst);
}

function prepareLocalPkgConfig({ sdkRoot, includeDir, libDir, pkgConfigDir }) {
  if (!existsSync(pkgConfigDir)) {
    throw new Error(`FFmpeg pkgconfig path missing: ${pkgConfigDir}`);
  }

  const outDir = resolve(tmpdir(), "xbxrc-ffmpeg-pkgconfig", process.platform, process.arch);
  mkdirSync(outDir, { recursive: true });

  const pcFiles = readdirSync(pkgConfigDir).filter(name => name.endsWith(".pc"));
  if (!pcFiles.length) {
    throw new Error(`No pkg-config files found in: ${pkgConfigDir}`);
  }

  for (const pcName of pcFiles) {
    const sourcePath = resolve(pkgConfigDir, pcName);
    const targetPath = resolve(outDir, pcName);
    const source = readFileSync(sourcePath, "utf8");
    const normalized = source
      .replace(/^prefix=.*$/m, `prefix=${sdkRoot}`)
      .replace(/^exec_prefix=.*$/m, "exec_prefix=$" + "{prefix}")
      .replace(/^libdir=.*$/m, `libdir=${libDir}`)
      .replace(/^includedir=.*$/m, `includedir=${includeDir}`);
    writeFileSync(targetPath, normalized);
  }

  return outDir;
}

run();
