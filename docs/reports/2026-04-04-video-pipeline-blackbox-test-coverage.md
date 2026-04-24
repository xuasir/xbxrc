# Video Pipeline Blackbox Test Coverage Report

> 说明：本 Report 仅用于复杂任务完全完成后的最终总结，不用于记录执行中的阶段性进度；中间过程应持续回写到对应 RFC。

## Summary

- Related RFC: [`docs/rfcs/2026-04-04-video-pipeline-blackbox-test-coverage.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-04-04-video-pipeline-blackbox-test-coverage.md)
- 本轮已基于真实 runtime trace 的行为模式，为接包、组帧、解码、渲染链路补齐黑盒测试与跨阶段集成测试，并继续把覆盖扩展到启动期 priming、`cooldownSuppressed` 出口、`transportAwaitRecoveryAnchor` 坏窗升级和 host viewport 时序合同。

## Delivered

- `video_source/sink` 黑盒边界测试：primary passthrough、非 RTX repair 丢弃、truncated RTX 丢弃。
- `video_source/source` 黑盒组帧测试：真实 RTP bootstrap 成帧、缺参数集时拒收并发出 `AwaitRecoveryKeyframe`、repair/RTX 补齐 bootstrap 缺口后成功成帧、缺少 follow-up boundary 时不误出半帧。
- `video_decode` 集成测试：`assembled -> decode -> render` 闭环、backend failure 后经 clean bootstrap 恢复到 `Nominal`。
- actor / 端到端测试：`render/actor` 的 overwrite 与 presentError 统计投影，以及 `RTP -> source -> decode -> pacer -> renderer` 完整链路回归。
- `renderer` 行为边界测试：latest slot 持续被覆盖时维持 `latest-overwrite`，直到最新帧被 ack 后才允许恢复 `nominal`。
- 测试资产化：新增 `crates/xbxengine/core/src/media/video/test_fixtures.rs`，把 bootstrap RTP/H264 样本、发送 helper、组帧结果构造与 `RtcVideoFrameSource` harness 收敛成共享 fixture。
- 启动期/恢复协调器黑盒测试：在 `session/policy.rs` 新增“`Connected + remoteTrackAttached + 无 host/decode 输出` 的首帧前 priming 窗口内不得因 `adapterIdleTimeout` 误入恢复链，窗口到期后必须升级”，在 `recovery/coordinator.rs` 新增“`cooldownSuppressed` 在 `Connected + track attached + 0 present/decode progress` 下不能无限续命”。
- viewport/host 时序黑盒测试：在 `src-tauri/src/mods/native_video/scheduling.rs` 新增“只有 display tick、没有 first present 时，即使连续 `noPendingFrame` 也必须维持 `priming`，不能提前进入 `starved/steady`”。

## Changes

- 用历史日志 `runtime-trace-1775289054327.jsonl`、`runtime-trace-1775292592042.jsonl` 提炼启动、恢复、overwrite、await-keyframe 模式，并将其映射为测试场景。
- 组帧测试夹具改为真实 RTP H264 包序列，并显式追加 follow-up packet 刷新 `SampleBuilder`，避免伪场景。
- 解码集成测试不再只验证内部状态，而是验证输入帧、恢复行为与渲染可见结果的闭环。
- 进一步把 pacer / renderer actor 拉进同一条测试链，确认恢复段的 `latestSlotOverwrite` 能从真实输入一路投影到 runtime stats。
- `source` / `decode` 相关用例改为复用同一套共享 fixture，避免后续继续在多个测试模块内复制 RTP payload 和 harness 逻辑。
- 在 `source` 新增“repair 补洞成功成帧”和“frame boundary 未闭合不得误出帧”两类边界，把黑盒目标从理想化乱序容忍收敛到更符合真实远端行为的恢复语义。
- 在 `renderer` 新增“持续 overwrite 直到 ack 才恢复 nominal”边界，覆盖 trace 中 `latest-overwrite -> nominal` 短窗振荡对应的稳态约束。
- 结合 `runtime-trace-1775319678083.jsonl` 与 `runtime-trace-1775310674617.jsonl` 的启动卡死样本，把“`remoteTrackAttached` 已到、但 viewport/host 还未 ready”从 steady/idle 故障中剥离，改为受首帧前 bad window 保护；同一轮还把 `cooldownSuppressed` 的短窗抑制要求改成“必须能在持续 zero-progress 下升级退出”，避免日志中的 suppression 自旋。

## Validation

- `cargo test -p xbxengine transport::rtc::stream::video_source::sink -- --nocapture`
- `cargo test -p xbxengine transport::rtc::stream::video_source::source -- --nocapture`
- `cargo test -p xbxengine media::video::decode::video_decode -- --nocapture`
- `cargo test -p xbxengine media::video::render::renderer -- --nocapture`
- `cargo test -p xbxengine media::video::render::actor -- --nocapture`
- `cargo test -p xbxengine media::video::pacer::actor -- --nocapture`
- `cargo test -p xbxengine transport::rtc::session::policy -- --nocapture`
- `cargo test -p xbxengine transport::rtc::recovery::coordinator -- --nocapture`
- `cargo test -p xbxrc mods::native_video::scheduling -- --nocapture`
- `cargo check -p xbxengine`
- `cargo check -p xbxrc`

## Risks

- 当前 source 层黑盒测试仍位于模块内，断言虽已行为导向，但尚未形成独立日志回放 harness。
- 当前 replay 仍以内嵌样本构造真实包序列，共享资产解决了样本复用问题，但还没有把运行日志中的原始包流沉淀成 trace-backed fixture。
- viewport 迟到路径当前只锁住了 `native_video` cadence 合同，还没有把 `runtime_state::sync_native_video_host_feedback` 到 core runtime stats 的完整桥接链做成回放测试。

## Follow-up

- 若后续需要更强仿真，可把 runtime trace 中的包序列抽成 fixture，建立可复用的 packet replay harness。
- 可继续把 cloud / home 两类 trace 的差异行为拆成两套 replay profile，覆盖慢反馈、recovering、starved 等更多远端模式。
- 若后续再出现“viewport 已 attach 但 surface 未 ready”导致的误判，应继续补 `src-tauri/src/mods/xbxengine/runtime_state.rs` 级别的黑盒桥接测试，把 `attach viewport -> host feedback sync -> stats projection` 全链条固化。
