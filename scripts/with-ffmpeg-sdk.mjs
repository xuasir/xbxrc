#!/usr/bin/env node
import { spawn } from "node:child_process";
import { copyFileSync, existsSync, lstatSync, mkdirSync, readdirSync, readFileSync, readlinkSync, rmSync, statSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { resolve } from "node:path";
import process from "node:process";
import { createMacosBundleConfig } from "./macos-bundle-config.mjs";

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
  const appConfig = JSON.parse(readFileSync(resolve(workspaceRoot, "src-tauri", "tauri.conf.json"), "utf8"));
  if (process.platform === "darwin" && command === "tauri" && ["build", "bundle"].includes(args[0]) && !args.includes("--help") && !args.includes("-h")) {
    let signingIdentity = appConfig.bundle.macOS.signingIdentity;
    let hardenedRuntime = appConfig.bundle.macOS.hardenedRuntime;
    let knownSigningConfig = true;
    // 按Tauri配置顺序读取当前JSON覆盖；其他格式保留CLI自身的签名策略。
    for (let index = 0; index < args.length; index++) {
      if (!["--config", "-c"].includes(args[index])) continue;
      const input = args[++index];
      try {
        const overlay = JSON.parse(input.trim().startsWith("{") ? input : readFileSync(resolve(input), "utf8"));
        const mac = overlay.bundle?.macOS;
        if (mac && Object.hasOwn(mac, "signingIdentity")) signingIdentity = mac.signingIdentity;
        if (mac && Object.hasOwn(mac, "hardenedRuntime")) hardenedRuntime = mac.hardenedRuntime;
      } catch {
        knownSigningConfig = false;
      }
    }
    const effectiveIdentity = signingIdentity ?? process.env.APPLE_SIGNING_IDENTITY ?? "-";
    const overlay = createMacosBundleConfig(sdkRoot, { signingIdentity: effectiveIdentity, hardenedRuntime });
    if (!knownSigningConfig) delete overlay.bundle.macOS.hardenedRuntime;
    // 多个--config按顺序合并；此覆盖只装配SDK目录与已知签名策略。
    const separator = args.indexOf("--");
    args.splice(separator < 0 ? args.length : separator, 0, "--config", JSON.stringify(overlay));
  }
  if (process.platform === "win32" && command === "tauri" && ["build", "bundle"].includes(args[0]) && !args.includes("--help") && !args.includes("-h")) {
    const ffmpegRoot = resolve(sdkRoot, "..");
    const resources = Object.fromEntries([
      ...readdirSync(binDir).filter(name => name.toLowerCase().endsWith(".dll")).map(name => [resolve(binDir, name), name]),
      ...["LICENSE-NOTICE.md", "COPYING.LGPLv2.1", "COPYING.LGPLv3"].map(name => [resolve(ffmpegRoot, name), `licenses/ffmpeg/${name}`]),
    ]);
    const separator = args.indexOf("--");
    args.splice(separator < 0 ? args.length : separator, 0, "--config", JSON.stringify({ bundle: { resources } }));
  }
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
    CARGO_TARGET_DIR: resolve(workspaceRoot, process.env.CARGO_TARGET_DIR ?? "target/native-wip"),
    FFMPEG_DIR: sdkRoot,
    FFMPEG_INCLUDE_DIR: includeDir,
    FFMPEG_LIB_DIR: libDir,
    MACOSX_DEPLOYMENT_TARGET: process.platform === "darwin"
      ? appConfig.bundle.macOS.minimumSystemVersion
      : process.env.MACOSX_DEPLOYMENT_TARGET,
    // 未配置开发者证书时使用ad-hoc签名，保证本地与CI产物的签名完整性。
    APPLE_SIGNING_IDENTITY: process.platform === "darwin"
      ? process.env.APPLE_SIGNING_IDENTITY ?? appConfig.bundle.macOS.signingIdentity ?? "-"
      : process.env.APPLE_SIGNING_IDENTITY,
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

function ensureRuntimeAssets({ workspaceRoot, libDir, binDir }) {
  const platform = process.platform;
  if (platform !== "darwin" && platform !== "win32") {
    return;
  }

  const targetRoot = resolve(workspaceRoot, process.env.CARGO_TARGET_DIR ?? "target/native-wip");
  const targetDirs = [resolve(targetRoot, "debug"), resolve(targetRoot, "release")];

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
    const srcStat = statSync(src);
    const dstStat = statSync(dst);
    // Windows: DLLs under target/ are often still mapped by a previous dev run.
    // Same size strongly indicates the vendored copy is already in place; skip rm/copy.
    if (srcStat.size === dstStat.size) {
      return;
    }
    try {
      rmSync(dst, { recursive: true, force: true });
    } catch (err) {
      const code = err && typeof err === "object" ? err.code : undefined;
      if (code === "EPERM" || code === "EBUSY") {
        const dstStatAfter = statSync(dst);
        if (srcStat.size === dstStatAfter.size) {
          console.warn(
            `[with-ffmpeg-sdk] Skip replacing locked runtime DLL (same size as SDK): ${dst}`,
          );
          return;
        }
      }
      throw err;
    }
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
