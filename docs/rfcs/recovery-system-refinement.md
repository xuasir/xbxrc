# 恢复系统精细化修复方案

## 元信息
- RFC编号: recovery-system-refinement
- 创建日期: 2026-04-17
- 状态: completed
- Completion: 已完成

## Context

在实施新恢复层（SimplifiedRecoveryCoordinator）的初步修复后，通过代码审查发现了几个系统性问题：

1. **NACK预算语义混乱**：在budget.rs定义预算后，nack_scheduler.rs又做了一层"快速判死"，导致anchor/keyframe包直接0预算，违背了"至少一次有效repair尝试"的设计意图
2. **队列控制职责重叠**：decoder和pacer都根据host_cadence_phase收紧队列，导致双重背压
3. **host cadence与release cadence混用**：用视频流帧率替换了真实host刷新率，导致压力判断失真
4. **retry_count语义不清**：retry=0就触发expiredRetryBudget，说明首发和重试的计数语义不统一

这些问题导致：
- NACK局部修复完全失效（高价值包直接放弃）
- 恢复期关键帧刚解码就被队列溢出丢弃
- Host压力判断不准确，影响cadence_phase决策

## Goals

1. **统一NACK预算语义**：只保留budget.rs的主预算，删除scheduler的二次判死逻辑
2. **明确retry_count语义**：确保至少有一次有效repair尝试
3. **拆分队列控制职责**：pacer主控host适配，decoder只做轻量缓冲
4. **分离host cadence和release cadence**：真实观测用于压力判断，限速用于消费控制

## Non-Goals

- 不改变episode生命周期管理
- 不改变SimplifiedRecoveryCoordinator的状态机
- 不改变H.264 bootstrap gate的验证逻辑

## 实施计划

### 阶段1：修复NACK预算语义（优先级最高）

#### 问题分析
当前实现在两处定义预算：
1. `budget.rs:retry_budget()` - 主预算：anchor=3, supply=2, disposable=0
2. `nack_scheduler.rs:calculate_effective_retry_budget_static()` - 二次预算：anchor/keyframe直接返回0

这导致高价值包（anchor）完全放弃NACK修复，违背设计意图。

#### 修复方案
**删除nack_scheduler.rs的动态预算逻辑**：
1. 移除`calculate_effective_retry_budget_static()`函数
2. 移除`NackSchedulerConfig`中的`hard_retry_limit`字段
3. 恢复使用`max_retry_count`作为唯一预算来源
4. 在`tick()`中直接使用`pending.max_retry_count`进行预算判定

**明确retry_count语义**：
- 选项A：`retry_count`只统计重试（不含首发），预算判定用`>=`
- 选项B：`retry_count`包含首发，预算判定改用`>`
- 推荐选项A，语义更清晰

#### 关键文件
- `crates/xbxengine/core/src/transport/rtc/stream/nack_scheduler.rs`
  - 删除`calculate_effective_retry_budget_static()`（约50行）
  - 简化`tick()`中的预算判定逻辑（503-525行）
  - 移除`hard_retry_limit`字段初始化

#### 验证点
- NACK事件中，anchor包至少有1次重试（retry_count=1时才expiredRetryBudget）
- 不再出现retry_count=0就expiredRetryBudget的情况
- NACK恢复成功率>0%（当前为0%）

---

### 阶段2：拆分host cadence和release cadence

#### 问题分析
当前实现用视频流帧率替换了真实host刷新率：
```rust
// pacer/actor.rs detect_video_frame_interval()
let frame_interval_ms = (1_000.0 / video_fps).round() as u64;
// 直接设置为refresh_interval_ms
```

这导致：
1. 144Hz屏幕被误判为60fps（16.67ms），压力判断失真
2. `cadence_lag_ratio`、`display_fps`等指标基于错误的baseline
3. Owner无法看到真实host压力

#### 修复方案
**在pacer中分离两个概念**：
1. `host_refresh_interval_ms` - 真实host/display刷新间隔，用于压力判断
2. `release_interval_ms` - 消费/release限速间隔，可按视频帧率设置

**修改resolve_host_pacing_context()**：
```rust
pub struct HostPacingContext {
    pub host_refresh_interval_ms: u64,  // 真实host刷新率（如6.94ms for 144Hz）
    pub release_interval_ms: u64,       // release限速间隔（如16.67ms for 60fps视频）
    pub cadence_phase: Option<String>,
    // ... 其他字段
}

fn resolve_host_pacing_context(...) -> HostPacingContext {
    // 1. 获取真实host刷新率（从display_fps或系统查询）
    let host_refresh_interval_ms = if stats.display_fps > 0.0 {
        (1_000.0 / stats.display_fps).round() as u64
    } else {
        16 // 默认60Hz
    };
    
    // 2. 计算release限速间隔（基于视频流帧率）
    let release_interval_ms = detect_video_frame_interval(stats)
        .unwrap_or(host_refresh_interval_ms);
    
    HostPacingContext {
        host_refresh_interval_ms,
        release_interval_ms,
        // ...
    }
}
```

**修改压力判断逻辑**：
- `cadence_lag_ratio`、`display_fps`等只使用`host_refresh_interval_ms`
- `resolve_host_release_wait_duration()`使用`release_interval_ms`进行限速

#### 关键文件
- `crates/xbxengine/core/src/media/video/pacer/actor.rs`
  - 修改`HostPacingContext`结构（约10行）
  - 修改`resolve_host_pacing_context()`（约30行）
  - 修改`resolve_host_release_wait_duration()`（约10行）

#### 验证点
- 144Hz屏幕上，`host_refresh_interval_ms`应为6.94ms
- 60fps视频流，`release_interval_ms`应为16.67ms
- `display_fps`应为144，不是60
- `cadence_lag_ratio`基于真实host刷新率计算

---

### 阶段3：拆分队列控制职责

#### 问题分析
当前实现中，decoder和pacer都根据`host_cadence_phase`调整队列容量：
- Decoder: starved→1, priming→2, steady→3, idle→4
- Pacer: starved→1, priming→2, 默认→3

双重收紧导致：
1. Starved时，decoder队列=1 + pacer队列=1，总缓冲只有2帧
2. 恢复期关键帧刚解码就被挤出队列
3. 背压过度，影响恢复效果

#### 修复方案
**明确职责分工**：
- **Pacer主控host适配**：根据`cadence_phase`动态调整队列，响应host压力
- **Decoder只做轻量缓冲**：固定容量（如3帧），只防止短时爆发，不跟随cadence收紧

**删除decoder的动态容量调整**：
1. 移除`adjust_queue_capacity_from_cadence()`方法
2. 恢复`MAX_DECODED_FRAME_QUEUE_LEN = 3`为固定值
3. 移除actor.rs中的cadence_phase读取和调整调用

**保留pacer的动态容量**：
- Pacer继续根据cadence_phase调整队列（已有逻辑）

#### 关键文件
- `crates/xbxengine/core/src/media/video/decode/video_decode.rs`
  - 删除`adjust_queue_capacity_from_cadence()`方法（约15行）
  - 移除`max_decoded_frame_queue_len`字段，恢复使用常量
  - 恢复`decoded_frame_queue_is_full()`和`enqueue_decoded_frame()`使用常量
- `crates/xbxengine/core/src/media/video/decode/actor.rs`
  - 删除cadence_phase读取和调整调用（约10行）

#### 验证点
- Decoder队列容量固定为3
- Starved时，总缓冲=decoder(3) + pacer(1) = 4帧
- 队列溢出次数应减少（不会因双重收紧而过度丢帧）

---

### 阶段4：恢复期队列策略（可选优化）

#### 问题分析
当前队列策略不区分正常运行和恢复期：
- 恢复期（RebuildingSupply/SupplyStarved）关键帧刚解码就被outputQueueOverflow丢弃
- 导致恢复失败，触发新一轮关键帧请求

#### 修复方案
**在decoder中增加恢复期检测**：
```rust
fn is_in_recovery_phase(stats: &XbxEngineMediaRuntimeStats) -> bool {
    // 检查owner状态或recovery信号
    stats.recovery_state == "RebuildingSupply" 
        || stats.recovery_state == "SupplyStarved"
        || stats.transport_await_recovery_keyframe
}

fn enqueue_decoded_frame(...) {
    let max_capacity = if is_in_recovery_phase(stats) {
        MAX_DECODED_FRAME_QUEUE_LEN_RECOVERY  // 如5帧
    } else {
        MAX_DECODED_FRAME_QUEUE_LEN  // 3帧
    };
    // ... 使用max_capacity
}
```

#### 关键文件
- `crates/xbxengine/core/src/media/video/decode/video_decode.rs`
  - 添加`is_in_recovery_phase()`辅助函数
  - 修改`enqueue_decoded_frame()`使用动态容量

#### 验证点
- 恢复期decoder队列容量扩展到5帧
- 关键帧解码后不会立即被outputQueueOverflow丢弃
- 恢复成功率提升

---

## 实施顺序

1. **阶段1（NACK预算）** - 立即实施，阻塞局部修复能力
2. **阶段2（cadence拆分）** - 次优先，影响压力判断准确性
3. **阶段3（队列职责）** - 第三优先，减少双重背压
4. **阶段4（恢复期策略）** - 可选优化，视前三阶段效果决定

每个阶段独立验证，确认效果后再进入下一阶段。

## 回滚策略

- **阶段1**：保留旧的`calculate_effective_retry_budget_static()`代码（注释），通过feature flag切换
- **阶段2**：保留单一`refresh_interval_ms`字段，通过配置选择使用host还是video帧率
- **阶段3**：保留`adjust_queue_capacity_from_cadence()`方法（注释），通过配置启用/禁用
- **阶段4**：恢复期容量扩展通过常量控制，可快速回退到固定值

## 风险与缓解

### 风险1：NACK预算放宽后网络负载增加
- **缓解**：监控NACK发送频率，如超过阈值则调整budget.rs的基础预算
- **回滚**：恢复快速判死逻辑

### 风险2：Decoder队列固定后仍有溢出
- **缓解**：先实施阶段2（cadence拆分），减少误判导致的过度收紧
- **回滚**：恢复动态容量调整

### 风险3：Host cadence拆分后压力判断仍不准
- **缓解**：增加日志，验证`host_refresh_interval_ms`和`release_interval_ms`的实际值
- **回滚**：恢复单一`refresh_interval_ms`

## 后续工作

完成4个阶段后，考虑：
1. 补充端到端测试（高RTT、恢复期、高刷屏幕场景）
2. 优化recovery coordinator入口合同（区分repairable-gap和need-sync-point）
3. 限制关键帧风暴（增加有效keyframe in-flight判定）
4. 恢复完成定义绑定到present（不只是decode）

## 参考

- 原始修复计划：`/Users/guo.xu/.claude/plans/stateful-toasting-snowflake.md`
- 修复清单：`docs/references/sdl3-cutover-notes.md`
- 相关代码：
  - `crates/xbxengine/core/src/transport/rtc/stream/nack_scheduler.rs`
  - `crates/xbxengine/core/src/media/video/ingress/budget.rs`
  - `crates/xbxengine/core/src/media/video/decode/video_decode.rs`
  - `crates/xbxengine/core/src/media/video/pacer/actor.rs`
