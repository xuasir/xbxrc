# 恢复层基线指标 (Phase 0 Baseline)

> 采集日期: 2026-04-16
> 数据来源: runtime-logs/ 中5个trace文件
> 总会话时长: 846.9秒 (~14.1分钟)
> 关联RFC: recovery-layer-simplification.md

## 数据来源

分析了以下5个runtime trace文件：
- runtime-trace-1776247584658-1.jsonl
- runtime-trace-1776232154018-1.jsonl
- runtime-trace-1776238985854-1.jsonl
- runtime-trace-1776046069885-1.jsonl
- runtime-trace-1776234623071-1.jsonl

## 1. NACK恢复基线

| 指标 | 值 |
|------|-----|
| 总NACK过期事件 | 9,753 |
| 平均重试次数 | 0.00 |
| 中位重试次数 | 0 |
| NACK事件频率 | 11.52 events/s |

**观察**：
- 所有NACK都是0重试，说明当前策略倾向于快速放弃NACK，直接升级到IDR
- 高频率的NACK过期（11.52/s）表明网络丢包较频繁

## 2. 关键帧请求基线

| 指标 | 值 |
|------|-----|
| 总关键帧请求 | 1,884 |
| 平均请求间隔 | 168ms |
| 中位请求间隔 | 31ms |
| 最小请求间隔 | 0ms |
| 最大请求间隔 | 30,271ms |
| 关键帧请求频率 | 2.225 requests/s |

**观察**：
- 中位间隔31ms表明关键帧请求非常频繁
- 最小间隔0ms说明存在同一时刻多次请求的情况（可能是风暴）
- 平均频率2.225 requests/s较高，可能存在过度请求

## 3. 解码器重置基线

| 指标 | 值 |
|------|-----|
| 总解码器重置 | 0 |

**观察**：
- 在这些会话中没有触发解码器重置
- 说明解码器层面较稳定，或者恢复策略在到达decoder reset前就已经处理

## 4. 重连基线

| 指标 | 值 |
|------|-----|
| 总重连事件 | 0 |

**观察**：
- 没有触发重连
- 说明恢复策略在更早的层级（NACK/IDR）就解决了问题

## 5. 帧丢弃基线

| 指标 | 值 |
|------|-----|
| 总帧丢弃 | 208 |
| 帧丢弃频率 | 0.246 drops/s |

**丢弃原因分布**：
| 原因 | 数量 | 占比 |
|------|------|------|
| decode:drop:outputQueueOverflow | 155 | 74.5% |
| render:replace:latestSlotOverwrite | 47 | 22.6% |
| reconfigure:parameterSetsChanged | 4 | 1.9% |
| frameAbandoned:late | 2 | 1.0% |

**观察**：
- 主要丢帧原因是解码输出队列溢出（74.5%）
- 说明解码速度跟不上或队列管理需要优化
- 渲染层覆盖占22.6%，说明渲染队列也有压力

## 6. 恢复决策基线

| 指标 | 值 |
|------|-----|
| 总恢复决策 | 47,421 |
| 恢复决策频率 | 56.00 decisions/s |

**动作分布**：
| 动作 | 数量 | 占比 |
|------|------|------|
| none | 20,020 | 42.2% |
| cooldownSuppressed | 14,342 | 30.2% |
| waitForDecoderResetBurst | 8,732 | 18.4% |
| coalesced:keyframeInFlight | 2,685 | 5.7% |
| coalesced:decoderResetInFlight | 1,120 | 2.4% |
| waitForBurst | 330 | 0.7% |
| requestKeyframe | 169 | 0.4% |
| requestDecoderReset | 23 | 0.0% |

**观察**：
- 42.2%的决策是"none"（无需恢复）
- 30.2%被cooldown抑制，说明冷却机制在频繁工作
- 18.4%在等待decoder reset burst，但实际requestDecoderReset仅0.0%
- 5.7%的keyframe请求被coalesce（in-flight门控生效）
- 实际执行的requestKeyframe仅0.4%（169次），远低于关键帧请求总数（1,884）

## 7. 恢复延迟基线

**注意**：当前日志中未能提取到完整的恢复延迟数据（从问题检测到恢复完成的端到端延迟）。

**需要补充的指标**：
- NACK → IDR平均延迟
- IDR → 解码完成平均延迟
- Decoder reset → 恢复平均延迟
- Reconnect → 首帧平均延迟
- 端到端恢复延迟

## 8. 恢复成功率基线

**注意**：当前日志中未能提取到明确的恢复成功率数据。

**需要补充的指标**：
- NACK自愈成功率
- IDR恢复成功率
- Decoder reset恢复成功率
- Reconnect恢复成功率

## 回归门槛

基于以上基线，设定以下回归门槛：

| 指标类别 | 基线值 | 允许劣化 | 回归门槛 |
|---------|--------|---------|---------|
| NACK事件频率 | 11.52 events/s | +10% | ≤12.67 events/s |
| 关键帧请求频率 | 2.225 requests/s | -20% (改善) | ≤1.78 requests/s |
| 关键帧平均间隔 | 168ms | +50% (改善) | ≥252ms |
| 帧丢弃频率 | 0.246 drops/s | +10% | ≤0.271 drops/s |
| 恢复决策频率 | 56.00 decisions/s | +5% | ≤58.80 decisions/s |
| cooldownSuppressed占比 | 30.2% | -50% (改善) | ≤15.1% |
| coalesced占比 | 8.1% | 持平 | ~8% |
| requestKeyframe占比 | 0.4% | 持平 | ~0.4% |

**关键改善目标**：
1. **降低关键帧请求频率**：从2.225 requests/s降低到≤1.78 requests/s（减少20%）
2. **增加关键帧请求间隔**：从168ms增加到≥252ms（增加50%）
3. **降低cooldown抑制率**：从30.2%降低到≤15.1%（减少50%）
4. **保持或降低帧丢弃率**：≤0.271 drops/s

## Replay Matrix

**需要冻结的测试用例**：
- coordinator_tests/ 所有测试用例的预期输出
- session/policy_tests/recovery_integration.rs 的预期行为
- 当前runtime trace中的恢复事件序列

**回归测试要求**：
- 所有现有测试用例必须通过
- 允许预期输出更新，但需人工审核
- 新实现的恢复事件序列应与基线序列在语义上等价

## 数据质量说明

**已采集**：
- ✅ NACK统计（事件数、重试次数、频率）
- ✅ 关键帧请求统计（数量、间隔、频率）
- ✅ 帧丢弃统计（数量、原因、频率）
- ✅ 恢复决策统计（数量、动作分布、频率）
- ✅ 会话时长和事件频率

**缺失**：
- ❌ 恢复延迟（NACK→IDR、IDR→解码完成等）
- ❌ 恢复成功率（各层级的成功率）
- ❌ 场景化指标（Home/Cloud/Relay分别的基线）
- ❌ 资源使用（CPU、内存、网络带宽）

**后续补充**：
- 需要在实际运行环境中采集恢复延迟和成功率
- 需要分场景（Home/Cloud/Relay）采集基线
- 需要采集资源使用基线（CPU/内存/网络）
