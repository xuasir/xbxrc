#!/usr/bin/env node
import { spawn } from "node:child_process";
import { existsSync, mkdirSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
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

  const sdkRoot = resolve(workspaceRoot, "third_party", "ffmpeg", sdkTarget);
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
  const spawnCommand
    = process.platform === "win32" && command === "tauri"
      ? "tauri.cmd"
      : command;

  const child = spawn(spawnCommand, args, {
    stdio: "inherit",
    shell: false,
    env: {
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
    },
  });

  child.on("exit", (code, signal) => {
    if (signal) {
      process.kill(process.pid, signal);
      return;
    }
    process.exit(code ?? 1);
  });
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
