#!/usr/bin/env bash
set -euo pipefail

# RTCP 压测分支（可通过外部环境变量覆盖）
export XBXENGINE_NEGOTIATION_BRANCH="${XBXENGINE_NEGOTIATION_BRANCH:-rtcp-test}"
export XBXENGINE_NEGOTIATION_RESOLUTION="${XBXENGINE_NEGOTIATION_RESOLUTION:-2560x1440}"
export XBXENGINE_NEGOTIATION_BITRATE_KBPS="${XBXENGINE_NEGOTIATION_BITRATE_KBPS:-20000,30000,60000}"
export XBXENGINE_NEGOTIATION_MAX_FRAME_RATE="${XBXENGINE_NEGOTIATION_MAX_FRAME_RATE:-60}"
# 默认不强制 REMB，避免压测脚本隐式覆盖自适应策略。
export XBXENGINE_FORCE_REMB_KBPS="${XBXENGINE_FORCE_REMB_KBPS:-}"
export XBXENGINE_NAPI_PATH="${XBXENGINE_NAPI_PATH:-target/release/libxbxengine_api.dylib}"

echo "[rtcp-test] negotiation_branch=${XBXENGINE_NEGOTIATION_BRANCH}"
echo "[rtcp-test] resolution=${XBXENGINE_NEGOTIATION_RESOLUTION}"
echo "[rtcp-test] bitrate_kbps=${XBXENGINE_NEGOTIATION_BITRATE_KBPS}"
echo "[rtcp-test] max_frame_rate=${XBXENGINE_NEGOTIATION_MAX_FRAME_RATE}"
echo "[rtcp-test] force_remb_kbps=${XBXENGINE_FORCE_REMB_KBPS:-auto}"
echo "[rtcp-test] napi_path=${XBXENGINE_NAPI_PATH}"

pnpm run cargo:build:xbxengine-api:release
exec electron-vite dev
