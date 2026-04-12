# 首帧到 Priming 调度说明

本文只描述当前代码里已经落地的事实，不描述理想化未来方案。

目标：

1. 说明连接建立后，视频如何从“尚未首帧”进入 `priming`
2. 说明首帧阶段 `keyframe / NACK / bootstrap / owner / coordinator` 的职责分工
3. 说明这段流程里哪些条件是真正的闸门，哪些只是观测或辅助信号
4. 记录当前已知的设计边界，避免后续把不同层级职责重新搅在一起

## 1. 范围

本文覆盖的时段是：

1. WebRTC / data channel 已建立到可请求首帧
2. 首个视频 access unit 到达、检查、准入、解码、渲染
3. 从“尚未首帧”进入 `priming`
4. `priming` 内仍处于建链/恢复收口阶段的调度

本文不覆盖：

1. `steady` 期的长期播放调度
2. 首帧之后的长期 BWE 策略
3. reconnect / failed-terminal 的完整升级矩阵

## 2. 先给结论

当前主线不是“连接成功后尽量显示任何一帧”，而是：

1. 尽快索要可自举的关键帧
2. 用严格的 H.264 bootstrap / ingress 闸门拒绝坏首帧
3. 对 transport gap 先做有限 `NACK` 尝试
4. 一旦判断本地补洞不值或参考链已坏，立即切回 recovery keyframe
5. 只有拿到首个可服务输出后，系统才进入 `priming`
6. 只有 clean anchor 与 host/decode/present 证据开始闭合后，`priming` 才可能收口到后续稳态

## 3. 参与模块与主权

### 3.1 浏览器 / control 启动面

- `src/player/protocol/channels/ControlChannel.ts`
- `crates/xbxengine/core/src/transport/rtc/connection/data_channel.rs`

职责：

1. control bootstrap
2. 首次 / 延迟 keyframe prime
3. gamepad remove/add 时序

约束：

1. 这里只负责“尽快向远端表达需要 keyframe”
2. 不负责判定当前帧是否可以作为首帧解码

### 3.2 视频源与 bootstrap 判定

- `crates/xbxengine/core/src/transport/rtc/stream/video_source/source.rs`

职责：

1. 组装 sample / access unit
2. 做 H.264 inspection
3. 判定当前 AU 是 `Accept` 还是 `AwaitRecoveryKeyframe`
4. 在首帧获取窗口内发起首帧专用 keyframe 请求

约束：

1. 这里负责 decoder safety
2. 这里不直接主导昂贵恢复
3. `bootstrap_reject_reason` 只描述“当前 AU 不能自举”，不等于“该 AU 不能作为 continuation 承接”

### 3.3 Ingress 准入

- `crates/xbxengine/core/src/media/video/ingress/scheduler.rs`

职责：

1. 决定 Submit / WaitKeyframe / DropLate / DropBacklog / DropUnrecoverable / Reconfigure
2. 保证冷启动阶段只接受 clean bootstrap
3. backlog 控制与局部丢弃

约束：

1. 这里是首帧进入解码前的最后一道本地准入闸门
2. 这里只做媒体准入，不直接决定 reconnect / decoder reset

### 3.4 NACK 修复

- `crates/xbxengine/core/src/transport/rtc/stream/video_source/nack.rs`
- `crates/xbxengine/core/src/transport/rtc/stream/nack_scheduler.rs`

职责：

1. 观察 RTP gap / sample loss
2. 做包级时效与价值 admission
3. 决定 `Attempted / SkippedTooLate / SkippedLowValue / SkippedChainBroken`
4. 在链已坏时触发 keyframe 恢复

约束：

1. `NACK` 是局部 transport 修复，不是首帧主权
2. 一旦链坏，`NACK` 必须让位给 recovery keyframe

### 3.5 Owner / Session / Coordinator

- `crates/xbxengine/core/src/transport/rtc/policy/video_scheduling_owner.rs`
- `crates/xbxengine/core/src/transport/rtc/session/startup_compat.rs`
- `crates/xbxengine/core/src/transport/rtc/session/policy.rs`
- `crates/xbxengine/core/src/transport/rtc/recovery/coordinator.rs`

职责：

1. 决定当前处于 `SeekingAnchor / Priming / RebuildingSupply / ...`
2. 首帧前保护窗
3. `transportAwaitRecoveryKeyframe` 是否继续只留在本地恢复域
4. 何时允许从本地 keyframe 恢复升级到 decoder reset / reconnect

约束：

1. 首帧前允许保护，但不能无限保护
2. owner 只负责状态与 intent，不直接发送媒体修复 effect

## 4. 首帧阶段的核心事实

### 4.1 连接建立后，系统会尽快请求关键帧

事实：

1. 浏览器 runtime 的 `ControlChannel.start()` 会在 authorization flow 后主动发一次 `videoKeyframeRequested`
2. Rust-owned runtime 在 control bootstrap 完成后会挂 `delayed_keyframe_prime_due_at_ms`
3. 到期后若 control ready，则立刻补发 keyframe prime；否则先记待回放请求

设计意图：

1. 尽量缩短连接后到首帧的空窗
2. 避免因为 control channel 时序略慢而丢掉第一轮关键帧请求

### 4.2 首帧不是“收到第一帧包”就算成功

当前系统把以下几件事严格区分开：

1. `media.videoReady`
2. `latest_video_packet_arrival_time_ms`
3. `latest_video_decode_ok_time_ms`
4. `latest_video_host_present_time_ms`

含义：

1. `media.videoReady` 只表示视频协商完成并拿到了尺寸，不表示首帧已解码/渲染
2. `latest_video_packet_arrival_time_ms` 只表示媒体 ingress 有包
3. `latest_video_decode_ok_time_ms` 才说明已有帧成功出了解码
4. `latest_video_host_present_time_ms` 才说明宿主真正 present 过画面

因此“首帧已到达”的事实，代码上更接近：

1. 已有 decode 或 host present 证据
2. 或 owner / runtime snapshot 里的 first-frame 相关边界条件已被解除

### 4.3 冷启动首帧默认必须是 clean bootstrap

`VideoIngress` 初始态是 `waiting_keyframe = true`。

这意味着：

1. 首帧冷启动默认只接受 `bootstrap_ready` 的 H.264 access unit
2. 若 inspection 判定当前 AU 不具备 clean bootstrap 条件，则不会继续喂解码器
3. 只有恢复阶段，在“committed SPS/PPS + continuation ready”前提下，才允许 continuation 抢先退锁

这是当前设计最关键的安全边界之一。

## 5. bootstrap 无效时的实际处理

### 5.1 哪些情况会被视为无效 bootstrap

当前主线里，以下 reject reason 会把冷启动首帧判成无效：

1. `bootstrapMissingSps`
2. `bootstrapMissingPps`
3. `NonIdrVcl`
4. `inspectionRejectInvalidSliceHeader`

### 5.2 无效 bootstrap 的处理顺序

当前事实不是“先解码试试看”，而是：

1. `source.rs` inspection 返回 `AwaitRecoveryKeyframe`
2. 时间线写入 `frame-inspection-rejected-await-keyframe` 或相关恢复事件
3. `waiting_for_recovery_keyframe` 置真
4. 首帧获取窗口内最多两次首帧专用 keyframe 请求继续推进
5. 该 AU 不进入 decoder 主线

### 5.3 continuation 的特殊语义

需要特别注意：

1. `bootstrap_reject_reason` 只表示“这帧本身不适合作为 bootstrap anchor”
2. 它不自动等于“这帧不能作为 continuation”
3. 若 `admission_accepted == true` 且已有 committed SPS/PPS + continuation ready，则上层恢复逻辑不能把它直接当成“无效 keyframe 响应”

这是 2026-04-12 这轮回归里明确修复过的条件冲突。

## 6. 首帧获取窗口

### 6.1 首帧获取窗口何时有效

当前跨 `startup_compat / owner / coordinator` 的统一条件大致是：

1. transport 已 `Connected`
2. video track 已 `remoteTrackAttached`
3. 已观察到 video bytes
4. 还没有 decode ok
5. 还没有 host present
6. 距 `first_video_packet_arrival_time_ms` 尚未超过 `pre_first_frame_reconnect_fallback_ms`

### 6.2 这个窗口的设计目的

目的不是“永远压住恢复升级”，而是：

1. 首帧还没建立前，优先让本地 keyframe probe / local recovery 先跑
2. 避免首个 `bootstrapMissingSps/Pps/NonIdrVcl` 马上被放大成 decoder reset 或 reconnect
3. 但超过 fallback 窗口后，必须允许昂贵恢复重新进入候选

### 6.3 当前代码里的关键修正

2026-04-12 之后，这个窗口已经不再是无限期：

1. `session/startup_compat.rs` 现在显式使用 `first_video_packet_arrival_time_ms`
2. `coordinator.rs` 的 pre-first-frame hold 也对齐到 runtime recovery profile 里的 fallback 窗口
3. 若没有 `first_video_packet_arrival_time_ms`，默认不视为“可无限保护”

## 7. NACK 在首帧到 Priming 里的角色

### 7.1 NACK 不是首帧主权

`NACK` 的职责是：

1. 修复 RTP gap / sample loss
2. 尝试把仍有价值的包补回来

它不负责：

1. 决定一个坏 bootstrap 帧是否能强行成为首帧
2. 长时间替代 recovery keyframe

### 7.2 什么时候先尝试 NACK

在以下场景里，系统会先给 transport repair 一次机会：

1. 首帧建链期存在 forward gap
2. sample loss 仍有 repairability
3. 当前包的价值仍高于 admission 下界
4. 预计到达时间仍赶得上 deadline

建链期 `Startup` repair phase 对 `reference/supply` 会比 steady 更宽容。

### 7.3 什么时候 NACK 让位给 keyframe

当前主线里，一旦出现以下情况，`NACK` 会很快退位：

1. `SkippedChainBroken`
2. `referenceChainUnrecoverable`
3. `estimatedArrivalPastDeadline`
4. 已处于 `waiting_for_recovery_keyframe`
5. 当前 gap 已被视作 chain broken

一旦链坏，`maybe_trigger_reference_chain_recovery()` 会：

1. flush 掉 pending 的非关键帧 NACK
2. 写入 chain-broken 时间线事实
3. 转成 soft 或 hard recovery keyframe request

## 8. 从首帧成功到进入 Priming

### 8.1 首帧真正推进的链路

当前事实链路是：

1. `source` 通过 inspection
2. `ingress` 允许提交
3. decode actor 成功输出 decoded frame
4. renderer actor 提交到 latest render slot
5. runtime sync 取走 latest render frame
6. host bridge `present_frame`
7. `acknowledge_latest_render_frame`
8. host cadence telemetry 更新 `latest_video_host_present_time_ms / video_present_epoch`

### 8.2 Priming 的含义

`Priming` 不是 steady，也不是“还没首帧”。

它表示：

1. 已经至少开始形成可服务的媒体输出
2. 但 clean anchor、present cadence、supply continuity 还没完全稳定
3. owner 仍可能因为 `transportAwaitRecoveryKeyframe / ingress waiting / rebuilding-supply` 等证据保持保守

### 8.3 Owner 为什么可能还停在 Priming

只要以下任一条件仍成立，owner 就不应把状态过早当成已稳定：

1. 仍在 `ingress_waiting_keyframe`
2. 仍缺 clean anchor
3. host present 还没有持续更新
4. decode / present freshness 仍不足
5. timeline 还停留在 gap 修复或恢复保活阶段

## 9. Priming 内的调度

### 9.1 Priming 的调度目标

不是“已经有首帧就立刻按 steady 规则处理”，而是：

1. 先保证 supply 不断
2. 继续观察 clean anchor 是否闭合
3. 允许 transport repair 与 recovery keyframe 并存
4. 避免把短暂 gap 噪声过早升级成昂贵恢复

### 9.2 Priming 期 owner 的关键判断

owner 主要看三组证据：

1. anchor 侧：是否仍在等恢复关键帧，是否已有 clean anchor
2. supply 侧：present / decode 是否足够新鲜，renderer 是否 stalled
3. timeline 侧：是否已有 `frame-complete-candidate / frame-observed / gap-repair-in-flight / gap-resolved`

2026-04-12 之后，首帧前保护不再过度依赖单一 `source_event`：

1. 显式 wait-keyframe 事件可触发保护
2. `is_ingress_waiting_keyframe(...)` 为真时也保持保护
3. `gap-repair-in-flight` 这类建链期 transport 事件不再导致首帧优先保护意外失效

### 9.3 Priming 何时可能退出

虽然本文不展开 steady 期，但可以明确：

1. 仅有首帧 decode 不足以退出 priming
2. clean anchor、host present、新鲜 decode/present、timeline progress 必须开始闭合
3. owner 需要确认当前不再是“等待恢复关键帧”或“建链期保护窗”主导

## 10. 当前代码约束与维护要求

后续继续维护这段逻辑时，必须守住以下边界：

1. `bootstrap_reject_reason` 不是 continuation 不可用的同义词
2. 首帧前保护窗必须有上限，不能回到无限压制
3. `NACK` 不得长期替代 recovery keyframe
4. owner 的首帧保护不能只绑死在某几个具体 `source_event`
5. `media.videoReady` 不得被误当作“首帧已出图”
6. host present 仍是首帧真正落地的权威证据

## 11. 建议阅读顺序

如果要继续看代码，建议按下面顺序：

1. `crates/xbxengine/core/src/transport/rtc/stream/video_source/source.rs`
2. `crates/xbxengine/core/src/media/video/ingress/scheduler.rs`
3. `crates/xbxengine/core/src/transport/rtc/stream/video_source/nack.rs`
4. `crates/xbxengine/core/src/transport/rtc/session/startup_compat.rs`
5. `crates/xbxengine/core/src/transport/rtc/policy/video_scheduling_owner.rs`
6. `crates/xbxengine/core/src/transport/rtc/recovery/coordinator.rs`
7. `crates/xbxengine/core/src/api/runtime/sync.rs`
8. `src-tauri/src/mods/native_video/presenters.rs`

## 12. 对应事实来源

本文对应的代码与回归主要来自：

1. `source / ingress / nack / owner / coordinator / session policy` 当前主线实现
2. 2026-04-12 这轮关于首帧前保护窗、invalid bootstrap continuation、owner 首帧保护事件面的修正与回归
3. 当前 `xbxengine` 单测中 `first_frame_acquisition`、`transport_await_invalid`、`pre_first_frame_*` 相关用例
