# 解码到显示链的 pending-output pull-model RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 已完成
- Current State: completed
- Owner: TBD
- Last Updated: 2026-04-09

## Background

- 现有显示链已经有队列历史驱动的 pacing 和宿主 cadence 联动，但 `decode -> pacer` 仍是“有帧就尽快推送”，没有显式的 pending-output 意识。
- `XbxVideoDecodeState` 目前只提供 `pop_decoded_frame()`，`DecodeActor` 在每次收到编码帧后会立刻把 decoded frame 尽量推给 `pacer`；当 `pacer` 临时背压时，已经解出来的帧会更容易被再次丢弃。
- 这使得 decoder/pacer 之间缺少一个更细粒度的“有待输出时优先排空、backpressure 时保留并重试”的闭环，仍偏向 push-first 而不是 pull-aware。

## Goal

- 让 `decode -> pacer` 具备最小可用的 pending-output / pull-model。
- 在 `pacer` 短时背压时，尽量保留 decoded 输出并短超时重试，避免把已解码帧过早记成 pipeline drop。
- 保持现有 ingress / recovery / BWE 主线不变，只收口 decoder 和 pacer 之间的输出推进语义。

## Scope

- In scope:
  - [`crates/xbxengine/core/src/media/video/decode/actor.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/media/video/decode/actor.rs)
  - [`crates/xbxengine/core/src/media/video/decode/video_decode.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/media/video/decode/video_decode.rs)
  - [`crates/xbxengine/core/src/media/video/types.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/media/video/types.rs)
- Out of scope:
  - ingress / jitter buffer / frame admission 的完整重构
  - recovery / BWE / owner 策略改动
  - renderer / native video presenter 行为改写
  - 新增独立 media runtime 或第二套 pipeline

## Plan

1. 在 `decode_state` 中承接 decoded output 的唯一队列归属，并让 `decode actor` 只负责 drain / retry / stop 语义。
2. 补齐定向测试，验证短暂背压下不会过早 drop、断开时会明确释放当前 pending frame、队列重排语义保持不变。
3. 完成格式化、单测和整仓检查，并回写任务跟踪与最终报告。

## Validation

- [x] `cargo test -p xbxengine media::video::decode::actor -- --nocapture`
- [x] `cargo test -p xbxengine media::video::decode::video_decode -- --nocapture`
- [x] `cargo check -p xbxengine`

## Risks

- 如果 retry 机制过于保守，可能让 decoded output 在队列里停留太久，反而增大端到端延迟。
- 如果 peek/retry 接口设计不够小，容易把 decode 状态对象变复杂。
- 当前工作树已有并行改动，必须只动 decode/pacer 相关文件，避免踩到其它收口任务。

## Progress

- [x] Step 1: 已完成问题边界收敛，确认本轮只做 decode -> pacer pending-output / pull-model
- [x] Step 2: 完成 decode 输出队列归属收口与 actor 推进改造
- [x] Step 3: 完成定向验证、运行态背压观测与任务跟踪收尾

## Execution Notes

- Date: 2026-04-03 | Status: completed
- Update: 已在 `decode/actor.rs`、`decode/video_decode.rs` 与 `media/video/types.rs` 完成 pending-output / pull-model 收口：decoded output 现在由 `decode_state` 自己持有，`decode actor` 只在队列非空时 drain 并在 `pacer.submit(...)` 短背压时保留 front frame 重试；断开时明确释放当前 frame 并记录 `pacerDisconnected`；同时补了背压进入/退出的运行态观察，`outputQueueOverflow` / `pacerBackpressure` / `pacerDisconnected` 的语义方向保持一致。
- Decision: 本轮不扩展到 ingress 完整 pull-model，不改 recovery / BWE / presenter 主线。
- Risk/Blocker: 仍需要观察更长时间的真实背压样本，确认 `PENDING_PACER_RETRY_TIMEOUT_MS` 与 `decoded_frame_queue` 容量在高抖动场景下不会过度放大端到端延迟。
