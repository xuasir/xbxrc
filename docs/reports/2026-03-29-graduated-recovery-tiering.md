# 渐进恢复分级与单飞切换 Report

> 说明：本 Report 仅记录复杂任务完全完成后的最终总结。

## Summary

- Related RFC: [`docs/rfcs/2026-03-29-graduated-recovery-tiering.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-03-29-graduated-recovery-tiering.md)
- 已完成恢复链的分级改造：轻恢复优先走 RTCP `PLI/FIR`，硬恢复保留现有控制通道 keyframe / decoder reset 兜底。
- 已把恢复请求收敛为单飞升级模式，同一 epoch / 同一恢复意图不会叠加排队。

## Delivered

- `RtcConnectionService` 新增 video recovery transport state，支持 `PLI -> FIR -> control keyframe` 的渐进升级。
- `transport_session` 会把实际触发的恢复层级回写到观测标签，便于 trace 反查恢复路径。
- `control_channel` 增加独立的 pending keyframe 清理，避免轻恢复成功后误清空 decoder reset 债务。

## Changes

- 在 [`crates/xbxengine/core/src/transport/rtc/connection/service.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/connection/service.rs) 接通 RTCP `PictureLossIndication` 与 `FullIntraRequest` 发送路径，并引入单飞阶段状态与升级时序。
- 在 [`crates/xbxengine/core/src/transport/rtc/stack/transport_session.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stack/transport_session.rs) 将 `RequestKeyframe` 的成功观测改为动态标签，区分 `pli / fir / control`。
- 在 [`crates/xbxengine/core/src/transport/rtc/connection/control_channel.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/connection/control_channel.rs) 新增仅清理 pending keyframe 的能力，保持 decoder reset replay 独立。
- 在 [`docs/rfcs/2026-03-29-graduated-recovery-tiering.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-03-29-graduated-recovery-tiering.md) 与 [`docs/project-task.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/project-task.md) 回填了执行结果与完成态。

## Validation

- `cargo test -p xbxengine request_video_keyframe_ -- --nocapture`
- `cargo test -p xbxengine transport::rtc::stack::transport_session -- --nocapture`
- `cargo fmt`

## Risks

- 轻恢复阶段现在已经接通，但远端编码器与 receiver/SSRC 解析异常时仍会回退到 control keyframe，仍需继续观察复杂波动场景下的实际命中率。
- `video_pli_request_count_total` 当前仍复用为整体恢复 keyframe 活动计数，后续如果需要更细粒度统计，可能还要拆分指标。

## Follow-up

- 继续用最新 Cloud trace 观察 `awaitingRecoveryAnchor` 与 `requestKeyframe` 的密度变化。
- 如果后续 trace 仍显示同一 gap 反复恢复，可再收紧 coordinator 的升级冷却或补充更细的重复抑制。
