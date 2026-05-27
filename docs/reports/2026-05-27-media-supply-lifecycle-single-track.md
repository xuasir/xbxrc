# Report：MediaSupply 生命周期单轨

**日期：** 2026-05-27  
**RFC：** [2026-05-27-media-supply-lifecycle-single-track.md](../rfcs/2026-05-27-media-supply-lifecycle-single-track.md)

## 实施摘要

| 层 | 变更 |
|----|------|
| L0 | `MediaSupplyPhase` + `derive_media_supply_phase_from_stats`；`recovery_surface_phase` 由 L0 投影；PS 严进 / repairing 压力需 `displayed_idr` 且非 priming |
| L1 | `InsertContext.media_supply_phase`；Priming 下 gap keyframe-only 仍允许 steady continuation |
| L3 | `post_first_present_acquisition_active`；Priming 不再仅凭 completion 未就绪即 `SupplyStarved` |
| L4 | HoldRepair 在 must-idr/repairing/supply-break 发 receive-local PLI；session fresh-output 抑制在 priming/must-idr/repairing 绕过 |
| 观测 | `media_supply_phase` stats/trace；`trace_midsegment_report.py` 增加 `MEDIA_SUPPLY_GATE` |

## 验证

```bash
cargo test -p xbxengine --lib   # 978 passed
python3 .agents/skills/analyze-runtime-logs/scripts/trace_midsegment_report.py runtime-logs/runtime-trace-<id>.jsonl
```

## 四条 trace 门禁（需 clean build 复采后判定）

| Trace | 场景 | 改前问题 | 复验命令 |
|-------|------|----------|----------|
| 1779865309109 | 起播 | 首显后 supply-starved、无 PLI | 全窗 + `MEDIA_SUPPLY_GATE` |
| 1779701428966 | 稳态 | 基线 PASS | `--start-s 79 --end-s 150` |
| 1779863398229 | 修洞末段 | Hold 与 PLI 脱节 | 末段 90s 窗（待脚本扩展） |
| 1779853304655 | 回归 | supply-break 叠层 | 长窗人工 |

**说明：** 仓库内既有 trace 为改前二进制采集，`MEDIA_SUPPLY_GATE` 在 `1779865309109` 上预期仍为 FAIL，直至合入后重采。
