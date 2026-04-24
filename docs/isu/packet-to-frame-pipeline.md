# 数据包到产帧：局部能力梳理

更新时间：2026-04-13

本文梳理从 RTP 数据包进入系统到产出可解码帧这段局部链路的现有实现，以及与 Moonlight FEC 模型对比后识别出的能力缺口。目的不是立即改代码，而是先把这段链路至少要提供什么能力说清楚，作为后续改进的基准。

## 1. 链路全貌

UDP 包到达后经过以下阶段：

1. RtcVideoSourceSink.on_raw_packet() [sink.rs] — 包类型识别、优先级分类、背压缓冲
2. mpsc channel
3. RtcVideoFrameSource.recv_frame_inner() [source.rs] — 序列号追踪、NACK 触发、jitter buffer、产帧决策

职责边界：入口是原始 UDP/RTP 包（含 RTX），出口是 AssembledVideoFrame（带 is_keyframe、media_dropped_packets、frame_importance 等元数据）。不包含解码、渲染、Pacer、RecoveryCoordinator 的升级决策。

## 2. 各阶段分析

### 2.1 包入口与优先级分类（sink.rs）

`RtcVideoSourceSink` 是 UDP 包进入处理流水线的第一道门，负责包类型识别、优先级分类、背压缓冲，然后通过 mpsc channel 交给 `recv_frame_inner`。

#### 现有能力

**包类型识别与规范化**

- `normalize_video_packet` 区分 `PrimaryVideo` 和 `RepairVideo` 两条路由
- RTX 包通过 `unpack_rtx_packet` 还原原始序列号、payload type、SSRC，以 `RtxReinject` 身份进入后续流程
- `RepairPrimaryPassThrough`：repair 路由上携带的 primary payload，通过 FID 映射修正 SSRC 后透传
- 无法识别的 repair payload 直接丢弃，不污染主流

**三级优先队列**

| 队列 | 容量上限 | 内容 | 满时策略 |
|------|----------|------|----------|
| `pending_priority_primary` | `priority_backlog_limit`（≤32） | IDR/SPS/PPS 包 | 丢新包 |
| `pending_repair` | `repair_backlog_limit`（≤32） | RTX 重传包 | 丢最旧 |
| `pending_best_effort` | `best_effort_backlog_limit`（≤8） | 普通 delta 包 | 丢最旧 |

`flush_pending()` 调度顺序：primary 优先 → repair（连续超过 4 个后强制插入 best_effort）→ best_effort。

**H.264 NAL 级优先识别**

`is_likely_h264_recovery_priority` 通过 NAL type 识别 IDR(5)/SPS(7)/PPS(8)，包括 STAP-A(24) 和 FU-A(28) 首片，这些包进 `PriorityPrimary` 队列。

**背压可观测性**

每次丢包都通过 `record_local_backpressure_drop` 记录，带 reason/detail/class/timestamp。

#### 缺口

**1. 没有时效性过滤**

`on_raw_packet` 入口没有任何基于 RTP timestamp 的时效性判断。旧帧的 RTX 包和当前帧的包得到完全相同的处理。高丢包+高重排场景下，大量已无意义的旧帧 RTX 包持续占用 repair 队列容量，挤压当前帧的包。

**2. FU-A 后续分片被降级为 BestEffort（正确性问题）**

`is_likely_h264_recovery_priority` 对 FU-A(28) 只识别 Start bit 为 1 的首片，后续分片的 Start bit 为 0，被分类为 `BestEffort`。在高背压场景下，IDR 帧的首包进了 `PriorityPrimary`，后续分片包却进了 `BestEffort` 并可能被丢弃或延迟，导致 IDR 帧在 `sample_builder` 里永远无法组装完整。

**3. `RepairPrimaryPassThrough` 的 IDR 包被错误降级（正确性问题）**

`classify_backpressure_class` 对 `RepairPrimaryPassThrough` 直接走 repair 分支，跳过了 `is_priority_primary_packet` 的 NAL type 检查。通过 repair 路由到达的 IDR 包会被放入 repair 队列而不是 primary 队列，优先级错误。

**4. `PriorityPrimary` 满时丢新包存在反直觉场景**

如果 primary 队列里积压的都是旧帧的 IDR 包，新来的当前帧 IDR 包反而被丢掉。两个队列的满时策略方向不一致（primary 丢新，repair 丢旧），没有统一的"当前帧优先"语义。

**5. 背压队列只在下一个包到来时排空**

`flush_pending` 是同步调用，只在 `on_raw_packet` 时触发。如果网络包到达速率突然降低（高丢包后短暂静默），背压队列里的包会滞留到下一个包到来才被消费，在包稀疏场景下引入额外延迟。

**6. `repair_burst_streak` 重置导致 best_effort 可能饥饿**

`repair_burst_streak` 在发出任意 primary 包时重置为 0。在 repair 包持续高频到来、primary 队列偶发有包的场景下，streak 会被频繁重置，best_effort 包需要等待的 repair 包数量远超 4 个，普通 delta 帧包可能被长期饿死。

**7. repair 队列没有"已无意义"的主动清理**

当上层处于 `waiting_for_recovery_keyframe` 状态时，pending_repair 里积压的 delta 帧 RTX 包即使恢复回来也会被丢弃。sink 层不感知这个状态，这些包仍然占用队列容量，延迟真正有用的 keyframe RTX 包进入。

#### 值得借鉴（Moonlight）

**架构差异是根本**

Moonlight 的 `VideoReceiveThreadProc` 是单线程同步流水线：收包 → 旧帧过滤 → 解密 → `RtpvAddPacket` → FEC 恢复 → 产帧，全程无跨线程 channel，不需要优先级队列。FEC 恢复是同步完成的，包进来凑够就立刻恢复，不存在"修复包挤占主流"的问题。

我们的跨线程 channel 设计带来了背压队列的必要性，也带来了上述缺口 2/3/5/6。这是架构层面的差异，不是参数调优能解决的。

**旧帧过滤（VideoStream.c:184-220）**

Moonlight 用 `NV_VIDEO_PACKET.frameIndex`（帧编号，单调递增整数）在解密前过滤旧帧包，不浪费 AES-GCM 开销。关键约束：过滤逻辑只做无状态丢弃，不基于未解密包修改任何状态（安全边界）。

我们的等效位置是 `on_raw_packet` 入口。可行性取决于 Xbox 协议包头是否携带帧级别标识——如果有，可在 `normalize_video_packet` 之后、入队之前过滤；如果没有，只能用 RTP timestamp 近似，需要处理 u32 回绕。

**帧边界即时清理（RtpVideoQueue.c:650 附近）**

新帧的包到来时，Moonlight 立即 `purgeListEntries` 清掉上一帧所有未处理的包并重置队列状态。我们依赖 `sample_builder` 的滑动窗口超时，旧帧包不会在新帧到来时立即清理，在高丢包场景下持续占用内存和队列容量。

**fast path 帧级路径锁定（RtpVideoQueue.c:queuePacket）**

Moonlight 用 `useFastQueuePath` 标志锁定：一旦某帧进入过 slow path（乱序），整帧都用 slow path 处理，避免同一帧内混用两种路径导致去重逻辑出错。我们的 `nack_window` 没有帧级别的路径锁定，乱序包到达后的去重行为依赖环形位图的全局状态，不区分帧边界。

### 2.2 序列号追踪与 OOS 检测（nack_window.rs）

`NackSequenceWindow` 是一个环形位图，维护 `last_consecutive`（最后连续收到的序列号）和 `end`（收到的最高序列号）。`missing_seq_numbers(skip_last_n)` 返回 `[last_consecutive+1, end-skip_last_n]` 区间内未收到的序列号，`skip_last_n` 给乱序包留出等待窗口。

#### 现有能力

- 标准环形位图，O(1) 更新，O(gap) 查询缺包
- `add()` 区分顺序包（推进 `end`）和 OOS 包（只更新 `last_consecutive`），不会把 OOS 包误当新 gap
- `missing_seq_numbers_in_range()` 支持指定范围查询，供 sample loss 路径使用
- `skip_last_n` 提供静态乱序容忍窗口

#### 缺口

**1. `skip_last_n` 是静态参数，不感知网络乱序程度**

乱序严重时 `skip_last_n` 不够大，误触发 NACK；乱序轻微时过大，延迟真正的丢包检测。没有任何机制根据观测到的 OOS 频率动态调整这个值。

**2. OOS 包到达后不通知 NackScheduler 取消对应 NACK**

`nack_window.add()` 在 OOS 包到达时更新 `last_consecutive`，但不会通知 `NackScheduler` 取消对应序列号的 pending NACK。已经在 pending 里的 NACK 会继续重试，直到 `resolve_sequence` 被调用。如果 OOS 包被背压丢弃（未进入 `recv_frame_inner`），`resolve_sequence` 永远不会被调用，这个 NACK 会一直重试到 deadline/maxAge 耗尽——发出了无意义的重传请求，浪费带宽。

**3. 没有帧级别的 OOS 状态追踪**

`nack_window` 是纯序列号级别的，不知道哪些序列号属于同一帧。OOS 包到达时无法判断"这个 OOS 包是否让某帧从不可恢复变成了可恢复"，也无法判断"当前帧是否已经进入过 OOS 路径"。这个信息对于 speculative 不可恢复预测非常重要（见 2.3 节）。

**4. 窗口大小固定，不感知当前帧的包数规模**

`size` 由 `log2_size_minus_6` 在初始化时确定，不会随帧大小变化。高分辨率帧（包数多）和低分辨率帧（包数少）使用相同的窗口大小，可能导致高分辨率场景下窗口不够用，或低分辨率场景下窗口过大浪费内存。

#### 值得借鉴（Moonlight）

**OOS 状态与 speculative 模式的联动（RtpVideoQueue.c:queuePacket）**

Moonlight 在 `queuePacket` 里，每次收到 OOS 包时设置 `receivedOosData = true` 并记录时间戳，同时关闭 speculative RFI 模式。只有在 `SPECULATIVE_RFI_COOLDOWN_PERIOD_US`（5 分钟）内没有再收到 OOS 包，才重新开启 speculative 模式。

这个机制的核心价值：**OOS 包的到达是网络乱序的直接证据，而乱序网络下 speculative 预测"帧不可恢复"极易误判**。我们没有等效的 OOS 状态追踪，`nack_window` 的 OOS 信息没有被传递给任何上层决策逻辑。

**`missingPackets` 实时维护（RtpVideoQueue.c:RtpvAddPacket）**

Moonlight 在每个包到达时实时维护 `missingPackets`：收到比当前最高序列号更高的包时 `missingPackets += gap`，收到之前缺失的包时 `missingPackets--`。这让它可以在任意时刻精确知道"当前 FEC block 还缺多少包"，从而做 speculative 预测（`missingPackets > totalPackets - neededPackets`）。

我们的 `nack_window` 也维护了类似信息，但只在 `maybe_run_nack_maintenance` 被调用时才查询，不是实时的，也没有被用于 speculative 预测。
