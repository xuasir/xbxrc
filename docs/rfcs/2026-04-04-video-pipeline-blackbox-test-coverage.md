# Video Pipeline Blackbox Test Coverage RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 已完成
- Current State: completed
- Owner: TBD
- Last Updated: 2026-04-09

## Background

- 当前流媒体测试更多集中在局部状态机、统计投影与定点回归，尚未围绕“接包 -> 组帧 -> 解码 -> 渲染”建立统一的黑盒测试矩阵。
- 用户要求测试用例必须基于功能目标与远端真实行为，而不是顺着实现细节倒推，因此需要先读取历史 runtime trace，提炼正常路径和异常边界，再把这些行为转成可执行测试。
- 这轮读取了 `runtime-logs/runtime-trace-1775289054327.jsonl`、`runtime-logs/runtime-trace-1775292592042.jsonl` 等样本，确认了以下高价值模式：
  - 启动阶段先看到 `videoTrackState=remoteTrackAttached`，随后才逐步出现稳定 `hostPresentState.presentSubmitCountTotal` 增长。
  - 恢复段会出现 `renderCandidateStateTransition=latest-overwrite -> nominal` 的短窗振荡。
  - 恢复失败或 bootstrap 条件不足时，会出现 `decoderRecoveryStateChanged` / `AwaitRecoveryKeyframe` 相关信号。

## Goal

- 为视频链路补齐基于真实运行行为抽象出来的黑盒测试。
- 覆盖接包归一化、组帧 admission、解码恢复、渲染 latest-slot 压力恢复四个关键阶段。
- 增加至少一组跨阶段集成测试，验证输入场景到输出行为的闭环，而不依赖内部实现步骤。

## Scope

- In scope:
  - `crates/xbxengine/core/src/transport/rtc/stream/video_source/sink.rs`
  - `crates/xbxengine/core/src/transport/rtc/stream/video_source/source.rs`
  - `crates/xbxengine/core/src/media/video/decode/video_decode.rs`
  - `crates/xbxengine/core/src/media/video/render/renderer.rs`
  - `crates/xbxengine/core/src/transport/rtc/session/policy.rs`
  - `crates/xbxengine/core/src/transport/rtc/recovery/coordinator.rs`
  - `src-tauri/src/mods/native_video/scheduling.rs`
  - `docs/project-task.md`
- Out of scope:
  - 新增线上埋点
  - 改动产品运行时主流程
  - 重写既有白盒测试，只在缺口处补黑盒场景

## Plan

1. 基于历史日志抽象远端正常路径与异常边界，形成测试矩阵。
2. 在接包/组帧层补黑盒场景，验证 RTP 归一化、bootstrap 组帧成功与 bootstrap 缺失拒收。
3. 在解码/渲染层补场景驱动测试，并增加跨阶段集成测试验证恢复与 latest-slot 行为。

## Validation

- [x] `cargo test -p xbxengine transport::rtc::stream::video_source::sink -- --nocapture`
- [x] `cargo test -p xbxengine transport::rtc::stream::video_source::source -- --nocapture`
- [x] `cargo test -p xbxengine media::video::decode::video_decode -- --nocapture`
- [x] `cargo test -p xbxengine media::video::render::renderer -- --nocapture`
- [x] `cargo test -p xbxengine media::video::render::actor -- --nocapture`
- [x] `cargo test -p xbxengine media::video::pacer::actor -- --nocapture`
- [x] `cargo test -p xbxengine transport::rtc::session::policy -- --nocapture`
- [x] `cargo test -p xbxengine transport::rtc::recovery::coordinator -- --nocapture`
- [x] `cargo test -p xbxrc mods::native_video::scheduling -- --nocapture`
- [x] `cargo check -p xbxengine`
- [x] `cargo check -p xbxrc`

## Risks

- `rtc_media::SampleBuilder<H264Packet>` 对 RTP H264 负载格式有严格要求，测试夹具若不贴近真实 RTP 包格式，容易产生伪失败。
- 当前 source 层不直接暴露完整外部 harness，黑盒测试仍需放在模块内实现，但断言必须保持行为导向而不是内部状态导向。

## Progress

- [x] Step 1: 已读取历史 trace，并提炼启动、恢复、overwrite、await-keyframe 等关键行为模式
- [x] Step 2: 已补齐接包归一化与真实 RTP 组帧黑盒测试
- [x] Step 3: 已补齐解码/渲染及跨阶段集成测试

## Execution Notes

- Date: 2026-04-04 | Status: completed
- Update: 建立黑盒测试目标，确认以 `runtime-trace-1775289054327.jsonl` 和 `runtime-trace-1775292592042.jsonl` 作为本轮主要行为样本。
- Decision: 测试不从代码分支倒推，而是围绕“clean bootstrap 成帧”、“缺参数集拒收并等待恢复 keyframe”、“恢复段 overwrite 后回到 nominal”这三类运行态行为展开。
- Risk/Blocker: 子代理两次都因 429 失败，本轮由主窗口直接收敛方案与实施。
- Date: 2026-04-04 | Status: completed
- Update: 在 `video_source/sink.rs` 新增 primary passthrough、非 RTX repair 丢弃、truncated RTX 丢弃；在 `video_source/source.rs` 新增真实 RTP bootstrap 组帧和缺参数集 await-keyframe 场景；在 `video_decode.rs` 新增 `assembled -> decode -> render` 闭环测试与 backend failure 后恢复到 nominal 的集成测试。
- Decision: `SampleBuilder<H264Packet>` 需要依靠“下一时间戳包”刷新前一帧，因此组帧测试夹具显式追加 follow-up RTP 包来模拟真实远端持续推流。
- Risk/Blocker: 当前验证通过，但渲染层仍主要依赖既有 latest-slot 回归；若后续需要更强端到端覆盖，可继续补 pacer/renderer actor 层的日志回放型场景。
- Date: 2026-04-04 | Status: completed
- Update: 进一步补齐 actor 级覆盖：`render/actor.rs` 现在验证 `latestSlotOverwrite` 与 `presentError` 的 runtime stats 投影；`video_decode.rs` 新增 `RTP -> source -> decode -> pacer -> renderer` 端到端测试，覆盖真实包序列驱动下的 overwrite 恢复段。
- Decision: 端到端用例采用真实 RTP H264 AU 序列驱动 `RtcVideoFrameSource`，并通过真实 `PacerActorHandle + RendererActorHandle` 完成最后两段链路，避免“组帧完成后直接伪造渲染结果”的测试空洞。
- Risk/Blocker: 目前尚未把真实 runtime trace 原始包序列固化为外部 fixture 文件；当前 replay 仍是代码内嵌样本，但行为目标已经闭环。
- Date: 2026-04-04 | Status: completed
- Update: 已把 bootstrap RTP/H264 样本、发送 helper 与 `RtcVideoFrameSource` harness 收敛进 `crates/xbxengine/core/src/media/video/test_fixtures.rs`，并让 `source` / `decode` 相关黑盒测试统一消费同一套资产。
- Decision: 鉴于 runtime trace 只记录行为观测而不含原始 RTP payload，本轮“资产化”不强行伪造日志回放文件，而是把已验证过的真实包序列收口为共享 fixture，先解决样本复用、一致性与后续扩展成本问题。
- Risk/Blocker: 当前共享资产仍以代码模块存在，还未演进成独立 replay profile；若后续拿到原始包流，再考虑升级成 trace-backed packet fixture。
- Date: 2026-04-05 | Status: completed
- Update: 继续补齐缺失边界，新增 `repair packet closes bootstrap gap and allows frame assembly`、`bootstrap packets without follow-up boundary do not emit partial frame`、`render candidate state stays latest-overwrite until latest slot is acknowledged` 等黑盒回归，覆盖 repair 补洞、未闭合 frame boundary、防止 overwrite 误恢复这三类高价值边界。
- Decision: 不再假设“主路任意乱序包都应直接成帧”；结合 trace 中的 `AwaitRecoveryKeyframe` / `streamIdleTimeout` / overwrite 短窗振荡，边界优先收敛到更真实的远端行为目标：缺口依赖 repair/RTX 补齐、frame boundary 未闭合时禁止误出帧、latest slot 未 ack 前不得提前回到 nominal。
- Risk/Blocker: 目前仍未覆盖 FU-A 分片截断、连续重复包、长时间连续 recover keyframe 不完整等更细颗粒边界；这些场景后续可继续在共享 fixture 基础上扩展。
- Date: 2026-04-05 | Status: completed
- Update: 基于 `runtime-trace-1775319678083.jsonl` 与 `runtime-trace-1775310674617.jsonl` 暴露的主故障面，继续把黑盒覆盖扩到“启动卡死 / cooldownSuppressed / transportAwaitRecoveryKeyframe / viewport 时序”四组系统合同：在 `session/policy.rs` 新增 `Connected + remoteTrackAttached + 无 host/decode 输出` 时首帧前 bad window 内不得因 `adapterIdleTimeout` 误入恢复链、窗口到期后必须升级；在 `recovery/coordinator.rs` 新增 `cooldownSuppressed` 不能在 `Connected + track attached + 0 present/decode progress` 场景下无限续命；在 `native_video/scheduling.rs` 新增“只有 display tick、没有 first present 时，即使连续 no-pending 也必须保持 priming”。
- Decision: 这轮不再把问题归咎于媒体局部接包/组帧，而是把最新 trace 揭示的 4 类系统行为合同直接资产化为黑盒断言：启动期不把 viewport 未就绪误判成 idle，cooldown 必须有出口，`transportAwaitRecoveryAnchor` 的升级必须受坏窗约束，host cadence 在 first present 前只能处于 `priming`。
- Risk/Blocker: 目前 viewport 时序覆盖先落在 `native_video/scheduling.rs`，还没有把完整 `runtime_state::sync_native_video_host_feedback` 注入路径做成集成回放；如果后续 trace 继续暴露 `surface ready / viewport attach / host feedback` 注入口径漂移，还需要再补 tauri runtime 层更靠近宿主桥接的黑盒测试。
