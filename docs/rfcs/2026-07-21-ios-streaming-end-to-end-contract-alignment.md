# iOS 串流端到端合同对齐 RFC

> 本 RFC 以桌面主线的共享 Rust 领域合同为权威，覆盖 iOS xCloud 与 xHome。实施按合同和证据推进，完成后产出对应 Report。

## Status

- Completion: 实现完成，外部验收待完成
- Current State: validation-blocked
- Owner: agent
- Last Updated: 2026-07-21
- Execution Gate: 用户已确认完整实现 RFC

## Background

- iOS 已具备统一 `create_stream_session + SessionFlowService + SessionScheduler` 会话骨架、libwebrtc 音视频接收、四条 DataChannel 创建与基础 bootstrap。
- 现有 `2026-07-16-ios-libwebrtc-streaming-runtime` RFC 将输入、震动、触摸和键鼠留在后续范围，真实账号 offer/answer/ICE、首帧与音频 trace 仍待验收。
- 当前目标要求本地和云端串流保持端到端真正对齐，并由凭证生命周期、xHome 上下文、四通道协议、输入/震动、终态和验证证据共同证明。

## Goal

1. 固化桌面权威实现的 token/会话、WebRTC、DataChannel/控制三组合同。
2. 让 iOS xCloud/xHome 复用共享 Rust 决策和协议构造，只保留 libwebrtc、GameController、系统音视频与 Swift 生命周期适配。
3. 为所有关键状态、消息序列、资源终态和错误分支建立确定性测试。
4. 形成源码合同、自动测试、构建、trace gate 与真实设备证据相互印证的最终审计包。

## Scope

### In Scope

- `crates/xbox-auth-flow`、`crates/xbox-streaming`、`crates/xbox-ios-bridge` 的 token、access lease、session flow 与 xHome context 适配。
- `crates/xbxengine/protocol` 与 `crates/xbxengine/core` 的 DataChannel/input/rumble 纯协议边界和共享 fixtures。
- `iosapp/XBXRC/Platform/Streaming` 的 libwebrtc plan 投影、DataChannel state、GameController/haptics、终态与 trace。
- `iosapp/XBXRCTests`、Rust 单元测试、SDP/packet golden fixtures、trace analyzer 与构建/链接门禁。
- xCloud 和 xHome 两条真实设备运行 trace，包含本地与云端目标身份、会话、媒体和控制证据。

### Out Of Scope

- Electron、React、Flutter 或第二套 native runtime/bridge。
- 麦克风采集、party chat 媒体上行和语音 UX；本 RFC 只验证 chat DataChannel 的协议生命周期。
- 自定义 Metal 后处理、HDR、HEVC/AV1 和独立媒体 pipeline。
- Xbox 服务端协议的产品语义改造；客户端按桌面权威合同投影并验证。

## Alignment Definition

“对齐”分为两层：

- 共享合同一致：token 解析、session plan、远端会话 flow、DataChannel profile、消息 payload 和终态分类由共享 Rust 类型承载；桌面 Rust 作为输入/震动 wire authority，iOS 用独立 Swift 纯类型实现同一字节和语义合同。
- 平台行为等价：桌面 Rust RTC 与 iOS libwebrtc 在相同远端事实下达到相同的协商结果、控制序列、输入/震动语义、关闭终态和诊断证据。平台库内部机制通过 SDP/RTCStats/事件与 trace 证明。

### Authority Precedence

1. `xbox-streaming::Plan` 是 target、session、negotiation、runtime、input 和 render 决策的一级权威。
2. 桌面 WebRtcDirect 是 iOS libwebrtc 的 SDP、transceiver、codec preference、ICE adapter 与连接状态投影权威。
3. 桌面 Rust-owned RTC 是 TWCC/NACK/PLI/FIR/REMB/GCC、receive recovery、media supply、display health 和 terminal evidence 的目标权威。
4. `xbxengine-protocol` 是四通道 bootstrap 与 remote terminal 分类的跨平台协议参考；iOS input bytes 与 rumble bytes 使用独立 Swift 实现，通过相同 wire fixtures 验证协议等价。
5. iOS 平台 API 只负责实现上述 plan/protocol 的可观察等价行为；libwebrtc 内部反馈机制通过 SDP 与 RTCStats 验证，保持单一 Apple RTC runtime。

## Authoritative Contracts

### A. Token / Session

权威入口：

- `src-tauri/src/mods/auth/token_policy.rs`：user/web/xCloud/xHome token 有效性与 app level。
- `crates/xbox-auth-flow`：登录、refresh/finalize、streaming token 与 transfer token。
- `crates/xbox-streaming/src/session/*`：`SessionFlowService`、`SessionScheduler`、mutation gate、generation-safe store、keepalive、close 与 session terminal。
- `crates/xbox-streaming/src/policy/*`：target、headers、session settings、region、resolution、runtime/negotiation/input plan。

合同要求：

1. Keychain 只持久化恢复登录所需的 refresh token、seed 和 web token；streaming gsToken、transfer token、access handle、remote session id 保持短生命周期。
2. `prepare_cloud_access` 与 `prepare_home_access` 每次通过 refresh/finalize 获取新 streaming tokens，并原子保存刷新后的 `AuthSession`。
3. access handle 绑定 target、region host、account、refresh token、`SessionAccessContext` 与 force region；handle 释放后立即失效，跨账号、跨 target 和过期 generation 均被拒绝。
4. xCloud 使用 `streamTitleId`；xHome 使用主机 `serverId -> id -> deviceId` 的串流身份优先级。target 由 access context 权威携带。
5. cloud/home 共用 `SessionFlowService + SessionScheduler`：create、ReadyToConnect transfer token、SessionReady、offer/answer、ICE、keepalive、connected、close。
6. session mutation 服从单 generation 串行化；旧 monitor、旧 ICE、旧 close 无法覆盖或复活新 session。
7. xHome context 覆盖主机远程管理/串流能力、开机与注册等待、console addresses、home resolution、region 与 TURN/ICE 输入。

当前差距：

- M1 已完成：access registry 使用 UUID scoped lease，绑定 account/target/owner generation/expiry/revocation；xHome capability、console addresses、wake/registration、resolution 与 fallback TURN 进入 Rust plan；Swift 终态和 cleanup 使用同一 attemptId/generation/operationId。
- 外部证据仍缺：真实账号的 refresh/rotation 与真实 xHome 主机 trace，需要设备和服务环境采集后通过 streaming analyzer gate。

### B. WebRTC

权威入口：

- `crates/xbox-streaming/src/policy/negotiation/compiler.rs`：codec、bitrate、audio channel、IPv6、end-of-candidates、xHome console address 注入。
- `crates/xbox-streaming/src/policy/runtime/compiler.rs`：runtime owner、TURN、BWE、TWCC/GCC、REMB、NACK/jitter/recovery 参数。
- `crates/xbxengine/core/src/transport/rtc/*`：SDP、ICE、RTCP feedback、receive recovery、latest-only display 与 terminal evidence。

合同要求：

1. iOS 的 offer 由共享 `Plan` 投影：H.264 profile/packetization、Opus/stereo、audio sendrecv、video recvonly、bitrate、帧尺寸、IPv6/ICE 与 target profile 可审计；audio sendrecv 保持协商能力，麦克风采集和本地 audio track 在本 RFC 内保持关闭。
2. 本地 ICE candidate、completion、远端去重、end-of-candidates 和 30 秒应用窗口保持单 generation；Rust `a=candidate:` 在 Swift → M137 native 边界适配为 `candidate:`，异常 `UDP + tcptype` candidate 在该边界过滤。
3. xHome console addresses 与显式/fallback TURN 进入 ICE plan；cloud/home 均保留 Xbox STUN。
4. iOS libwebrtc 启用标准 TWCC、NACK、PLI/FIR、REMB/GCC 能力，并通过 offer/answer SDP 与 RTCStats 证明实际协商结果。
5. WebRTC connection、ICE、signaling、DataChannel、audio track、video track、首帧和 stats 使用同一 attempt/generation 串联。
6. 首帧、steady media supply、decode/render、帧供给 delta、freeze、jitter、RTT、packets lost、NACK/PLI/FIR 与 submit/present 长尾进入 trace 审计。
7. failed/disconnected/closed、远端 session terminal、用户停止、后台策略和登录退出均收敛为单次 cleanup。

当前差距：

- M2 已完成：iOS 消费 Rust `XboxWebRtcPlan`，配置 audio sendrecv/video recvonly、H.264 profile/packetization/level asymmetry、bitrate/fmtp、Opus stereo、标准 RTCP feedback、ICE context/candidate policy、同 peer ICE restart 与失败后的 peer rebuild；remote ICE native adapter 对齐桌面边界；RTC/display health 已按 production key snapshot 投影。
- 外部证据仍缺：真实 offer/answer SDP、selected candidate pair、首帧/steady media 和设备 build，需要真实 WebRTC 运行 trace 与 Xcode Device build。

### C. DataChannel / Control

权威入口：

- `crates/xbxengine/protocol/src/stream_data_channel.rs`：四通道 profile 与共享 bootstrap payload。
- `crates/xbxengine/core/src/transport/rtc/connection/data_channel.rs`：通道状态机、握手、post-handshake、control replay、输入背压与 inbound message。
- `crates/xbxengine/core/src/transport/rtc/connection/rumble.rs`：Better xCloud/legacy rumble 二进制解析。
- `ohmygamepad-protocol`：逻辑手柄与四路震动语义。

合同要求：

1. 四通道固定为 `input/1.0`、`control/controlV1`、`chat/chatV1`、`message/messageV1`，ordered=true。
2. message open 后每个 peerEpoch 发送一次 Handshake；收到 HandshakeAck 后发送一次六条 post-handshake message。
3. control/input open 后执行 pre-handshake bootstrap；HandshakeAck 后按权威顺序幂等 replay control/input。每个阶段独立至多成功一次，发送失败保留阶段内重试能力。
4. input 数据包遵循桌面 Rust codec 的 wire layout；iOS 独立 Swift codec 覆盖 sequence、timestamp、按钮、摇杆、扳机、D-pad、neutral frame、背压与断连停止，不引入 Rust 手柄库或 UniFFI 手柄 DTO。
5. inbound input 二进制解析 Better xCloud 与 legacy rumble，保留 low/high frequency motor 与 left/right trigger 四路语义。
6. iOS 使用 `GCController` 采样物理手柄并通过 `GCHapticsLocality`/`GCDeviceHaptics` 执行震动；连接切换、能力降级、停止震动和 session cleanup 可确定性验证。
7. message channel 解析远端 kick/closed/error，立即建立权威 terminal reason 并终止恢复、输入和震动。
8. chat 通道保持协议 ready 与可观测状态；麦克风/party chat 的媒体能力继续单独立项。

当前差距：

- M3-M5 已完成：四通道状态机支持任意 open 顺序、Handshake/Ack 幂等、pre-handshake 预发、Ack 后 replay、分阶段幂等和失败重试；Ack 后 control replay 完成产生 `controlReady`，`.playing` 由 peer connected、control ready 与首帧共同决定；Swift 独立 input/rumble/haptics 实现覆盖 neutral、backpressure、四路 locality、降级和 cleanup；remote kick/closed/error 进入 typed terminal。
- 外部证据仍缺：完整 XCTest target build、真实 haptics controller 设备结果和 streaming trace analyzer gate。

## Contract Matrix

| ID | Contract | Desktop authority | iOS current state | Required proof | Owner |
| --- | --- | --- | --- | --- | --- |
| AUTH-01 | user/web/stream token expiry and refresh skew | `src-tauri/src/mods/auth/token_policy.rs` | Rust bridge refresh/finalize; Swift Keychain restore | expiry fixtures, refresh rotation, redaction scan | `xbox-auth-flow`, `xbox-ios-bridge` |
| AUTH-02 | scoped access lease and revocation | Tauri auth/session provider | UUID scoped lease with generation/expiry/release | lease state tests, stale/cross-target rejection, release pairing | `xbox-ios-bridge` |
| AUTH-03 | cloud/home target identity | Tauri streaming service + host service | cloud title ID and home `serverId -> id -> deviceId` projection | identity fixture and plan snapshot | iOS bridge + Swift clients |
| AUTH-04 | xHome readiness context | `SessionFlowProvider::get_remote_consoles` and policy context | capability/wake/registration/address/TURN context in Rust plan | wake/registration/capability/address/TURN fixture | `xbox-ios-bridge` |
| SESS-01 | canonical session progression | `SessionFlowService + SessionScheduler` | shared Rust flow adapter | fake provider sequence and transfer-token-once assertion | `xbox-streaming` |
| SESS-02 | generation-safe mutation and close | session store/mutation gate | Rust generation plus Swift actor generation | stale callback/recreate/close ordering tests | Rust + Swift |
| RTC-01 | offer direction | `WebRtcTransport`: audio sendrecv, video recvonly | audio sendrecv, video recvonly | SDP direction verifier | `LibWebRtcRuntime.swift` |
| RTC-02 | H.264 profile/packetization/bitrate | `SdpManipulator`, Rust builder and policy compiler | typed profile/packetization/bitrate/fmtp projection | offer/answer SDP fixture and negotiated profile | Swift runtime + shared plan |
| RTC-03 | ICE server/context and candidate policy | browser ICE policy + Rust builder | Rust ICE context plus Swift deterministic candidate ordering/restart | candidate ordering, address-family and restart fixtures | Rust + Swift |
| RTC-04 | TWCC/NACK/PLI/FIR/REMB/GCC | Rust interceptor, feedback arbiter and BWE policy | SDP feedback projection plus RTCStats fields | SDP ext/feedback verifier plus stats gate | Swift runtime |
| RTC-05 | media and terminal evidence | browser/Rust runtime stats and lifecycle | RTC/display health snapshots and typed terminal cleanup | trace fields for supply, recovery, present and terminal | Swift trace + analyzer |
| DC-01 | four channel profile | shared `stream_data_channel.rs` and Rust bootstrap | Rust profile projection and Swift creation snapshot | profile and creation fixture | shared protocol + Swift |
| DC-02 | handshake/bootstrap idempotency | Rust data channel state | order-independent state machine with retry and typed terminal | open-order/replay/send-failure virtual clock tests | Swift adapter |
| DC-03 | input packet codec | Rust `data_channel_state.rs` 作为 wire 参考 | iOS 独立 Swift 38-byte codec，未依赖 Rust 手柄库 | exact golden bytes、neutral/backpressure、Swift 6 typecheck | iOS streaming |
| DC-04 | rumble protocol | Rust `rumble.rs` 作为 wire 参考 | iOS 独立 Swift parser、四路 haptics locality 与降级 | Better-xCloud/legacy four-motor、stop、locality fallback fixtures | iOS streaming |
| DC-05 | remote terminal message | Rust message catalog/lifecycle | typed kick/closed/error classifier and terminal reason | typed terminal reason and one cleanup test | Swift + Rust |
| TERM-01 | terminal taxonomy and cleanup | browser lifecycle + Rust recovery lifecycle | disconnected/failed/closed/recovering 分流，单次固定 cleanup | terminal source matrix and release ordering | Swift actor |
| TRACE-01 | operationId and sensitive-field gate | desktop runtime trace schema | attempt/generation/operationId canonical anchors and redacted health snapshots | paired event analyzer, token/handle scan | iOS diagnostics |

## Implementation Plan

### M0. Contract Freeze

1. 将三组合同整理成代码可消费的 typed records/enums 与 fixture。
2. 以 `AUTH-*`、`SESS-*`、`RTC-*`、`DC-*`、`TERM-*`、`TRACE-*` 矩阵绑定 owner、代码入口、测试和 trace evidence。
3. 为现有 iOS runtime 增加 characterization tests，锁定当前行为、已有输入/rumble 实现和缺口。

### M1. Credential And Session Lifecycle

1. 将 access handle 收口为账户代际内可复用、可撤销的 scoped lease；记录 owner generation、target、created/expires/released 状态，允许云目录分页复用，串流启动共享同一代际但不跨代际。
2. 固化 prepare/persist/start/close/release 配对与错误清理，覆盖 app restore、refresh、logout、region change、并发启动和 stale callback。
3. 扩展 iOS session provider，消费 xHome 主机能力、console addresses、home resolution 与 TURN context。
4. 投影稳定的 startup/session terminal DTO 和脱敏 trace。

### M2. WebRTC Plan Projection

1. 从共享 `Plan` 导出 iOS 可消费的 negotiation/runtime profile。
2. 用 profile 配置 audio sendrecv、video recvonly、H.264 profile/packetization、bitrate、frame constraints、ICE policy 与 target-specific context。
3. 增加 offer/answer SDP verifier 与 RTCStats collector，覆盖 TWCC/NACK/PLI/FIR/REMB/GCC 的协商/运行证据。
4. 实现 disconnected/failed/closed 分流、ICE restart 与 runtime rebuild 回退，并与共享 terminal model 对接。
5. 建立媒体供给与显示健康 trace：network、recovery、media supply、steady supply、submit/present、frame delta。

### M3. Four-Channel State Machine

1. 将 DataChannel phase、bootstrap sequence、ack、replay、terminal message 收口到可测试状态机；input bytes 与 rumble parser 在 iOS 使用独立 Swift 纯类型实现。
2. Swift adapter 只映射 `RTCDataChannelState`、发送共享 payload、投影 inbound frame。
3. 为乱序 open、重复 callback、HandshakeAck 重放、channel close/reopen、发送失败和 cleanup 建立虚拟时钟测试。

### M4. Input And Rumble

1. 在 iOS 建立独立 Swift input codec、rumble parser、send gate 与 haptics routing plan，不依赖 Rust 手柄库或 UniFFI 手柄 DTO。
2. 将现有 iOS `GCController` 采样接入 Swift codec，补齐 neutral frame、input bufferedAmount 水位与停止语义。
3. 用 Swift parser 覆盖 Better-xCloud/legacy rumble，补齐四路 rumble 到 iOS haptics locality 的映射、能力降级、coalescing、duration 和 stop。
4. 用 golden bytes、fake controller、fake haptics engine 和虚拟时钟验证输入/震动闭环。

### M5. Terminal And Audit Closure

1. 建立单一 terminal model：user stop、background stop、auth revoked、session failed/closed、remote kick、ICE failed、peer closed、channel fatal。
2. cleanup 顺序固定为停止输入/震动、取消 ICE/stats tasks、关闭 DataChannel/PeerConnection、关闭 remote session、释放 access handle。
3. 每个 attempt 只接受一个终态，所有资源释放和 trace operation 均完成配对。
4. 运行确定性测试、Device/Simulator 构建、真实 xCloud/xHome trace gate，产出最终 Report。

## Stage Exit Criteria

| Stage | Exit condition |
| --- | --- |
| M0 | 每个合同 ID 绑定一个权威实现、一个确定性测试和一个 trace/构建证据；旧 iOS 行为已记录。 |
| M1 | access lease 可撤销且按账户代际隔离；cloud/home plan 快照包含 target、host、capability、region、console address 和 TURN/ICE；session startup/terminal DTO 可脱敏序列化。 |
| M2 | xCloud/xHome 的 iOS offer 通过 direction、H.264、bitrate、feedback 和 ICE verifier；断线恢复与终态分流测试通过。 |
| M3 | 四通道在任意 open 顺序下完成 pre-handshake 与 Ack 后 replay；每阶段幂等，重复 Ack、发送失败、重连和远端 terminal 均有确定性结果。 |
| M4 | iOS 输入 bytes 与 Rust golden 完全一致；rumble 两种格式和四路语义通过 fixture；neutral、背压、haptics 降级和 stop 有测试。 |
| M5 | 所有终态只触发一次 cleanup；operationId 配对、敏感字段扫描、Rust/Swift/Xcode 门禁、xCloud/xHome 真实 trace 全部通过。 |

## Deterministic Test Matrix

| Area | Required Cases | Evidence |
| --- | --- | --- |
| Credentials | login/restore/refresh、token rotate、prepare failure、Keychain save failure、logout、region change | Rust + Swift actor tests、敏感字段扫描 |
| Access handle | target mismatch、stale generation、double consume/release、expiry、cross-account | Rust deterministic tests |
| Session | cloud/home create、queue、ReadyToConnect、transfer token once、mutation serialization、close/recreate | `xbox-streaming` fake provider tests |
| xHome | host identity、wake、registration wait、capability rejection、console addr、TURN fallback | Rust fake host/session tests |
| WebRTC | audio/video direction、H.264 profile/packetization、SDP feedback、ICE completion/dedup/restart、state transition、stats projection | Swift fake peer + SDP fixtures |
| Four channels | profile、open order、Handshake/Ack、post-handshake once、control order、reopen/close | Shared state-machine tests + Swift adapter tests |
| Input | golden packet、sequence/time、full logical pad、neutral、backpressure、disconnect | Rust golden bytes + fake GCController |
| Rumble | Better xCloud/legacy、four motors、coalesce、duration、stop、capability fallback | Rust fixtures + fake haptics |
| Terminal | every terminal source、single cleanup、stale callback rejection、resource release order | Swift actor virtual-clock tests |
| Trace | operationId pairing、terminal reason、channel/input/rumble counts、media health、redaction | iOS trace analyzer gate |

## Streaming Trace Contract

项目内 `check-streaming-core-trace.py` 复用 `analyze-ios-runtime-trace` 的 schema、sequence、privacy 与 budget 能力，按每个 attempt/generation/peerEpoch 验证核心连接、控制面和真实上屏链。`.agents` skill 目录恢复可写后再合并为 `--require-flow streaming-core`：

```text
streamLaunchStarted
-> accessPrepareStarted -> accessPrepareSucceeded|Failed
-> sessionCreateStarted -> sessionReady|sessionTerminal
-> offerStarted -> answerApplied|signalingTerminal
-> localIceStarted -> localIceCompleted
-> remoteIceBatchReceived -> remoteIceBatchApplied? -> remoteIceCompleted
-> peerConnected
-> dataChannelProfilesCreated
-> {messageHandshakeSent, controlBootstrapPreHandshakeCompleted}
-> messageHandshakeAcked
-> messagePostHandshakeCompleted -> controlBootstrapCompleted -> controlReady
-> firstVideoFrame
-> steadyMediaObserved
-> videoSurfaceAttached -> videoSurfaceSized -> videoSurfaceRendererReady
-> terminalSelected
-> inputStopped -> hapticsStopped -> peerClosed -> remoteSessionClosed -> accessReleased
```

门禁规则：

1. `streamLaunchStarted`、`terminalSelected` 与 cleanup 共享 attemptId；generation 在同一 attempt 内保持不变。
2. 每个 started 事件恰好配对一个 succeeded/failed/terminal；每个 attempt 恰好一个 `terminalSelected`。
3. 四通道的 label/protocol/ordered 快照必须完整；Handshake、post-handshake、control/input bootstrap 按阶段各至多成功一次。
4. `controlReady` 必须发生在 HandshakeAck 和 Ack 后 control replay 之后；`playing` 必须同时具备 peer connected、control ready 与 first video frame。
5. `videoSurfaceAttached`、非零 `videoSurfaceSized`、非零 `videoSurfaceRendererReady` 必须共享 attemptId/generation/peerEpoch，证明真实 Metal surface 已具备画面条件。
6. `inputFrameSent`、`inputBackpressureDrop`、`rumbleParsed`、`hapticsApplied/Degraded/Unsupported` 进入独立 `streaming-input` 验收，使用计数/摘要并禁止记录原始 payload；本轮核心串流门禁不要求物理手柄输入。
7. `rtcHealthSnapshot` 至少投影 selected candidate pair、RTT、jitter、loss、receive bitrate、frames decoded/dropped、freeze、NACK/PLI/FIR、first/last media time 与 frame supply delta。
8. `displayHealthSnapshot` 至少投影 submit/present count、last submit/present time、submit-to-present 长尾和 displayed frame delta；平台无法提供的字段显式标记 `unsupported`。
9. token、seed/JWK、access handle、account identity、title ID、完整 URL/SDP/candidate 和原始 DataChannel payload 的隐私扫描必须为零。

## Validation Gates

- [x] `cargo fmt --all -- --check`
- [x] `cargo test -p xbox-auth-flow`
- [x] `cargo test -p xbox-streaming --lib`
- [x] `cargo test -p xbxengine-protocol`
- [x] DataChannel/input/rumble 共享协议定向测试
- [x] `cargo test -p xbox-ios-bridge`
- [x] `cargo check -p xbox-ios-bridge -p xbox-streaming -p xbxengine`
- [x] UniFFI bindings 生成与差异门禁
- [x] `pnpm lint:fix`（本 RFC 未修改前端或共享 TypeScript，N/A）
- [x] Swift parse、strict-concurrency typecheck（全 App 与 XCTest module iPhoneOS Swift 6 typecheck 已通过）
- [ ] XCTest target build/run（待 Xcode Device build 权限和 CoreSimulatorService 恢复）
- [ ] iOS Device Debug/Release 与 Simulator build
- [x] `check-streaming-session-boundary.sh`
- [x] `git diff --check`
- [ ] xCloud 真实账号 trace gate
- [ ] xHome 真实主机 trace gate
- [ ] `python3 -B iosapp/scripts/check-streaming-core-trace.py --strict` 对 xCloud/xHome 分别 PASS
- [ ] trace token/seed/JWK/handle/account/title/full URL 敏感信息扫描为零

## Final Audit Evidence

最终 Report 必须逐项链接以下证据：

1. 凭证生命周期图与每个 transient/persistent 字段的 owner、创建、轮换、释放和脱敏规则。
2. xHome context 快照，证明 target identity、capabilities、region、console addresses、TURN/ICE 与 session payload。
3. 四通道 profile、bootstrap 顺序、HandshakeAck、输入和震动的 golden fixture 与测试输出。
4. cloud/home offer/answer SDP 摘要和 RTCStats，证明 codec、feedback、ICE、首帧与 steady media。
5. 每种终态的单次 cleanup 测试和真实 trace operationId 配对。
6. 网络健康、恢复链健康、media supply、steady supply、submit/present 长尾与帧供给 delta 的 trace 结论。
7. 所有验证命令的通过记录、构建产物和运行环境。

## Risks

- libwebrtc 隐藏部分反馈控制内部状态，审计需组合 SDP、RTCStats、远端响应和媒体结果证据。
- iOS GameController haptics 能力随控制器型号变化，合同需要明确四路保真、基础震动降级与无 haptics 三种结果。
- xHome console address 与 fallback TURN 依赖实时主机/网络事实，确定性测试使用 fixtures，真实设备 trace 负责证明环境集成。
- 当前工作区含同一 iOS 串流主线的未提交改动，实施需按文件边界增量合并并保护现有用户修改。

## Progress

- [x] Step 1: 已定位桌面与共享 Rust 权威入口。
- [x] Step 2: 已完成三组合同的首版差距矩阵。
- [x] Step 3: 已定义分阶段实现、稳定合同 ID、确定性测试矩阵和最终审计证据。
- [x] Step 4: 用户已确认完整 RFC 执行范围，冻结 typed contracts 与 wire fixtures。
- [x] Step 5: 补齐凭证/session 与 xHome context。
- [x] Step 6: 补齐 WebRTC plan/stats 与四通道状态机。
- [x] Step 7: 补齐输入/震动和终态模型。
- [ ] Step 8: 完成自动验证、真实 trace 审计与 Report。

## Execution Notes

- Date: 2026-07-21 | Status: planned
- Update: 基于当前工作区完成桌面权威入口、iOS 已有实现与缺口审计。
- Decision: 三组合同依次为 token/session、WebRTC、DataChannel/control；共享 Rust 承载跨平台协议和决策，Swift 承载 iOS 平台适配。
- Decision: cloud 与 home 共用会话和通道合同，target-specific 差异全部进入 typed context/plan。
- Decision: 权威优先级固定为共享 Plan、WebRtcDirect 平台适配、Rust-owned 反馈/恢复目标、共享 DataChannel 协议；iOS 保持单一 libwebrtc runtime。
- Decision: WebRTC 首批实施顺序固定为 audio direction、H.264/SDP、feedback evidence、ICE restart、terminal split。
- Decision: access handle 采用可撤销 scoped lease；目录分页复用同一 lease，串流和登录代际变化触发撤销。
- Decision: 用户确认 iOS 输入/震动独立实现；Swift 自持 input codec、rumble parser、GameController 和 haptics，不复用 Rust 手柄库，使用 wire fixtures 保持协议等价。
- Decision: 最终完成条件包含确定性测试和真实 xCloud/xHome trace，两类证据共同构成端到端验收。
- Risk/Blocker: 完整 Xcode Device XCTest build 当前被 SwiftPM manifest diagnostics 写入 `/Users/guo.xu/Library/Caches/org.swift.swiftpm` 的沙箱权限和 CoreSimulatorService 连接错误阻塞；源码级 Swift 6/iPhoneOS typecheck（真实缓存 WebRTC M137 Device framework）与纯 Swift deterministic fixture 已通过。
- Date: 2026-07-21 | Status: in-progress
- Update: 用户确认执行 M4 输入/震动，边界固定为 iOS 原生独立实现。
- Decision: 本阶段只修改 iOS streaming、XCTest、Xcode 工程与任务文档；Rust 手柄库保持原样。
- Date: 2026-07-21 | Status: in-progress
- Update: M4 实现完成：新增 `IOSStreamInputProtocol.swift`，接入 `GCController` 8ms 采样、38-byte wire encoder、change/250ms keepalive、1024/512 bufferedAmount hysteresis、neutral close frame、Better-xCloud/legacy rumble parser、四 locality haptics routing、coalescing、duration clamp、stop 和 cleanup；`XboxStreamDataChannels.swift` 已接入输入/震动生命周期与结构化事件。
- Validation: `cargo test -p xbxengine --lib rumble -- --nocapture`（8 passed）、`cargo test -p xbxengine --lib gamepad_packet_converts_sdl_style_y_only_at_stream_boundary -- --nocapture`（1 passed）、Swift pure deterministic fixtures（exact golden/keepalive/backpressure/rumble/locality，PASS）、Swift 6 iPhoneOS typecheck（真实缓存 WebRTC M137 Device framework，PASS）、`xcrun swiftc -parse`、PBX plist lint、`check-streaming-session-boundary.sh`、定向 `git diff --check`（均 PASS）。
- Evidence gap: 完整 Xcode XCTest/Device build 尚未获得当前环境证据；两次 `xcodebuild build-for-testing` 均停在 SwiftPM manifest diagnostics cache 权限，另有 CoreSimulatorService 不可用。
- Date: 2026-07-21 | Status: in-progress
- Update: M1 bridge contract 已补齐：access handle 采用账户+target scoped lease，owner generation 递增，旧代际撤销，显式 release 保留 revoked tombstone，expiry 含 60 秒 skew 与 15 分钟上限；opaque handle 使用 UUID；cloud catalog 只接受 Cloud lease。准备结果脱敏投影 lease generation/expiry，Swift Keychain 仍只编码 refresh token、seed、web token、app level 与非敏感 account/region scope。
- Update: xHome 主机事实按账户缓存并由 Smartglass 刷新，target 身份按 `serverId -> id -> deviceId` 规范化；能力、power/wake、console addresses、registration snapshot、fallback TURN 与官方 Xbox STUN 进入共享 `Plan` 和 SessionFlow provider。新增 scoped session bridge 入口校验 target/account/generation。
- Update: `XboxPreparedSignaling` 新增共享 `XboxWebRtcPlan` 投影，包含 audio/video direction、H.264 profile/packetization、按 desktop SessionService height profile 的 max-fs/max-fr/码率、stereo、RTCP feedback、candidate types、ICE policy 与 IPv6/end-of-candidates。
- Validation: `cargo test -p xbox-ios-bridge`（38 passed）、`cargo check -p xbox-ios-bridge -p xbox-streaming`、`cargo fmt --all -- --check`、UniFFI bindings 生成、Swift parse、目标 `git diff --check` 均通过；Swift Keychain redaction fixture 和 Rust lease/home/WebRTC deterministic fixtures 已加入。
- Evidence gap: StreamingRuntime/StreamSessionActor 仍需消费 scoped session 与 `snapshot.webRtcPlan` 的 Swift 映射，完整 Device XCTest/build 与真实 xCloud/xHome trace 仍待共享环境和设备证据。
- Date: 2026-07-21 | Status: implementation-complete
- Update: M1-M5 已完成接线。Swift runtime 消费 scoped session 和完整 Rust WebRTC plan；四通道状态机覆盖任意 open 顺序、Handshake/Ack 幂等、严格 post-handshake/control/input bootstrap、100ms 失败重试与 typed remote terminal；单终态 cleanup 固定为 input/haptics、ICE tasks、peer、remote session、access。
- Update: canonical trace 已补齐 attemptId/generation/peerEpoch/operationId、四通道 `label/protocol/ordered` profile 快照、输入/背压/rumble/haptics 累计摘要、production 可见的 RTC/display health、offer/answer/local ICE 顺序和 `iceTasksCancelled` cleanup anchor；trace 不记录 SDP、candidate 或 DataChannel 原始 payload。
- Update: access prepare failure、prepare 后 supersede 和 actor stale request 现在都产生唯一 `terminalSelected`，已获得 access 时同步释放；offer 阶段失败补 `signalingTerminal`。disconnected 先复用 peer 做 ICE restart，失败后在同一 attempt/generation 内递增 peerEpoch、关闭旧 peer、复用 Rust remote session 创建新 peer；旧 peer callback 和旧 ICE task 由 peerEpoch 拒绝。
- Validation: `cargo fmt --all -- --check`；`cargo test -p xbox-auth-flow -p xbox-ios-bridge -p xbox-streaming -p xbxengine-protocol`（0/39/97/7 passed）；`cargo check -p xbox-ios-bridge -p xbox-streaming -p xbxengine`；xbxengine rumble 8 项与 input boundary 1 项；UniFFI bindings；Swift parse；全 App 与 XCTest module iPhoneOS Swift 6 strict-concurrency typecheck；PBX lint；streaming boundary；全局 diff check，全部通过。既有 analyzer 3 项测试通过。
- Evidence: 发现 6 份既有 Simulator dev trace（2119 rows），schema/sequence/privacy/file budget 均为零违规；这些旧 trace 尚无新 canonical streaming anchors，且包含 8 个旧流程 pairing violation，只作为 analyzer 基线事实。
- Evidence gap: `.agents/skills/analyze-ios-runtime-trace` 在当前文件系统 profile 中只读，`--require-flow streaming` analyzer 实现审批持续返回 503；完整 Device `build-for-testing` 审批同样返回 503。真实 xCloud/xHome trace 仍需要账号、主机与真机运行环境采集。
- Date: 2026-07-23 | Status: in-progress
- Audit: 发现 Rust signaling 固定输出 `a=candidate:`，iOS M137 native boundary 直接消费导致 xCloud poll ICE、xHome console/Teredo 候选格式错位；同时确认 iOS Ack 前扣住 control/input bootstrap，与桌面 pre-send + Ack replay 合同存在时序差距。
- Update: Swift native ICE adapter 已剥离 `a=`、过滤 EOC 与异常 `UDP + tcptype`，单 candidate add 失败按候选隔离；DataChannel 已实现 open 后 pre-handshake 预发、Ack 后 post-handshake/control/input 幂等 replay、阶段内失败重试与 `controlReady`。
- Update: `.playing` 现在要求 peer connected、control ready、first video frame 与当前 Metal renderer ready；presentation surface 事件携带 attemptId/generation/peerEpoch；remote ICE batch received/applied/completed、pre/post bootstrap 与 control ready 已进入 canonical trace。
- Update: `.agents` 目录只读期间，项目内新增 `iosapp/scripts/check-streaming-core-trace.py`，复用基础 analyzer 并验证 remote/local ICE、四通道、Handshake、control ready、playing 因果、steady media、非零 Metal surface、唯一 terminal 与 cleanup。
- Validation: Rust bridge/streaming/protocol 44/97/7 passed；streaming-core gate 11 项、既有 analyzer 3 项、全量 Swift parse、session boundary、Rust fmt 与全局 diff check 通过。隔离 Device `build-for-testing` 仍被 SwiftPM diagnostics cache 与 CoreSimulatorService 阻断，沙箱外审批服务返回 404。
- Validation: 2026-07-23 复跑 UniFFI bindings 生成、全量 Swift parse、streaming-core Python tests 11 项，均通过；Rust 44/97/7、session boundary、fmt 与 diff check 保持通过。
- Update: Rust `audio_bitrate_kbps` 已贯通 `XboxWebRtcPlan`、UniFFI 和 Swift SDP projector，audio m-line 写入默认 128 kbps `b=AS`；RTX `apt` 保留和两种 `tcptype` wire 写法均有确定性测试。
- Update: media stats 改为采样 delta，断流不再推进 `lastMediaAt` 或 `steadyMediaObserved`；`closeTransport` 独立关闭并置空四条 DataChannel；`.playing` 增加当前 attempt/generation/peerEpoch 的 Metal `videoSurfaceRendererReady` 门禁。
- Update: xHome 初始启动消费 `wake_console/require_console_ready`，待机主机首轮唤醒后等待显式注册；Rust scheduler 在 SessionReady 后持续轮询远端 terminal，iOS bridge 投影脱敏 terminal，Swift 在 ICE EOC 后低频继续轮询并收口 cleanup。
- Validation: 2026-07-23 第二轮复跑 Rust bridge/streaming/protocol 47/103/7、全量 Swift parse、streaming boundary、streaming-core 11 项、Rust fmt 与 diff check，均通过。
- Update: 独立 iPhoneOS 26.1 / Swift 6 strict typecheck 捕获并修复 Metal renderer ready sink 的可选 trace context 编译错误；App 与 XCTest 使用真实 WebRTC M137 Device framework、UniFFI bridging header 完成 typecheck，XCTest 仅保留两处既有末表达式 warning。
- Update: iOS 设置页与 `AppSettingsStore` 现在只暴露有消费点的串流设置，并已贯通到 Rust `control_plan`：`preferred_game_language`、xCloud/xHome resolution、codec、IPv6、xCloud/xHome/audio bitrate 与 xHome TURN fallback；`video_format`、`display_options`、`performance_style`、`super_resolution_experimental`、`stream_runtime_mode`、`use_vulkan` 等无 iOS 原生消费点的选项继续跳过。
- Validation: 2026-07-24 复跑 Rust bridge/streaming/protocol 49/103/7、UniFFI bindings、全量 Swift parse、streaming boundary、streaming-core 11 项与 diff check，均通过；新增桥接测试固定验证 cloud/home 两条设置快照都会进入 Rust `Plan`。
- Evidence gap: 现有 6 份 Simulator trace 共 849 rows，schema/sequence/privacy/budget 通过，但没有 `ios-streaming` 事件；streaming-core gate 唯一失败为 `missing-streaming-attempt`。最终完成仍需 fresh xHome/xCloud 真机 trace 分别通过 streaming-core gate。
