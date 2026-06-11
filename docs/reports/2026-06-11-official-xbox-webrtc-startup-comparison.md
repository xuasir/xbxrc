# 官方 Xbox WebRTC 启动与建连对照报告

## 范围

本报告汇总 2026-06-11 的官方 Xbox Remote Play 独立 Chrome 采集，并对照当前项目 runtime trace 与本轮实现。目标是回答三件事：

- 官方从主机卡片启动到串流建连的标准流程。
- 主机准备较慢与主机已就绪两类启动路径的时间差异。
- 我们历史 50s 级建连等待与官方路径的差距，以及对应优化。

可用官方样本：

| 样本 | 目录 | 有效性 | 结果 |
| --- | --- | --- | --- |
| 空样本 A | `runtime-logs/official-xbox-capture-2026-06-11T06-16-09-085Z/` | 仅有 meta，`events/webrtc-stats` 为空 | 只记录采集启动，不能用于协议结论 |
| 失败网络 | `runtime-logs/official-xbox-capture-2026-06-11T06-20-13-579Z/` | 有效 | 控制面完成，ICE checking 后失败 |
| 空样本 B | `runtime-logs/official-xbox-capture-network2-20260611-144536/` | 仅有 meta，`events/webrtc-stats` 为空 | 只记录采集启动，不能用于协议结论 |
| 成功网络 | `runtime-logs/official-xbox-capture-2026-06-11T06-46-55-910Z/` | 有效 | 控制面完成，ICE connected，1080p60 稳定输入 |

## 官方标准流程

两份有效样本的控制面顺序一致：

1. `GET /v6/servers/home` 读取主机列表。
2. 用户点击主机卡片。
3. `POST /v5/sessions/home/play` 创建 home session。
4. 多次 `GET /v5/sessions/home/{sessionId}/state` 等待 console/session ready。
5. `GET /v5/sessions/home/{sessionId}/configuration` 获取串流配置。
6. 浏览器创建 `RTCPeerConnection`，ICE server 仅见 `stun:relay.communication.microsoft.com:3478`。
7. `POST /v5/sessions/home/{sessionId}/sdp` 提交 offer。
8. `GET /v5/sessions/home/{sessionId}/sdp` 先 `204`，后 `200` 取得 answer。
9. `POST /v5/sessions/home/{sessionId}/ice` 提交本地候选。
10. `GET /v5/sessions/home/{sessionId}/ice` 轮询远端候选，先多次 `204`，最后 `200`。
11. 浏览器 `addIceCandidate` 后进入 `checking/connecting`。
12. 成功样本进入 `connected/connected` 并 keepalive；失败样本进入 `disconnected/failed` 后 DELETE session。

## 启动维度

现有有效采集没有保存官方控制面响应体，事件里没有明文硬件电源态字段。这里用 `POST /play -> configuration 200` 和 state polling 次数表达主机准备耗时，可覆盖“待机式准备较慢”和“已开机/已就绪准备较快”的差异。采集脚本已补 `network-response-body` 摘要采集，后续复采可直接从 `servers/home`、`state`、`configuration` 的脱敏响应体确认 power/session state。

| 维度 | 失败网络样本 | 成功网络样本 |
| --- | --- | --- |
| `POST /play -> configuration 200` | `14.231s` | `6.089s` |
| `play 202 -> configuration 200` | `12.303s` | `3.576s` |
| state poll 次数 | `5` | `2` |
| `configuration 200 -> pc-created` | `4.432s` | `20.986s` |
| `pc-created -> remote ICE applied` | `7.211s` | `7.339s` |
| `remote ICE -> connected` | 无 | `0.395s` |
| `remote ICE -> failed` | `15.437s` | 无 |
| `POST /play -> connected/failed` | `41.311s` 到 failed | `34.809s` 到 connected |

结论：

- 主机准备阶段差异主要体现在 state polling：较慢样本 5 次轮询、约 14.2s 到 configuration；较快样本 2 次轮询、约 6.1s 到 configuration。
- SDP/ICE 交换阶段在两份有效样本中接近，`pc-created -> remote ICE applied` 都约 7.2-7.3s。
- 真正决定串流成功和失败的是远端 ICE 到达后的 connectivity check 结果。

## 建连方式

两份有效样本的 PeerConnection 配置一致：

- ICE server：`stun:relay.communication.microsoft.com:3478`
- 官方采集中未见 TURN relay server。
- 本地候选：host UDP/TCP、srflx UDP IPv4。
- 远端候选：host UDP IPv4、host UDP IPv6、srflx UDP IPv4。
- 视频 answer：H264，RTCP feedback 包含 `goog-remb`、`transport-cc`、`ccm fir`、`nack`、`nack pli`。
- data channel：`input`、`control`、`message` 等通道，成功样本均 open。

失败网络样本：

- PeerConnection 在 `checking/connecting` 后 `15.437s` 进入 failed。
- candidate pair 保持 `in-progress/nominated=false`。
- 最大 STUN check 计数：`requestsSent=35`、`responsesReceived=0`。
- 无 selected/nominated pair，无 inbound video。

成功网络样本：

- 远端 ICE applied 后 `0.395s` 进入 `connected/connected`。
- selected pair：`succeeded/nominated=true`。
- local selected candidate：`prflx UDP IPv4`。
- remote selected candidate：`srflx UDP IPv4`。
- 最终 candidate pair：`requestsSent=133`、`responsesReceived=133`。
- 约 341s 稳定段：视频 1080p60，平均码率约 `15.67Mbps`，丢包率约 `0.044%`，NACK 增量 `241`，PLI 增量 `1`。

## 与项目历史 trace 对比

项目旧 trace 显示 direct ICE 长时间停在 `Connecting`：

| 项目 trace | 关键窗口 |
| --- | --- |
| `runtime-logs/runtime-trace-1781155623392-1.jsonl` | `transportState=Connecting` 后出现 `233.903s` ledger silence，无 selected pair、无入站视频 |
| `runtime-logs/runtime-trace-1781148446586-1.jsonl` | `transportState=Connecting` 后出现 `663.364s` silence，无 selected pair、无入站视频 |
| `runtime-logs/runtime-trace-1781155396663-1.jsonl` | `pollIce:Streaming error: token missing`，属于 token/启动失败样本，ICE 性能对比价值较低 |

官方失败网络样本在 `15.437s` 内收口，官方成功样本在 `0.395s` 内连上。项目历史 `50s` 级等待和数百秒 Connecting 空转均明显慢于官方失败收口。

## 已实施优化

### 流程标准化

- `runtime-host` 记录 `launchRuntimeAttempt`、`runtimeLaunchReadyToInvoke`、`runtimeLaunchPortBound`、`directFirstExhaustionProbeScheduled`、`directFirstExhaustionProbe`、`fallbackTurnRetry`、`fallbackTurnRetryResult`。
- direct-first 首轮启动保留直连语义，fallback retry 显式记录 `directPathExhausted`。
- TURN relay allocation 失败时返回 `xbxEngineRtcTurnRelayAllocationFailed`，保留 `rtcTurnRelayAllocationFailed` 诊断，避免 fallback 轮次被误记为成功。

### 候选逻辑

- Browser runtime 与 Rust-owned ICE policy 保留完整远端 direct 候选集合。
- family mismatch gate 改为观测信号，`skippedByFamilyMismatchCount=0`，digest 增加 `familyMismatchObserved`。
- 远端 IPv4 host、IPv6 host、IPv4 srflx 都进入 ICE，交给标准 ICE connectivity check 裁决。
- 成功样本证明 local `prflx` 是有效 selected path，诊断与 selected pair 统计保留该类型。
- Rust `transport_path` 增加 `Direct (prflx->srflx)`，把官方成功路径归入直连 NAT 路径。

### 建连收口

- 移除“进入 connecting 后固定 12s 切 TURN”的时间型触发。
- 新增 `rust-owned direct-first` 耗尽探测：首轮 direct 启动后延迟 `8s` 读取 stats。
- 旧耗尽证据保留：`transportState=Connecting + transportCandidatePair 为空 + inboundVideoPacketCountTotal=0 + inboundVideoBytesTotal=0 + launch spec 带 fallback TURN` 时触发一次 fallback。
- 新增 Rust `iceConnectivityProbe`：每秒聚合 candidate-pair `requestsSent/responsesReceived`、selected/nominated、succeeded/in-progress/failed、local/remote candidate type 与 address family，并写入 `latest_observation=iceConnectivityProbe`。
- 首帧前 `Connecting/Recovering + maxRequestsSent>=8 + responsesReceivedTotal=0 + 无 selected/nominated + probe <=2.5s 新鲜 + no-progress 持续 12s` 时，session policy 触发 `RequestReconnectCandidate`。
- `closed/failed` 事件仍可触发同一 fallback gate，和官方失败态收口一致。

## 验证

已执行：

```bash
node --check scripts/official-xbox-capture.mjs
./node_modules/.bin/vitest run src/streaming/runtime/runtime-host-policy.test.ts src/streaming/runtime/ice-candidate-policy.test.ts
cargo test -p xbxengine family_mismatch --lib
cargo test -p xbxengine remote_ice_policy_keeps_cross_family_host_candidates --lib
pnpm exec eslint src/streaming/runtime/ice-candidate-policy.ts src/streaming/runtime/ice-candidate-policy.test.ts src/streaming/runtime/browser-runtime.ts src/streaming/runtime/runtime-host.ts src/streaming/runtime/runtime-host-policy.ts src/streaming/runtime/runtime-host-policy.test.ts --fix
cargo fmt
cargo check -p xbxrc
cargo check -p xbxengine
cargo test -p xbxengine transport_metrics --lib
cargo test -p xbxengine reconnect_lifecycle --lib
git diff --check
git diff --cached --check
```

全量 `pnpm lint:fix` 仍被既有无关 lint 阻塞：

- `src/player/infra/render/Renderers.test.ts:1040`
- `src/shared/gamepad/wait-pad-neutral.ts:58`
- `src/streaming/diagnostics.ts:246`

## 后续验收口径

下一条 fresh trace 应验证：

- direct 首轮失败时出现 `directFirstExhaustionProbe result=exhausted`。
- Rust-owned trace 出现 `iceConnectivityProbe`，失败网络下应看到 `noResponse=true`、`maxReq>=8`、`respTotal=0`、`selectedOrNominated=false`。
- `fallbackTurnRetry` 在 8-16s 量级触发，避免回到 50s 级等待。
- 候选 digest 包含 `familyMismatchObserved`，且远端 IPv4/IPv6 host 与 srflx 未被删除。
- 成功直连时 `transportCandidatePair`、inbound video 包或 bytes 出现后，probe 输出 `stillPending` 或被 connected 清理。
