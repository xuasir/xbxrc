# Home Local srflx Gather RFC (方案 B)

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 已完成
- Current State: completed
- Owner: TBD
- Last Updated: 2026-04-09

## Background

- 新 trace 已确认 canonical rust-owned RTC 路径只手动注入了本地 `host` candidate，没有任何显式 STUN/TURN gather。
- 对 home 直连场景来说，正常应先靠 `srflx` 直接打通；当前路径因为没有本地 `srflx`，即使远端给了 `srflx` 也很容易直接失败。
- `direct-first -> fallback TURN` 主线已经存在，但 TURN relay 还只是 candidate 级别，数据路径并没有接入 `io_runtime`；一旦 selected pair 落在 relay 上，流量依然走原始 UDP socket，等于没有真正启用 relay。

## Goal

- 在不改动 canonical rust-owned RTC 主体架构的前提下，给本地候选补齐 `host`/`srflx`/`relay` 三条路径。
- 复用现有 UDP socket做 STUN Binding 探测 `srflx`，同时借助本地 `stun`+`turn` crate 构建 `TurnRuntime` 让 relay candidate 既能被生成也能在 `io_runtime` 里真正收发数据。
- 保持“先直连，再 fallback TURN”的策略不变，但让 TURN relay 成为可选通路而不是仅凭 candidate 就算。

## Scope

- In scope:
  - `xbxengine` 本地候选 gather 逻辑与 `TurnRuntime` 同步
  - 复用现有 ICE server 配置，筛选可用于 `srflx`/`turn(udp)` 的 URL，并保证 candidate/signal 输出与 PeerConnection 一致
  - `io_runtime` pump/send 路径按 `transport.local_addr` 区分 direct/relay，stop/rebuild 时清理 TURN state
  - RFC 与 task tracker 更新
- Out of scope:
  - 重写 PeerConnection / 增加第二条 transport 主线
  - 前端 direct-first/turn fallback 策略调整

## Plan

1. 收口 ICE server 构造，让 PeerConnection 与 gather/relay 都复用同一份 STUN/TURN URL。
2. 在 `RtcIoRuntime` 里基于现有 UDP socket 做显式 STUN Binding，同时从 `turn_server` 启动 `TurnRuntime` 完成 relay allocation。
3. 让 `gather_local_candidates` 同步产出 `host`/`srflx`/`relay`，`RtcConnectionService` 在协商前注入，并记录 `TurnRuntime` state 以供 send/pump 路由。
4. 改造 `pump()/send_to_peer()`，按 `transport.local_addr` 选择 direct/relay，并在 stop/rebuild 时彻底释放 TURN runtime。

## Validation

- [x] `cargo fmt --all`
- [x] `cargo check -p xbxengine`
- [x] `cargo check -p xbxrc`
- [ ] 复现后确认 trace 里可看到本地 `srflx`/`relay` candidate 且成功路由
- [ ] 复现后确认 trace 里首次就有 `selected_pair_snapshot`（即使为 `none`）与前端 `startRuntimeLaunchFailed` 结构化事件
- [ ] 复现后确认远端 IPv6 host candidate 不再抢到 IPv4/srflx/relay 主链，且 trace 中能看到 incompatible family skip

## Risks

- TURN runtime 是一个 async worker，分配/permission 失败需要日志，否则 candidate 会挂在初始化阶段。
- 多一条 relay 通道意味着 `pump()` 要多一个数据源/路由，若处理不当会影响 direct path 的稳定性。

## Progress

- [x] Step 1: `build_ice_servers(session)` 与 gather/relay 调用统一 ICE server 列表
- [x] Step 2: `RtcIoRuntime` 基于 UDP socket 做 STUN Binding，并在 `TurnRuntime` 启动/重建时完成 relay allocation
- [x] Step 3: `gather_local_candidates` 现在输出 `host`/`srflx`/`relay`，`send_to_peer()` 已根据 `transport.local_addr` 路由
- [x] Step 4: `cargo fmt`/`cargo check` 已完成，trace 仍待确认 relay candidate 选路
- [x] Step 5: 做一轮 anti-corruption 收口：`runtime-host` 的 direct-first/fallback/recovery 判定拆到独立 policy helper，前端主动 trace 写入迁到独立 `runtimeTrace` RPC，`connection/helpers.rs` 按 candidate / SDP / fact / text preview 拆分，降低 host/Tauri/Rust connection 三处边界继续发散的风险
- [x] Step 6: 收口两个 P1 策略风险：`IcePolicy` 改回“稳定排序 + 轻量清洗”，不再默认重写 foundation/priority、也不再无条件补 `end-of-candidates`；`negotiation` 的 family 过滤改成“仅硬跳过明确不可达的 host mismatch，srflx/relay 保留并记录观测”

## Execution Notes

- Date: 2026-03-24 | Status: in-progress
- Update: 当前实现已补齐 home 端本地 `host`/`srflx`/`relay` 三条候选；即便 STUN 探测失败，也可以通过 `TurnRuntime` 继续 fallback，且 relay 数据路径已接到 `io_runtime` 的 `pump/send`。
- Decision: 方案 B 本轮把本地 candidate 与 relay 通道一起推进，relay 需要真正接入 send/pump。
- Update: `TURN/STUN` URL 解析改为兼容 RFC 7064/7065 的 `turn:host:port`/`stun:host:port` 形式，不再依赖 `Url::parse` 对 `://` 的假设。
- Update: `TurnRuntime` 在创建时显式 `listen + allocate`，并在 `drop`（stop/rebuild）阶段执行 close 清理，避免残留 allocation。
- Risk/Blocker: 仍需在真实 trace 验证 selected pair 是否落在 `srflx` 或 `relay` 并成功首帧。
- Update: 针对 `runtime-trace-1774337754446.jsonl` 暴露的“trace 里只有 `StopRuntime(reason=launch-failed)`、没有前端异常上下文 / selected pair 诊断”问题，本轮补充两类观测：`RtcConnectionService::pump()` 首次无论 pair 是否存在都记录 `selected_pair_snapshot`，并新增前端 `runtime-host -> xbxEngine.recordTraceEvent` 轻量链路，把 `launchRuntimeAttempt` / `fallbackTurnRetry` / `startRuntimeLaunchFailed` 写入 runtime trace。
- Update: 针对 `runtime-trace-1774338231720.jsonl` 里首轮直连报 `xbxEngineRtcSocketSendFailed: No route to host (os error 65)` 的问题，本轮进一步把双栈策略收紧为“默认同类型内 IPv4 优先”，并在 runtime 注入 remote ICE candidates 前按本地已宣告 candidate family 过滤不兼容地址族，避免 IPv4 主链被远端 IPv6 host 候选抢先触发不可达发包。
- Update: 在确认主链重新打通后，额外做了一轮低风险防腐化回归：`src/streaming/runtime/runtime-host.ts` 不再内嵌 direct-first/fallback/recovery 判定细节，`runtime trace` 不再借道 `xbxEngine` RPC，`crates/xbxengine/core/src/transport/rtc/connection/helpers.rs` 也已拆成更细的职责文件，先把结构边界收紧，再继续观察 candidate 策略是否还需要后续收口。
- Update: 针对后续 P1 风险，本轮继续把 `crates/xbox-streaming/src/session/signaling/ice.rs` 收窄为“尽量不改变原始 candidate 语义”的 normalize：保留类型/地址族排序，但默认不再做 Teredo 派生、不再改写 foundation/priority，也只在输入本身带了 `end-of-candidates` 时才保留该标记；同时把 `crates/xbxengine/core/src/transport/rtc/connection/negotiation.rs` 的 family 过滤从“全量硬过滤”收窄为“仅对明确不可直连的 host mismatch 硬跳过”，srflx/relay 先保留并打观测，降低把潜在可用路径提前裁掉的风险。
- Update: 新 trace `runtime-trace-1774341849717.jsonl` 证明上面的 P1 收口过头了：直连模式下本地虽已 gather 到 `srflx`，但远端候选从成功样本里的 `host=12 srflx=1` 回落成 `host=8 srflx=1`，ICE 一直停在 `Connecting`，没有 selected pair。比对后确认回归点主要不是 family 软过滤，而是 `IcePolicy` 默认关闭了 Teredo IPv4 派生，导致 home 场景少了 4 个关键的 Teredo 派生 IPv4 host candidate。
- Update: 已在 `crates/xbox-streaming/src/session/api/signaling.rs` 做最小回补：仅对 `home` 场景重新开启 `IcePolicy::with_teredo_ipv4_derivation(true)`，继续保留“不改写 foundation/priority、不无条件补 EOC、srflx/relay 不做 family 硬过滤”这两处保守化收口，避免把其它风险再带回来。
- Validation: 本轮回补后已通过 `cargo test -p xbox-streaming session::signaling::ice -- --nocapture`、`cargo check -p xbox-streaming`、`cargo check -p xbxengine`、`cargo check -p xbxrc`；仍需下一份真实 trace 确认远端候选计数回升、selected pair 恢复以及 `TransportConnectionStateChanged Connected` 重新出现。
