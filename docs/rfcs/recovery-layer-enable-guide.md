# 如何启用新恢复层

> 新恢复层已完全集成并默认启用

## 当前状态

**✅ 新恢复层已完全集成**：SimplifiedRecoveryCoordinator 已替换旧的 RecoveryCoordinator，成为默认恢复系统。

- ✅ 新系统已实现并测试（17个单元测试）
- ✅ 已集成到主恢复路径（session/policy.rs）
- ✅ 启动时会记录：`[Recovery] Using SimplifiedRecoveryCoordinator (new recovery system)`
- ✅ 无需配置，开箱即用

## 验证新系统已启用

启动应用后，查看日志确认：

```
[Recovery] Using SimplifiedRecoveryCoordinator (new recovery system)
```

运行时日志会显示新系统的状态转换：

```
[Recovery] State: Healthy
[Recovery] State transition: Healthy -> LocalRepair
[Recovery] IDR request (in-flight: false)
[Recovery] State transition: LocalRepair -> FrameRecovery
```

## 收集性能指标

使用 `analyze-runtime-logs` skill 分析运行时日志：

```bash
# 运行应用 ~14分钟，生成 runtime-trace-*.jsonl

# 分析恢复指标
python .claude/skills/analyze-runtime-logs/scripts/summarize_runtime_trace.py \
  runtime-logs/runtime-trace-*.jsonl \
  --categories state,decision,event \
  --domain recovery
```

### 目标指标

| 指标 | 旧系统基线 | 新系统目标 | 改善 |
|------|-----------|-----------|------|
| 关键帧请求频率 | 2.225 requests/s | ≤1.78 requests/s | -20% |
| 关键帧平均间隔 | 168ms | ≥252ms | +50% |
| Cooldown抑制率 | 30.2% | ≤15.1% | -50% |
| 帧丢弃频率 | 0.246 drops/s | ≤0.271 drops/s | +10%容忍 |

## 架构对比

### 旧系统（已移除）

- 4层架构：Signal → Diagnosis → Escalation → Coordinator
- ~4,658行代码
- 20+个状态跟踪字段
- 10+个重叠时间窗口
- 复杂的3级预算跟踪（keyframe/decoder_reset/reconnect）
- 4阶段transport-await

### 新系统（当前）

- 3层架构：Observation → StateMachine → ActionCoordinator
- ~1,270行代码（-73%）
- 5个状态跟踪字段（-75%）
- 4个独立超时（-60%）
- 仅reconnect预算限制（-67%）
- 统一状态机

## 核心改进

### 1. 防风暴机制

**无需预算限制**，通过3层门控防止风暴：

1. **In-flight门控**：IDR/decoder reset进行中时阻止新请求
2. **状态门控**：进入恢复状态后不再每帧重触发
3. **最小间隔**：IDR重试最小间隔50ms（仅防死锁）

### 2. 快速恢复

- IDR失败时立即重试（仅50ms最小间隔）
- Decoder reset失败时立即升级到reconnect
- 无预算限制，恢复更快

### 3. 简化状态机

5个恢复状态，单向转换：

```
Healthy → LocalRepair → FrameRecovery → DecoderRecovery → TransportRecovery → Healthy
```

## 技术细节

### 集成方式

新系统通过 `SimplifiedRecoveryAdapter` 适配到 `session/policy.rs`：

```rust
// session/policy.rs
let simplified_coordinator = SimplifiedRecoveryCoordinator::new(
    recovery_profile,
    recovery_epoch,
);
self.recovery_coordinator = SimplifiedRecoveryAdapter::new(simplified_coordinator);
```

适配器提供与旧系统兼容的接口：
- `propose_from_owner_signal()` - 生成恢复提案
- `acknowledge_clean_anchor()` - 确认恢复成功
- `acknowledge_stable_recovery()` - 确认稳定恢复
- `rollback_decoder_reset_burst_after_transport_family_defer()` - 空实现（新系统不需要）

### 文件变更

**新增文件**：
- `recovery/observation.rs` (240行) - 统一观察层
- `recovery/state_machine.rs` (380行) - 恢复状态机
- `recovery/action_coordinator.rs` (450行) - 动作协调器
- `recovery/simplified_coordinator.rs` (200行) - 简化coordinator
- `recovery/simplified_adapter.rs` (120行) - 适配器

**修改文件**：
- `session/policy.rs` - 使用SimplifiedRecoveryAdapter
- `pipeline/session_loop.rs` - 移除signal/diagnosis依赖

**标记为废弃**：
- `recovery/signal.rs` - 已被observation.rs替代
- `recovery/diagnosis.rs` - 已被observation.rs替代

## 参考文档

- RFC: `docs/rfcs/recovery-layer-simplification.md`
- 迁移指南: `docs/rfcs/recovery-layer-migration-guide.md`
- 基线指标: `docs/rfcs/recovery-layer-baseline-metrics.md`
