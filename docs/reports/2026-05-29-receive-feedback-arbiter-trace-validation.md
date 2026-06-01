# Report：Receive Feedback Arbiter Trace 验收（双主线）

**关联：** [`rfcs/2026-05-29-receive-feedback-arbiter-landing-fix-checklist.md`](../rfcs/2026-05-29-receive-feedback-arbiter-landing-fix-checklist.md) · [`reports/2026-05-29-receive-feedback-arbiter-landing-fix.md`](2026-05-29-receive-feedback-arbiter-landing-fix.md)

## 样本矩阵（本地 `runtime-logs/`）

| 角色 | 文件 | 录制时期 | 说明 |
|------|------|----------|------|
| Failing replay（基线） | `runtime-trace-1779961935840-1.jsonl` | 2026-05-28 | 改前 silent-stuck 画像；**非**当前二进制重放 |
| Continuation-only | `runtime-trace-1780024427780-1.jsonl` | 2026-05-29 | 弱恢复段；含 `receivePictureRecoveryTerminal` |
| Healthy（代理） | `runtime-trace-1780017393821-1.jsonl` | 2026-05-29 | 含 `FreshAnchorRecovered`；无完整 `DisplayStable` 相位 |
| Supply-starved | — | — | 本地 trace 未出现 `displaySupplyStarved` 字符串；新采时需覆盖 |

脚本 JSON 归档：[`trace-validation/`](trace-validation/)（`replay.json` / `continuation.json` / `healthy.json`）。

## 自动化验收（`trace_receive_feedback_report.py`）

```bash
python3 .agents/skills/analyze-runtime-logs/scripts/trace_receive_feedback_report.py runtime-logs/<trace>.jsonl
```

### 结果摘要

| 样本 | `receiveFeedbackGate` | `keyframeChain` | `terminalAny` | 主线判定 |
|------|----------------------|-----------------|---------------|----------|
| replay | **FAIL** `silentStuck`, `lowResponseObservedRate` | sent=98，response=0，decoded=1，anchor=1 | 0 | 基线符合改前 FAIL 画像 |
| continuation | **FAIL** `arbiterMismatchTotal`, `needKeyframeNonIdrFeedViolations`(1) | sent=178，decoded=1，terminal 路径有进展 | **38** | **终端策略运营闭环成立**（无 `silentStuck`） |
| healthy | **FAIL** 同上 | sent=230，anchor=1，response=2 | 6 | 有部分锚点闭合；**待当前构建新采** 以冲 `receiveFeedbackGate=PASS` |

脚本调整（本轮）：

- `keyframeChain` 计入 `FreshAnchorRecovered` / `PlaybackRecovered`（与 stats 主线相位对齐）。
- `insertSurfacePhaseActionStage` 仅在 trace 带 **`packetRecoveryActionStage`** 且其为 surface 名时 FAIL；旧 `actionStage` 投影不计入 gate。
- `sparseMustIdrMismatchTotal` 仅报告、不进入硬 gate（诊断投影）。
- 输出 `episodeProjectionStateCounts` / `displaySupplyStarvedBlockerCounts`（新 trace 投影字段）。

## 人工核对（ledger-first）

### 1. Coordinator / response observed

- **continuation** `seq=457`：`receivePictureRecoveryTerminal` `reason=remote-continuation-only`，`ledgerGeneration=23`，`responseState=non-idr-only`，`referenceState=need-keyframe` — 远端 continuation 有明确 terminal，非长期 `repairing` 沉默。
- **replay**：98× `sent`，0 terminal，0 `receivePictureRecoveryTerminal` — 与改前 checklist 画像一致。

### 2. Owner / display

- continuation 存在 `pictureRecoveryTransition` → `PlaybackRecovered`（`seq=249`），未要求旧 episode 绑定。
- 旧 trace 的 `insertGateDecision` 仍带 `actionStage`/`mediaSupplyPhase`（改前投影）；当前代码已改为 `packetRecoveryActionStage` + `mediaSupplyPhaseDiagnostic`（见 `trace_projection` 单测）。

### 3. Insert gate

- continuation 仅 1 条 `need-keyframe` + `emit`（`reason=decodableToFeed`，`actionStage=steady`）— 旧投影时序；新 trace 应用 `packetRecoveryActionStage` 复核。

### 旧架构清除（代理）

| 项 | continuation 证据 |
|----|-------------------|
| Terminal 可读 | 38× `receivePictureRecoveryTerminal`，含 `remote-continuation-only` / `remote-no-response` |
| Ledger generation | terminal payload 带 `ledgerGeneration` |
| Episode | 多为 `episodeId: null`；closure 不依赖 episode |
| 新投影字段 | 代码已发 `episodeProjectionState` / `displaySupplyStarvedBlocker`；**上述三份 JSONL 为改前录制，字段为空** |

## 结论与下一步

| 主线 | 状态 |
|------|------|
| 核心接收侧 — 终端 / 无 silent stuck（continuation 样本） | **运营闭环可接受**（待同构建 healthy 新采冲 gate） |
| 核心接收侧 — 全链 `sent→displayStable` | **待新采 healthy trace**（本地无 `DisplayStable` 相位记录） |
| 旧架构 — 控制面 ledger-first | 代码 + 单测闭合；trace 需 **当前 workspace 新采** 验证投影字段 |
| Failing replay 文件 | 保留作回归基线（预期 FAIL `silentStuck`） |

**建议你统一新采时：**

1. `runtime_trace_mode` 非 off，命名 `-healthy` / `-continuation`。
2. healthy：正常起播至稳定画面 ≥30s。
3. continuation：弱网或已知 continuation-only 场景。
4. 每条跑脚本；期望 healthy `receiveFeedbackGate=PASS` 且 `keyframeChain.displayStable>0`（或 `PlaybackRecovered`+ledger `display-stable`）。
