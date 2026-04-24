# Home TURN Gathering Failure Observability RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 未完成
- Current State: in-progress
- Owner: TBD
- Last Updated: 2026-04-09

## Background

- 最新 home trace 已确认 direct-first -> fallback TURN 主线生效，但 fallback attempt 里本地仍只有 `host` candidates，没有看到 `srflx/relay`。
- 当前 trace 还能看到 `StopRuntime`，但在链路上只能看到“停了”，看不到是谁、因为什么停；同时 `rtc` crate 已支持 `OnIceCandidateErrorEvent`，canonical 路径此前没有消费这类错误。

## Goal

- 把 “TURN/STUN gather 是否真实报错” 直接打进 runtime trace/日志。
- 把 `StopRuntime` 的调用原因从前端透传到 Tauri / xbxengine trace，便于还原是谁提前中断了 gather。
- 为下一轮判断“是 gather 真失败，还是被过早 stop”提供稳定证据。

## Scope

- In scope:
  - `crates/xbxengine` 的 ICE error 观测
  - `src/streaming/runtime/*` 到 `src-tauri/src/mods/xbxengine/*` 的 StopRuntime reason 透传
  - RFC 与 task tracker 更新
- Out of scope:
  - 直接修改 TURN gather 机制或更换 RTC 集成路线
  - 引入新的 recovery/backoff 行为
  - 完整修复 “为何本地没有 srflx/relay” 的最终方案

## Plan

1. 接住并记录 `OnIceCandidateErrorEvent` 细节。
2. 给 `StopRuntime` 加 reason 透传与前端触发日志。
3. 基于新日志再次复盘 trace，再决定是否要放宽过早停止。

## Validation

- [x] `cargo fmt --all`
- [x] `cargo check -p xbxengine`
- [ ] 使用新 trace 验证能看到 ICE error / StopRuntime reason

## Risks

- 这轮主要补观测，不能单独保证修复 srflx/relay 缺失。
- TS 静态检查本机未跑通，因为当前环境缺少 `node` / `pnpm`。

## Progress

- [x] Step 1: 已在 `RtcConnectionService::drain_peer_events()` 接住并记录 `OnIceCandidateErrorEvent`
- [x] Step 2: 已让 `StopRuntime` 支持透传 `reason`，并在 runtime host / xbxengine runtime / Tauri runtime state 打日志
- [ ] Step 3: 待用户用新构建复现后，根据新 trace 再决定是否需要调整 stop/recovery 时序

## Execution Notes

- Date: 2026-03-24 | Status: in-progress
- Update: 新增 ICE gather error 观测，并把 `StopRuntime(reason)` 从前端 runtime host 透传到 xbxengine control trace，当前能直接区分 `launch-failed`、`runtime-event:*:recovery-handler-threw` 等停止来源。
- Decision: 方案 B 先补“能证明真因”的观测，而不是盲目继续改 stop/recovery 策略；先确认到底是 TURN gather 自身报错，还是 gather 尚未完成就被前端 stop。
- Risk/Blocker: 仍需用户复现一份新 trace；如果 trace 里依旧没有 ICE error，但也没有 srflx/relay，则下一步要继续深挖 `rtc` sans-I/O 集成是否根本没有驱动到 STUN/TURN gather。
