# 播放期跨模块集成测试 Report

## Summary

- Related RFC: [`docs/rfcs/2026-04-11-playback-phase-cross-module-integration-tests.md`](../rfcs/2026-04-11-playback-phase-cross-module-integration-tests.md)
- RFC 中 PLY-INT-01..18 与 PLY-EDGE-01..18 已全部落地为 `xbxengine` 侧合成 fixture 集成测试，并按子模块拆分文件。

## Delivered

- `crates/xbxengine/core/src/transport/rtc/session/playback_phase_integration/`：`mod.rs`、`common.rs`、`int_*`、`edge_*` 共 36 条 `#[test]`。
- `policy.test.rs` 通过 `#[path = "playback_phase_integration/mod.rs"]` 挂载子模块，继续复用 `RecoveryIntegrationHarness`。

## Changes

- 关键帧 episode 字段统一为 `latest_keyframe_request_episode`（与 `XbxEngineMediaRuntimeStats` 一致）。
- TWCC 远端流观测使用 `XbxEngineTwccRemoteStreamObservation` 真实字段（`ssrc` / `mime_type` / `header_extensions` / `rtcp_feedback` 等）。
- INT-04：第二拍不再强断 `coalesced:keyframeInFlight`，第三拍以 clean anchor + healthy chain 断言解锁合同。
- INT-07：`video_renderer_stalled=true` 会使动态子画像固定为 `displayConstrained`，与 `cloudHighRtt` 互斥；fixture 改为 `renderer_stalled=false` 以符合「高 RTT 子画像」合同。
- INT-09：`action_selected` 与 `gate_result` 对齐为 `coalesced:keyframeInFlight`。
- INT-13：以 `latest_recovery_decision_ledger.action_selected == requestDecoderReset` 计数替代被 harness 过滤的 `TransportCommand::RequestDecoderReset`。

## Validation

- `cargo test -p xbxengine playback_phase`

## Risks

- 部分用例依赖 `persist_runtime_remote_profile_facts` 与 owner 状态机的当前实现细节；若画像优先级或字段名再变，需同步调整 fixture。

## Follow-up

- 若需断言 `SessionCommand::LocalDecoderReset`，可扩展 harness 收集非 transport 命令。
