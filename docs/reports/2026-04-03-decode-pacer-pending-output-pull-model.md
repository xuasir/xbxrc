# Decode Pacer Pending-Output Pull-Model Report

> 说明：本 Report 仅用于复杂任务完全完成后的最终总结。

## Summary

- Related RFC: [`docs/rfcs/2026-04-03-decode-pacer-pending-output-pull-model.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-04-03-decode-pacer-pending-output-pull-model.md)
- 已完成 `decode -> pacer` 的 pending-output / pull-model 收口，核心落在 [`crates/xbxengine/core/src/media/video/decode/actor.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/media/video/decode/actor.rs)，并把 pending-output 的归属收回到 [`crates/xbxengine/core/src/media/video/decode/video_decode.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/media/video/decode/video_decode.rs)；[`crates/xbxengine/core/src/media/video/types.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/media/video/types.rs) 仅做了 `DecodedFrame` 的可克隆支撑。

## Delivered

- `XbxVideoDecodeState` 现在持有 decoded output 的唯一队列，`DecodeActor` 只负责 drain / retry / stop，不再外挂临时 pending 队列。
- 当 `pacer.submit(...)` 出现短背压时，front frame 会保留在 decode 队列里并短超时重试，不会过早转成 pipeline drop。
- `decodePacerBackpressure` 与 `decodePacerBackpressureCleared` 的运行态观察补上了，便于看见“正在等”与“已经恢复”。
- 保留了 `outputQueueOverflow` / `pacerBackpressure` / `pacerDisconnected` 这些既有语义，并补了 actor 级回归测试。

## Changes

- 把 pending-output 的队列归属从 actor 临时状态收回到 `XbxVideoDecodeState`，actor 只在 output 为空时继续吃入新帧。
- 增加了背压重排、断开释放、队列重入顺序三类行为的定向测试。
- 收紧了 RFC 范围，使其与实际实现保持一致，并把运行态背压观察补进了现有 observation 入口。

## Validation

- `cargo fmt --all`
- `cargo test -p xbxengine media::video::decode::actor -- --nocapture`
- `cargo test -p xbxengine media::video::decode::video_decode -- --nocapture`
- `cargo check -p xbxengine`

## Risks

- `PENDING_PACER_RETRY_TIMEOUT_MS` 仍是保守的小值，后续若出现更高频背压，可能需要再调优。
- 该实现只覆盖 decode -> pacer 的 pull-model，没有升级到 ingress 完整 pull-model。

## Follow-up

- 后续如果要继续向 Moonlight 风格靠近，可以再评估把 ingress 侧的输出调度一起收口，但那会超出本轮范围。
