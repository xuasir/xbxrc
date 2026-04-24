# xbxengine-core 全面切换至 rtc RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 已完成
- Current State: completed
- Owner: TBD
- Last Updated: 2026-04-09

## Background

- `xbxengine-core` 的 RTC 主体已经切到 `rtc 0.9`，但媒体、反馈、STUN/TURN 辅助链路仍混用 `webrtc-rs 0.17` 生态。
- 当前直接残留包括 `webrtc` / `webrtc-media` / `webrtc-util` / `rtp` / `rtcp` / `stun` / `turn`，会导致类型边界分裂、维护成本升高，也不利于后续统一演进 sans-io 主线。

## Goal

- 将 `xbxengine-core` 中直接使用的 `webrtc-rs 0.17` 相关依赖全面迁移到 `rtc-* 0.9`。
- 完成后，`xbxengine-core` 不再直接依赖 `webrtc` / `webrtc-media` / `webrtc-util` / `rtp` / `rtcp` / `stun` / `turn` 这套 0.17 主线。

## Scope

- In scope:
  - `Cargo.toml`
  - `crates/xbxengine/core/Cargo.toml`
  - `crates/xbxengine/core/src/transport/rtc/stream/*`
  - `crates/xbxengine/core/src/transport/rtc/connection/*`
  - `crates/README.md`
- Out of scope:
  - `rtc` 主连接架构重写
  - 非 `xbxengine-core` crate 的协议栈切换
  - 与本次迁移无关的 UI / Tauri / session 流程调整

## Plan

1. 将 RTP/RTCP/media 类型使用面切到 `rtc-*`
2. 将 STUN/TURN 辅助链路切到 `rtc-*`
3. 删除 0.17 直依赖并完成回归验证

## Validation

- [ ] `cargo fmt --all`
- [ ] `cargo check -p xbxengine`
- [ ] `cargo test -p xbxengine --lib`

## Risks

- `rtc-* 0.9` 与旧 `0.17` 类型混用期间容易出现 trait / type 不兼容。
- TURN / STUN 行为迁移虽然 API 接近，但 runtime 行为差异可能影响 relay / srflx 候选产出。

## Progress

- [x] Step 1: 完成迁移面审计，确认直接耦合入口集中在 7 个文件
- [x] Step 2: RTP/RTCP/media 类型迁移完成，视频 sample builder、RTP 包装、TWCC/NACK 已切到 `rtc-*`
- [x] Step 3: STUN/TURN 辅助链路迁移完成，srflx 探测与 relay runtime 已切到 `rtc-stun` / `rtc-turn`
- [x] Step 4: 删除 `webrtc-rs 0.17` 直依赖并完成格式化、编译与单测验证

## Execution Notes

- Date: 2026-03-24 | Status: in-progress
- Update: 建立 RFC，确认 `xbxengine-core` 当前已是 `rtc` 主体 + `webrtc-rs 0.17` 边缘残留的混合态，并按 `RTP/RTCP/media -> STUN/TURN -> 依赖清理` 三阶段推进。
- Decision: 先做低风险类型迁移，再处理 STUN/TURN 行为迁移，最后统一删除 0.17 直依赖。
- Risk/Blocker: `rtc-turn` / `rtc-stun` 的 API 虽有对应实现，但需要结合当前 `TurnRuntime` / `RtcIoRuntime` 现有泵送行为做细致对齐。

- Date: 2026-03-24 | Status: in-progress
- Update: 已完成三阶段迁移实现；`TurnRuntime` 改为基于 `rtc-turn` 的同步泵送模型，`RtcIoRuntime` 的 srflx 探测改用 `rtc-stun`，并删除 `webrtc-rs 0.17` 相关直依赖。
- Decision: relay runtime 保持当前 `RtcIoRuntime::pump()` 可接入的同步队列/泵送接口，不引入额外 async runtime，减少宿主边界漂移。
- Risk/Blocker: 仍需跑 `cargo fmt --all` 与更完整的 `cargo test -p xbxengine --lib`，确认 TURN 权限刷新与既有测试在当前仓库状态下都稳定。

- Date: 2026-03-24 | Status: completed
- Update: `cargo fmt --all`、`cargo check -p xbxengine` 与 `cargo test -p xbxengine --lib` 全部通过；本次迁移闭环完成。
- Decision: 维持 `rtc-turn` 的同步泵送封装，保证对现有 `RtcIoRuntime` 边界最小侵入。
- Risk/Blocker: 无阻塞；后续只剩历史文案与更细粒度 TURN 行为守护测试可选清理。
