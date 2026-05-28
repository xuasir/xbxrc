# Report：恢复链减法验证（2026-05-27）

**RFC：** [2026-05-27-recovery-subtraction-libwebrtc-alignment.md](../rfcs/2026-05-27-recovery-subtraction-libwebrtc-alignment.md)  
**参考 trace：** `runtime-logs/runtime-trace-1779945208404-1.jsonl`（减法前基线）

## 代码变更验证

| 检查项 | 方法 | 结果 |
|--------|------|------|
| 单元/集成测试 | `cargo test -p xbxengine --lib` | 982 passed（2026-05-27 本地，补齐后） |
| Insert 单轨 | `insert_gate` 无 `timedFallbackDecodable`；`mustIdrHold` 覆盖 MustIdr delta | 通过 insert_gate / ingress_loop 测 |
| Fast path 移除 | `displayed_idr_fast_path` 模块不存在 | grep 无生产引用 |
| 冗余 API | 无 `reference_chain_*`、`timed_fallback_decoder_unstick_*` 导出 | 静态确认 |
| Contract 拆分 | 无 `behavior.rs`；`contract/mod.rs` 聚合行为子模块 | 静态确认 |
| Episode/ledger | 仅在 `session/facts/recovery_episode.rs`、`gap_severity.rs` | 静态确认 |
| Session reset promote | coordinator 无 `maybe_promote_*decoder_unstick` | 静态确认 |
| 四标准不退化 | `DelegatedToReceive` / `suppress_session_picture_recovery_action` 保留 | escalation/coordinator 测通过 |

## 后续 trace 验收（由你本地执行）

**前置：** 复跑同场景并生成 `runtime-logs/<your-trace>.jsonl`。

```bash
# 1. 全量测试
cargo test -p xbxengine --lib
cargo test -p xbxengine ingress_loop contract

# 2. 汇总（路径替换）
python3 .agents/skills/analyze-runtime-logs/scripts/summarize_runtime_trace.py \
  runtime-logs/<your-trace>.jsonl --categories event,decision,snapshot

# 3. 重点 grep / count
# - requestDecoderReset：末段仅 backend-error 附近，无 200+ 集群
# - insertGateDecision reason=mustIdrHold：waiting-keyframe 段无 decodableToFeed emit
# - recovery_surface_phase / session_phase：尽早 await-idr / active-recovery
# - pictureRecoveryDelegated、recovery_session_keyframe_in_flight=False
```

| 可证伪项 | 期望 | 执行结果 |
|----------|------|----------|
| Reset 风暴消失 | 末段 `requestDecoderReset` 无 200+ 集群 | _待填_ |
| Insert 纪律 | `mustIdrHold` 段无 hopeless `decodableToFeed` | _待填_ |
| 控制面叙事 | `await-idr` / `active-recovery` 尽早出现 | _待填_ |
| Receive 主权 | `pictureRecoveryDelegated`、receive PLI 可见 | _待填_ |
| Session in-flight | `recovery_session_keyframe_in_flight` 仍全 False | _待填_ |

`trace_projection` 已消费 `latest_insert_decision_reason`；新标签 **`mustIdrHold`** 与 Insert 对齐。

## 已知保留

- `recovery_timed_fallback_active_from_stats` 仍用于 display 放松、exit path 与 decode 续播窄路径，**不**再驱动 decoder reset promote / supply-break unstick
- `decoder_reset_permitted`：`waiting-keyframe` 时仅 Reconfigure bypass、fresh IDR admission、或 episode 进度达标

## Definition of Done（补齐）

| # | 项 | 状态 |
|---|-----|------|
| 1 | 无 `reference_chain_*`、`timed_fallback_decoder_unstick_*` 导出 | 已满足 |
| 2 | `contract/` 多模块；无 `behavior.rs` | 已满足 |
| 3 | episode/ledger 仅在 `session/facts/`；contract 不 re-export | 已满足 |
| 4 | `decoder_reset_permitted` 不在 supply-break / timed-fallback 上放行 reset | 已满足 |
| 5 | `cargo test -p xbxengine --lib` 全绿 | 982 passed |
| 6 | 本 report 含可执行 trace 验收脚本 | 已写入；**执行结果待填** |
