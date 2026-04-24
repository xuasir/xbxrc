# 恢复层简化 - 迁移指南

> 本文档说明如何从旧的恢复系统迁移到新的简化恢复系统

## 概述

新的简化恢复系统已经完全实现并测试，提供了更简单、更高效的恢复逻辑。本指南说明如何在新代码中使用新系统，以及如何逐步迁移现有代码。

## 新旧系统对比

| 特性 | 旧系统 | 新系统 | 改善 |
|------|--------|--------|------|
| 代码行数 | ~4,658行 | ~1,270行 | -73% |
| 架构层次 | 4层（Signal→Diagnosis→Escalation→Coordinator） | 3层（Observation→StateMachine→ActionCoordinator） | -25% |
| 状态跟踪字段 | 20+个 | 5个 | -75% |
| 时间窗口 | 10+个重叠窗口 | 4个独立超时 | -60% |
| 预算跟踪 | 3级预算（keyframe/decoder_reset/reconnect） | 仅reconnect预算 | -67% |
| 恢复状态 | 分散在多个字段 | 单一状态机 | 统一 |

## 新系统架构

### 核心模块

1. **observation.rs** - 统一观察层
   - `RecoveryObservation`: 统一的恢复观察类型
   - `RecoverySeverity`: 6级严重性分类
   - 直接从`VideoEscalationReason`映射到严重性

2. **state_machine.rs** - 恢复状态机
   - `RecoveryState`: 5个恢复状态
   - `RecoveryBudget`: 预算跟踪（仅reconnect）
   - `StateTimeouts`: 场景化超时配置
   - `RecoveryStateMachine`: 状态机实现

3. **action_coordinator.rs** - 动作协调器
   - `ActionCoordinator`: 基于状态的动作分发
   - `RecoveryDecision`: 决策结果
   - In-flight门控（IDR/decoder reset/reconnect）

4. **simplified_coordinator.rs** - 适配器
   - `SimplifiedRecoveryCoordinator`: 兼容旧接口的适配器
   - `SimplifiedRecoveryDecision`: 简化的决策结果

### 5个恢复状态

```
Healthy (健康)
  ↓ 检测到丢包
LocalRepair (本地修复 - NACK活跃)
  ↓ NACK失败或低repairability
FrameRecovery (帧恢复 - RFI/IDR已请求)
  ↓ IDR失败或解码问题
DecoderRecovery (解码恢复 - decoder reset进行中)
  ↓ decoder reset失败或传输证据
TransportRecovery (传输恢复 - 重连)
  ↓ 成功
Healthy
```

### 防风暴机制

新系统通过以下机制防止风暴，**无需预算限制**：

1. **In-flight门控**：
   - IDR已请求但未解码时，阻止新IDR请求（coalesce）
   - Decoder reset进行中时，阻止新reset
   - Reconnect进行中时，阻止新reconnect

2. **状态门控**：
   - 进入FrameRecovery状态后，不再每帧重触发IDR请求
   - 进入DecoderRecovery状态后，不再每帧重触发reset请求
   - 状态转换单向，避免反复横跳

3. **最小间隔**：
   - IDR重试最小间隔50ms（仅防死锁，不限总次数）
   - Decoder reset无最小间隔（超时即升级）

4. **Reconnect严格预算**：
   - 每epoch仅1次reconnect（防止重连风暴）

## 使用新系统

### 在新代码中使用

```rust
use crate::transport::rtc::recovery::{
    simplified_coordinator::SimplifiedRecoveryCoordinator,
    escalation::VideoEscalationReason,
    policy::RecoveryScenarioProfile,
};

// 创建协调器
let profile = RecoveryScenarioProfile::from_kind(profile_kind);
let mut coordinator = SimplifiedRecoveryCoordinator::new(profile, recovery_epoch);

// 处理恢复信号
let decision = coordinator.on_signal(
    VideoEscalationReason::WaitKeyframe,
    "waitKeyframe".to_string(),
    observed_at_ms,
);

// 检查是否需要执行动作
if decision.should_execute() {
    if decision.is_keyframe_request() {
        // 执行IDR请求
        send_keyframe_request();
    } else if decision.is_decoder_reset_request() {
        // 执行decoder reset
        reset_decoder();
    } else if decision.is_reconnect_request() {
        // 执行reconnect
        reconnect();
    }
}

// 通知恢复完成
coordinator.on_clean_anchor(has_stable_output);
```

### 迁移现有代码

#### 步骤1：替换导入

```rust
// 旧代码
use crate::transport::rtc::recovery::{
    coordinator::RecoveryCoordinator,
    signal::VideoIngressSignal,
    diagnosis::diagnose_ingress_signal,
};

// 新代码
use crate::transport::rtc::recovery::{
    simplified_coordinator::SimplifiedRecoveryCoordinator,
    escalation::VideoEscalationReason,
};
```

#### 步骤2：替换协调器创建

```rust
// 旧代码
let coordinator = RecoveryCoordinator::new(
    profile.escalation_config(),
    startup_grace,
    stream_started_at,
);

// 新代码
let coordinator = SimplifiedRecoveryCoordinator::new(
    profile,
    recovery_epoch,
);
```

#### 步骤3：替换信号处理

```rust
// 旧代码
let signal = VideoIngressSignal::from_decision(&decision);
let diagnosis = diagnose_ingress_signal(signal);
let proposal = coordinator.on_reason(
    diagnosis.reason,
    diagnosis.label,
    phase,
    profile,
    recovery_epoch,
    runtime_stats,
    observed_at_ms,
);

// 新代码
let reason = match &decision {
    IngressDecision::WaitKeyframe => VideoEscalationReason::WaitKeyframe,
    IngressDecision::Reconfigure => VideoEscalationReason::Reconfigure,
    _ => VideoEscalationReason::WaitKeyframe,
};
let decision = coordinator.on_signal(
    reason,
    reason.label().to_string(),
    observed_at_ms,
);
```

#### 步骤4：替换决策处理

```rust
// 旧代码
match proposal.decision.action {
    RecoveryAction::RequestKeyframe => {
        // 执行IDR请求
    }
    RecoveryAction::CoalescedKeyframeInFlight => {
        // 已有IDR在飞行中，跳过
    }
    _ => {}
}

// 新代码
if decision.should_execute() {
    if decision.is_keyframe_request() {
        // 执行IDR请求
    }
}
// coalesce自动处理，无需显式检查
```

## 已迁移的模块

- ✅ `pipeline/session_loop.rs` - 信号入口点，已移除signal/diagnosis依赖

## 待迁移的模块

以下模块仍使用旧系统，可以逐步迁移：

- `session/policy.rs` - 主编排点（3000+行，建议分阶段迁移）
- `session/facts.rs` - 输入组装
- `session/expensive_recovery_gate.rs` - 门控执行
- `policy/recovery.rs` - 恢复决策账本
- `policy/scheduling.rs` - 调度策略
- 其他15+个文件

## 迁移优先级建议

1. **高优先级**（核心路径）：
   - `session/policy.rs` - 主编排点
   - `session/facts.rs` - 输入组装

2. **中优先级**（辅助功能）：
   - `policy/recovery.rs` - 决策账本
   - `policy/scheduling.rs` - 调度策略

3. **低优先级**（边缘功能）：
   - BWE层、Connection层、Stack层等

## 验证和测试

### 单元测试

新系统包含17个单元测试，覆盖核心逻辑：
- observation.rs: 5个测试
- state_machine.rs: 4个测试
- action_coordinator.rs: 5个测试
- simplified_coordinator.rs: 3个测试

运行测试：
```bash
cargo test --package xbxengine recovery::
```

### 集成测试

建议在迁移后进行以下验证：
1. 编译通过
2. 单元测试通过
3. 集成测试通过
4. 运行时验证（对比基线指标）

### 回归门槛

根据基线指标，设定以下回归门槛：
- 关键帧请求频率：≤1.78 requests/s（改善20%）
- 关键帧平均间隔：≥252ms（改善50%）
- cooldown抑制率：≤15.1%（改善50%）
- 帧丢弃频率：≤0.271 drops/s（允许劣化10%）

## 常见问题

### Q: 新系统是否向后兼容？

A: 是的。SimplifiedRecoveryCoordinator提供了与旧RecoveryCoordinator兼容的接口，可以直接替换。

### Q: 新系统是否会影响性能？

A: 不会。新系统代码更简单，决策分支更少，理论上性能更好。

### Q: 如何回退到旧系统？

A: 只需将SimplifiedRecoveryCoordinator替换回RecoveryCoordinator即可，无需其他改动。

### Q: 新系统是否支持所有场景？

A: 是的。新系统保留了场景化策略（Home/Cloud/Relay），从profile读取超时参数。

### Q: 为什么移除了IDR/decoder reset的预算限制？

A: 通过in-flight门控和状态门控可以更有效地防止风暴，无需预算限制。这样可以实现更快的恢复（失败后立即重试）。

## 参考文档

- RFC: `docs/rfcs/recovery-layer-simplification.md`
- 基线指标: `docs/rfcs/recovery-layer-baseline-metrics.md`
- 代码位置: `crates/xbxengine/core/src/transport/rtc/recovery/`
