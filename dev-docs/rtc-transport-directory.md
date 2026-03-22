# RTC Transport 目录说明

本文说明当前 `crates/xbxengine/core/src/transport/rtc/` 的稳定目录结构、职责边界和维护约束。

目标不是描述一套理想化未来目录，而是解释**现在这套代码应该怎么理解、怎么继续维护**。

## 1. 总原则

当前 RTC transport 目录按 5 类职责理解：

1. 主线编排
2. 状态归约
3. 策略与调度
4. 边界适配
5. 域算法

一句话约束：

- `session/*` 决定主线怎么流转
- `projection/*` 决定状态怎么归约
- `policy/*` 决定 proposal 和 planner
- `connection/*`、`media/*`、`protocol/*` 负责边界接入
- `recovery/*`、`bwe/*` 负责域算法

不要再把“目录是否整齐”当作核心目标。真正要守住的是**主权边界**。

## 2. 顶层目录总览

当前稳定目录：

```text
transport/rtc/
  stack.rs
  facts.rs
  events.rs
  stats.rs
  session/
  projection/
  policy/
  executor/
  connection/
  media/
  protocol/
  recovery/
  bwe/
  sdp/
  pipeline/
```

各层意义如下：

### 2.1 `stack.rs`

`[stack.rs](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stack.rs)`

这是 RTC transport 的对外装配入口。

负责：

1. 启动和持有 runtime
2. 装配 `connection`、`media`、`session`
3. 接收边界输入并写入 fact
4. 执行 session 产出的 command
5. 对外实现 `XbxMediaStackPort`

不负责：

1. 自己做 recovery 判定
2. 自己做 BWE 判定
3. 保存多份平行状态机

维护要求：

1. 允许在这里做装配和 effect dispatch
2. 不允许在这里根据共享状态直接决定 `reconnect / keyframe / decoder reset / REMB`

### 2.2 `facts.rs`

`[facts.rs](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/facts.rs)`

统一事实模型。

负责：

1. `PeerFact`
2. `MediaFact`
3. `TimerFact`
4. `TransportFact`
5. `TransportCommand`
6. `CommandResultFact`

维护要求：

1. 这里只表达“发生了什么”或“要执行什么”
2. 不要把策略判断塞进 fact 命名里
3. 新增 fact 之前先确认是不是投影字段或诊断字段，而不是事实

### 2.3 `events.rs`

`[events.rs](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/events.rs)`

历史兼容事件模型。

它不是当前主线中心，但仍可作为兼容层存在。

### 2.4 `stats.rs`

`[stats.rs](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stats.rs)`

RTC 通用时间/统计辅助。

## 3. 主线编排层

### 3.1 `session/`

`session/` 是当前 RTC transport 的**主线目录**。

文件：

```text
session/
  mod.rs
  actor.rs
  mailbox.rs
  clock.rs
  policy.rs
```

#### `actor.rs`

`[actor.rs](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/session/actor.rs)`

主线 actor。

负责：

1. 消费 `TransportFact`
2. 推动各类 projection 更新
3. 生成 `TransportSnapshot`
4. 调用 `SessionPolicyHook`

不负责：

1. 直接操作 peer connection
2. 直接发 RTCP
3. 直接改 decoder

#### `mailbox.rs`

`[mailbox.rs](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/session/mailbox.rs)`

串行事实队列。

它的价值不是复杂逻辑，而是保证主线按统一顺序消费事实。

#### `clock.rs`

`[clock.rs](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/session/clock.rs)`

主线时钟抽象。

主要用途：

1. 降低时间依赖对测试的耦合
2. 避免业务逻辑散落 `now_ms()` 调用

#### `policy.rs`

`[policy.rs](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/session/policy.rs)`

这是当前真正的主线策略收口点。

负责：

1. 从 snapshot 生成 recovery proposal
2. 从 snapshot 生成 reconnect proposal
3. 从 snapshot 生成 BWE proposal
4. 调用 planner 合并 proposal
5. 产出最终 `TransportCommand`

这是整个目录里最重要的边界之一：

- “是否做”优先写在这里
- 不是写到 `connection/service.rs`
- 也不是写到 `media/service.rs`

## 4. 状态归约层

### 4.1 `projection/`

`projection/` 负责把事实归并成稳定状态。

文件：

```text
projection/
  mod.rs
  snapshot.rs
  connection.rs
  media.rs
  recovery.rs
  bwe.rs
  diagnostics.rs
```

#### `connection.rs`

`[connection.rs](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/projection/connection.rs)`

连接投影。

负责：

1. lifecycle state
2. data channel state
3. transport path / rtt / loss 等连接级概览

#### `media.rs`

`[media.rs](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/projection/media.rs)`

媒体投影。

负责：

1. frame count
2. latest frame meta
3. ingress 决策相关概览

#### `recovery.rs`

`[recovery.rs](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/projection/recovery.rs)`

恢复投影。

负责：

1. latest diagnosis label
2. latest recovery-related observation time
3. command result 对 recovery 状态的回写

#### `bwe.rs`

`[bwe.rs](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/projection/bwe.rs)`

BWE 投影。

负责：

1. transport metrics sample 汇总
2. latest REMB 目标
3. BWE sample tick 视图

#### `diagnostics.rs`

`[diagnostics.rs](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/projection/diagnostics.rs)`

诊断投影。

负责：

1. latest label
2. latest summary
3. 对外 trace/read model 的轻量视图

#### `snapshot.rs`

`[snapshot.rs](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/projection/snapshot.rs)`

聚合 snapshot。

要求：

1. policy 只读 snapshot
2. 不让 policy 到处读共享锁

## 5. 策略与调度层

### 5.1 `policy/`

`policy/` 现在是**主线 proposal/planner 层**，不是域算法全集。

文件：

```text
policy/
  mod.rs
  recovery.rs
  bwe.rs
  reconnect.rs
  planner.rs
```

#### `planner.rs`

`[planner.rs](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/policy/planner.rs)`

统一 planner。

负责：

1. proposal 合并
2. 优先级控制
3. 命令收口

当前约束：

1. `Reconnect` 优先于其它 proposal
2. recovery / BWE 不再各自直接执行

#### `recovery.rs`

`[recovery.rs](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/policy/recovery.rs)`

recovery proposal 数据结构。

#### `reconnect.rs`

`[reconnect.rs](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/policy/reconnect.rs)`

reconnect proposal 数据结构。

#### `bwe.rs`

`[bwe.rs](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/policy/bwe.rs)`

BWE proposal 数据结构。

## 6. 执行层

### 6.1 `executor/`

当前 `executor/` 刻意保持很小。

文件：

```text
executor/
  mod.rs
  peer.rs
```

#### `peer.rs`

`[peer.rs](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/executor/peer.rs)`

当前唯一独立收口出来的执行面。

负责：

1. reconnect candidate staging

为什么没有 `control/decoder/rtcp`：

1. 这轮清理后并没有足够复杂度支撑单独文件
2. 不为目录对称性保留空壳

如果未来这些 effect 再次增长出独立复杂度，再拆。

## 7. 边界适配层

### 7.1 `connection/`

`connection/` 是 peer / ICE / data channel 边界目录。

文件：

```text
connection/
  mod.rs
  service.rs
  control_channel.rs
  data_channel_bootstrap.rs
  io_runtime.rs
  runtime_state.rs
  transport_metrics.rs
```

#### `service.rs`

`[service.rs](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/connection/service.rs)`

这是连接边界协调器，不再是系统主线。

负责：

1. peer event / read / write 边界协调
2. transport metrics 原始采样
3. data channel 生命周期处理
4. 连接 effect helper

不负责：

1. BWE 主决策
2. recovery 主决策
3. reconnect 主流程编排

#### `control_channel.rs`

`[control_channel.rs](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/connection/control_channel.rs)`

control channel 相关边界逻辑。

#### `data_channel_bootstrap.rs`

`[data_channel_bootstrap.rs](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/connection/data_channel_bootstrap.rs)`

data channel 建立后的 bootstrap 边界逻辑。

#### `io_runtime.rs`

`[io_runtime.rs](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/connection/io_runtime.rs)`

底层 IO 运行时辅助。

#### `runtime_state.rs`

`[runtime_state.rs](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/connection/runtime_state.rs)`

连接边界局部状态。

重要约束：

1. 它可以存在
2. 但不能重新变回全局共享真相源

#### `transport_metrics.rs`

`[transport_metrics.rs](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/connection/transport_metrics.rs)`

只做 transport stats 原始解析，不做策略升级。

### 7.2 `media/`

`media/` 是媒体边界目录。

文件：

```text
media/
  mod.rs
  service.rs
  adapter_types.rs
  frame_cadence.rs
  nack_scheduler.rs
  packet_router.rs
  packet_types.rs
  runtime_state.rs
  sink.rs
  video_source/*
```

#### `service.rs`

`[service.rs](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/media/service.rs)`

负责：

1. raw packet / frame 路由
2. sink 桥接
3. packet observation 汇总

不负责：

1. recovery 决策升级
2. reconnect 判定

#### `packet_router.rs`

`[packet_router.rs](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/media/packet_router.rs)`

RTP / repair route 分类。

#### `packet_types.rs`

`[packet_types.rs](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/media/packet_types.rs)`

媒体边界包模型。

#### `nack_scheduler.rs`

`[nack_scheduler.rs](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/media/nack_scheduler.rs)`

这是媒体 repair 辅助，不是会话级策略层。

#### `sink.rs`

`[sink.rs](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/media/sink.rs)`

媒体 sink 与 RTCP port trait 定义。

#### `video_source/*`

这是媒体输入接入目录。

职责：

1. RTP/RTX/FEC 等视频包进入
2. 组帧与基础观测
3. 向上游送媒体事实

### 7.3 `protocol/`

`protocol/` 保留当前主线仍需要的协议编码/状态。

文件：

```text
protocol/
  mod.rs
  data_channel_state.rs
```

`[data_channel_state.rs](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/protocol/data_channel_state.rs)`

当前保留内容：

1. 输入队列状态
2. 输入流打包
3. 视频 metadata frame 构造

不再保留：

1. recovery 执行辅助
2. data channel availability 发布链
3. 已无调用方的旧 rumble / handshake/control 壳

## 8. 域算法层

### 8.1 `recovery/`

`recovery/` 是 recovery 域算法目录。

文件：

```text
recovery/
  mod.rs
  coordinator.rs
  diagnosis.rs
  escalation.rs
  policy.rs
  signal.rs
  startup.rs
```

#### `coordinator.rs`

`[coordinator.rs](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/coordinator.rs)`

recovery 域核心规则中心。

它可以继续留在 `recovery/`，不需要为了形式搬到 `policy/`。

#### `diagnosis.rs`

`[diagnosis.rs](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/diagnosis.rs)`

负责 observation/signal 到 diagnosis 的映射。

#### `escalation.rs`

`[escalation.rs](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/escalation.rs)`

负责 action escalation 规则。

#### `policy.rs`

`[policy.rs](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/policy.rs)`

负责 recovery 场景 profile。

#### `signal.rs`

`[signal.rs](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/signal.rs)`

负责 recovery signal 模型。

#### `startup.rs`

`[startup.rs](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/startup.rs)`

负责 startup / recovering phase 域逻辑。

### 8.2 `bwe/`

`bwe/` 是 BWE 域算法目录。

文件：

```text
bwe/
  mod.rs
  evaluator.rs
  policy.rs
```

#### `policy.rs`

`[policy.rs](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/bwe/policy.rs)`

保留 BWE 域算法和测试。

#### `evaluator.rs`

`[evaluator.rs](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/bwe/evaluator.rs)`

现在已经缩成主线真正需要的 `RtcBweEvaluation` 结构，不再承载旧运行时状态机壳。

## 9. 协商与协议适配层

### 9.1 `sdp/`

`sdp/` 负责 SDP 适配与协商策略。

文件：

```text
sdp/
  mod.rs
  answer_adapter.rs
  candidate_adapter.rs
  offer_policy.rs
  policy.rs
  types.rs
```

这是稳定域目录，不属于 transport 主线编排层。

## 10. 辅助/过渡层

### 10.1 `pipeline/`

`pipeline/` 还存在，但它现在不是主线中心。

文件：

```text
pipeline/
  mod.rs
  observation.rs
  session.rs
  supervisor.rs
```

当前定位：

1. 媒体接入辅助
2. 媒体挂载/生命周期辅助
3. 媒体观测辅助

明确禁止：

1. 重新在这里放 recovery driver
2. 重新在这里放 scheduler 主权
3. 重新在这里让 session loop 直接执行恢复动作

换句话说：

- `pipeline/` 可以存在
- 但它不能再是“旧主线”的化身

## 11. 维护约束

后续改这个目录时，按下面这套规则判断：

### 11.1 新代码应该放哪里

1. 新的主线编排：
   - `stack.rs`
   - `session/*`
   - `projection/*`
   - `policy/planner.rs`

2. 新的边界接入：
   - `connection/*`
   - `media/*`
   - `protocol/*`
   - `sdp/*`

3. 新的域算法：
   - `recovery/*`
   - `bwe/*`

4. 新的执行收口：
   - `executor/*`
   - 或 `stack.rs` 中明确的 effect dispatch

### 11.2 明确禁止的写法

1. 在 `connection/service.rs` 中直接根据共享状态决定重连、keyframe、decoder reset、REMB
2. 在 `media/service.rs` 中直接升级恢复动作
3. 在 `pipeline/*` 中重新长出 recovery 执行主权
4. 在多个目录里同时维护同一种主状态
5. 为了目录对称性保留空壳文件

### 11.3 什么时候说明边界又坏了

如果一个文件同时承担下面 3 类及以上职责，就要警惕：

1. 事实接入
2. 状态归约
3. 策略判定
4. 命令执行

出现这种情况，基本就说明主线又开始回退成“大循环”了。

## 12. 阅读顺序建议

第一次接手这块代码，建议按下面顺序读：

1. [stack.rs](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stack.rs)
2. [facts.rs](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/facts.rs)
3. [session/actor.rs](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/session/actor.rs)
4. [session/policy.rs](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/session/policy.rs)
5. [policy/planner.rs](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/policy/planner.rs)
6. [projection/snapshot.rs](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/projection/snapshot.rs)
7. [connection/service.rs](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/connection/service.rs)
8. [media/service.rs](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/media/service.rs)
9. [recovery/coordinator.rs](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/coordinator.rs)
10. [bwe/policy.rs](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/bwe/policy.rs)

这样最容易先看清主线，再看边界，最后看域规则。
