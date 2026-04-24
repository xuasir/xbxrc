# Video Recovery Success Consumption Fix RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 已完成
- Current State: completed
- Owner: TBD
- Last Updated: 2026-04-09

## Background

- 最新 runtime trace 显示恢复链路已经多次拿到 `keyframeRequestEpisode decoded|on-time`、`h264Inspection admissionAccepted=true`、`deltaContinuationReady=true`、`frame-complete-candidate` 等成功信号，但上层恢复 owner / ingress 仍长期停在 `rebuilding-supply` / `waitingKeyframe`。
- 结果是恢复成功没有被及时消费，`waitKeyframe` 继续挡住可承接的 delta slice，`noPendingFrame` 持续增长，最终被放大成恢复风暴和重连循环。
- 这次修复目标不是继续放宽阈值，而是把“恢复成功”真正回灌到状态机收口点。

## Goal

- 让恢复链在收到可用 keyframe / clean anchor / frame-complete-candidate 后，能及时退出 `waitingKeyframe` 和 `rebuilding-supply`。
- 保持 H264 / delta slice 的既有承接规则不回退，不误伤正常帧。
- 让 trace 能清晰验证“成功恢复已被消费”，而不是只看到恢复信号出现却迟迟不收口。

## Scope

- In scope:
  - `crates/xbxengine/core/src/transport/rtc/recovery/coordinator.rs`
  - `crates/xbxengine/core/src/transport/rtc/recovery/runtime_state.rs`
  - `crates/xbxengine/core/src/transport/rtc/policy/video_scheduling_owner.rs`
  - `crates/xbxengine/core/src/media/video/ingress/scheduler.rs`
  - 必要的 trace / runtime stats 补点
- Out of scope:
  - WebRTC 协议栈大改
  - H264 参数集语义再次重定义
  - 继续调大/调小恢复冷却阈值作为主方案

## Plan

1. 定位成功恢复没有被消费的具体收口点，确认是 owner 视图、恢复协调器还是 ingress gate 在拦截。
2. 实现最小但完整的状态收口：让 clean anchor / frame-complete-candidate / bootstrap-ready 真正解锁恢复态。
3. 补充 trace / stats / 测试，验证恢复成功后 `waitingKeyframe` 能退出，`rebuilding-supply` 能回到 `stable-serving`。

## Validation

- [x] `cargo fmt --all`
- [x] `cargo test -p xbxengine transport::rtc::recovery::coordinator -- --nocapture`
- [x] `cargo test -p xbxengine transport::rtc::policy::video_scheduling_owner -- --nocapture`
- [x] `cargo test -p xbxengine media::video::ingress::scheduler -- --nocapture`
- [x] `cargo test -p xbxengine media::video::decode::video_decode -- --nocapture`
- [ ] 运行一轮实机 trace 验证恢复风暴是否收口

## Risks

- 成功恢复消费过早会误放行还未稳定的链路，导致短暂抖动回弹。
- 状态机收口点如果只改一层，可能会出现 owner / ingress / decode 之间口径不一致。
- 需要避免把“bootstrap 不完整”再次误判成“实际恢复失败”。

## Progress

- [x] Step 1: 已定位到生产路径缺少 clean anchor 事实写入
- [x] Step 2: 已在 clean keyframe 提交时补 clean anchor 写入，并补回归测试
- [x] Step 3: 已验证通过，等待实机 trace 再次确认

## Execution Notes

- Date: 2026-04-03 | Status: completed
- Update: 在 `RtcVideoFrameSource` 的 clean keyframe 提交路径补上 `record_transport_clean_anchor(...)`，并用单测确认 clean anchor 事实确实写入 runtime stats；`VideoSchedulingOwner` 现可消费该事实退出 `rebuilding-supply`。
- Decision: 维持现有 H264 / delta slice 语义不变，只补成功恢复的事实写入与消费闭环。
- Risk/Blocker: 仍需一次实机 trace 观察 `video_anchor_clean_epoch` 是否在真实恢复窗口中稳定出现并驱动 owner 回到 stable-serving。
- Date: 2026-04-03 | Status: in-progress
- Update: 已创建 RFC，准备并行定位恢复状态未消费的具体代码点。
- Decision: 主方案聚焦“消费成功恢复信号”，不继续调节阈值作为主路径。
- Risk/Blocker: 需要确认 owner / ingress / coordinator 中哪一层没有接住成功恢复信号。
