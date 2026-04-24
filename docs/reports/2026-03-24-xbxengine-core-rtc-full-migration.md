# xbxengine-core 全面切换至 rtc Report

> 说明：本 Report 仅用于复杂任务完全完成后的最终总结，不用于记录执行中的阶段性进度；中间过程应持续回写到对应 RFC。

## Summary

- Related RFC: [`docs/rfcs/2026-03-24-xbxengine-core-rtc-full-migration.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-03-24-xbxengine-core-rtc-full-migration.md)
- 已完成 `xbxengine-core` 对 `webrtc-rs 0.17` 直依赖的全面切换，RTP/RTCP/media/STUN/TURN 直接使用面已统一迁移到 `rtc-* 0.9`。

## Delivered

- 将视频 sample builder、RTP 包装、TWCC/NACK 类型切换到 `rtc-media` / `rtc-rtp` / `rtc-rtcp` / `rtc-shared`
- 将 srflx 探测与 TURN relay runtime 切换到 `rtc-stun` / `rtc-turn`
- 删除 `xbxengine-core` 中 `webrtc` / `webrtc-media` / `webrtc-util` / `rtp` / `rtcp` / `stun` / `turn` / `interceptor` 的 0.17 直依赖

## Changes

- `crates/xbxengine/core/src/transport/rtc/stream/video_source/*` 已全部改用 `rtc-*` 类型路径
- `crates/xbxengine/core/src/transport/rtc/connection/io_runtime.rs` 的 srflx STUN 请求已改用 `rtc-stun`
- `crates/xbxengine/core/src/transport/rtc/connection/turn_runtime.rs` 已改为基于 `rtc-turn` 的同步泵送与权限队列模型

## Validation

- `cargo fmt --all`
- `cargo check -p xbxengine`
- `cargo test -p xbxengine --lib`

## Risks

- 当前 `TurnRuntime` 继续保留项目现有的同步泵送封装，后续若要进一步复用更多 `rtc-turn` 原生能力，可再评估是否抽象更细的 relay 状态机
- 本次未清理历史注释/日志中所有 `webrtc-rs` 文案，仅同步了关键文档与依赖主线口径

## Follow-up

- 可继续清理代码注释、日志标签与文档中残留的 `webrtc-rs` 旧表述
- 如需进一步降低维护成本，可补一组专门覆盖 TURN permission/relay 行为的单测
