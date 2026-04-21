#!/usr/bin/env node
import { spawn } from "node:child_process";
import { existsSync } from "node:fs";
import { resolve } from "node:path";

function resolveSdkRoot() {
  const workspaceRoot = resolve(import.meta.dirname, "..");
  const platform = process.platform;
  const arch = process.arch;
  let sdkTarget = null;

  if (platform === "win32" && arch === "x64") {
    sdkTarget = "windows-x64";
  } else if (platform === "darwin" && arch === "arm64") {
    sdkTarget = "macos-arm64";
  } else if (platform === "darwin" && arch === "x64") {
    sdkTarget = "macos-x64";
  } else {
    throw new Error(`Unsupported platform/arch: ${platform}/${arch}`);
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
  const currentPath = process.env.PATH ?? "";
  const pathDelimiter = process.platform === "win32" ? ";" : ":";
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

run();
