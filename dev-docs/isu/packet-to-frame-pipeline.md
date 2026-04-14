# 数据包到产帧：局部能力梳理

更新时间：2026-04-14

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

**环形位图基础结构**

- 位图容量：`size = 1 << (log2_size_minus_6 + 6)`，当前初始化为 `log2_size_minus_6 = 7`，即 `size = 8192`
- 存储结构：`Vec<u64>`，长度为 `1 << log2_size_minus_6`（当前 128 个 u64），每个 u64 覆盖 64 个序列号
- 位图操作：`set_received` / `del_received` / `get_received` 通过 `seq % size` 映射到位图位置，O(1) 复杂度
- 序列号回绕：所有比较和范围判断都使用 `wrapping_sub` / `wrapping_add`，正确处理 u16 回绕

**`add()` 方法的完整行为**

```rust
pub(super) fn add(&mut self, seq: u16)
```

- **首包初始化**：`!started` 时设置 `end = last_consecutive = seq`，标记 `started = true`
- **重复包过滤**：`seq == end` 时直接返回，不做任何更新
- **顺序包（seq > end）**：
  - 清空 `[end+1, seq)` 区间的位图（`del_received`），标记为未收到
  - 推进 `end = seq`
  - 如果 `seq == last_consecutive + 1`，推进 `last_consecutive = seq`
  - 如果 `seq - last_consecutive > size`（窗口溢出），强制推进 `last_consecutive = seq - size`，然后调用 `fix_last_consecutive()` 向前扫描连续区间
- **OOS 包（`diff >= UINT16SIZE_HALF`，即 seq 在 end 之后的回绕区间，实际上 seq < end）**：
  - 如果 `seq == last_consecutive + 1`，推进 `last_consecutive = seq`，然后调用 `fix_last_consecutive()` 向前扫描
  - 否则只标记 `set_received(seq)`，不推进任何指针
  - **注意**：大多数 OOS 包走的是"否则"分支——位图里标记为已收到，但 `last_consecutive` 不推进，`missing_seq_numbers()` 的返回值不会立即变化；只有恰好填上 `last_consecutive+1` 空位的 OOS 包才会触发连续区间的推进
- **`fix_last_consecutive()`**：从 `last_consecutive + 1` 开始向前扫描，直到遇到第一个未收到的序列号，更新 `last_consecutive` 为扫描到的最后一个连续序列号

**缺包查询**

- `missing_seq_numbers(skip_last_n)`：返回 `[last_consecutive+1, end-skip_last_n]` 区间内未收到的序列号
  - 当前调用侧 `skip_last_n = 2`（硬编码在 `mod.rs:135`）
  - 如果 `end - skip_last_n < last_consecutive`（回绕或窗口为空），返回空 Vec
  - 否则遍历区间，逐个检查 `get_received()`，O(gap) 复杂度
- `missing_seq_numbers_in_range(start, end_exclusive)`：返回 `[start, end_exclusive)` 区间内未收到的序列号
  - 供 sample loss 路径使用（`collect_recent_missing_sequences` 调用时 `skip_last_n = 0`）
  - 如果 `end_exclusive - start >= UINT16SIZE_HALF`（回绕或无效区间），返回空 Vec

**与上游的集成（source.rs）**

```rust
let add_outcome = self.nack_window.add(seq);
self.on_nack_window_add_outcome(add_outcome, rtp.header.timestamp, now_ms);
self.push_recent_rtp_packet(seq, rtp.header.timestamp);
if let Some(resolved) = self.nack_scheduler.resolve_sequence(seq, now_ms) { ... }
```

- 每个包到达时立即调用 `add(seq)`，无论是 Primary、RepairPrimaryPassThrough 还是 RtxReinject
- `add()` 返回 `NackWindowAddOutcome`，包含 `is_oos`、`oos_distance_from_end`、`advanced_last_consecutive`、`overflow_advanced`、`opened_gap`、`closed_gap`、`overflow_pruned_range` 等字段
- `on_nack_window_add_outcome` 消费这个返回值：OOS 事件更新 `oos_event_count`、`recent_oos_active_until_ms`、`frame_oos_flags`，并驱动 `update_dynamic_nack_skip_last_n`；溢出事件触发 `prune_pending_nack_for_window_range` 清理对应区间的 pending NACK
- `add()` 之后调用 `nack_scheduler.resolve_sequence(seq)`，如果该序列号在 pending NACK 列表中，标记为已恢复

**gap 检测的两条并行路径**

`nack_window` 并不是唯一的 gap 检测机制，`source.rs` 里同时维护了 `last_highest_rtp_sequence`，由 `detect_forward_gap` 独立检测顺序 gap：

- `detect_forward_gap`：基于 `last_highest_rtp_sequence`，只检测"新包比已知最高序列号高出超过 1"的情况，触发 `observe_forward_gap_and_nack`，立即发出初始 NACK
- `nack_window` + `maybe_run_nack_maintenance`：基于环形位图，每次包到达时都查询 `missing_seq_numbers(skip_last_n=2)`，负责 NACK 的重试调度和 deadline 管理

两条路径对同一个 gap 都会产生动作：`detect_forward_gap` 在 gap 首次出现时发初始 NACK，`maybe_run_nack_maintenance` 在后续每个包到达时重新扫描并驱动重试。`nack_scheduler` 的去重逻辑（`observe_missing_sequences_with_policy` 对已在 pending 中的序列号不重复插入）是防止重复发 NACK 的唯一屏障。OOS 包到达时，`detect_forward_gap` 不会产生 gap（因为 seq < last_highest），但 `maybe_run_nack_maintenance` 仍会扫描到 `last_consecutive` 之后的所有 gap，包括这个 OOS 包之前的 gap。

#### 缺口

**1. `skip_last_n` 是静态参数，不感知网络乱序程度**

- **当前取值**：硬编码为 `2`（`mod.rs:135`）
- **问题场景**：
  - 乱序严重时（如 WiFi 多路径、移动网络），`skip_last_n = 2` 不够大，收到 `seq=100` 后立即把 `seq=98` 标记为缺失并触发 NACK，但 `seq=98` 可能只是延迟 1-2 个包到达，导致无意义的 NACK 请求
  - 乱序轻微时（如有线局域网），`skip_last_n = 2` 过大，真正丢失的包需要等 2 个后续包到达才能被检测，增加 NACK 延迟
- **影响**：误触发 NACK 浪费带宽，延迟检测增加恢复延迟
- **严重程度**：性能问题，不影响正确性（NACK 重试机制会最终收敛）

**2. OOS 包到达后可能触发重复 NACK（根因：`add()` 不返回 OOS 事件）**

- **根因**：`add()` 返回 `()`，调用侧无法知道这个包是否是 OOS 包、是否推进了 `last_consecutive`、是否让某个 gap 区间变成了连续区间
- **问题场景 A（OOS 包在 NACK 发出之前到达）**：
  1. 收到 `seq=100`，`detect_forward_gap` 检测到 `seq=98` 缺失，发出初始 NACK
  2. `seq=98` 因乱序延迟到达（OOS 包），`add(98)` 标记位图，`resolve_sequence(98)` 返回 `None`（因为 NACK 还在 pending 中但尚未被 `observe_missing_sequences_with_policy` 处理）
  3. 下一个包到达时，`maybe_run_nack_maintenance` 调用 `missing_seq_numbers(2)`，如果 `seq=98` 恰好填上了 `last_consecutive+1`，它不会出现在 missing 列表里；但如果 `last_consecutive` 没有推进（`seq=98 != last_consecutive+1`），`seq=98` 在位图里已标记为 received，`get_received(98)` 返回 true，同样不会出现在 missing 列表里——**这种情况下不会重复发 NACK**
  4. 真正的问题是：如果 `seq=98` 已经进入 `nack_scheduler` 的 pending 列表，但 `resolve_sequence` 没有被调用（例如 OOS 包被背压丢弃，从未进入 `recv_frame_inner`），这个 NACK 会持续重试直到 deadline 耗尽
- **问题场景 B（`add()` 不传递 OOS 事件给上层）**：
  - 无法在 OOS 包到达时关闭 speculative RFI 模式（Moonlight 的 `receivedOosData` 机制）
  - 无法在 OOS 包到达时更新"当前帧是否进入过乱序路径"的状态
  - 无法在窗口溢出时触发告警或调整策略
- **影响**：背压丢包场景下发出无意义的 NACK 请求；上层决策逻辑缺少 OOS 网络状态信号，无法实现自适应策略
- **严重程度**：场景 A 是性能问题（背压丢包时）；场景 B 是架构缺陷，阻碍后续优化（speculative 预测、动态 NACK 策略）

**3. 没有帧级别的 OOS 状态追踪**

- **当前行为**：`nack_window` 是纯序列号级别的，不知道哪些序列号属于同一帧
- **问题场景**：
  - 无法判断"这个 OOS 包是否让某帧从不可恢复变成了可恢复"
  - 无法判断"当前帧是否已经进入过 OOS 路径"（用于 speculative 预测的前置条件）
  - 无法在帧边界清理旧帧的 OOS 状态
- **影响**：无法实现 Moonlight 的 `useFastQueuePath` 帧级路径锁定，无法基于帧级 OOS 状态做 speculative 预测
- **严重程度**：架构缺陷，阻碍 speculative 不可恢复预测的实现

**4. 窗口溢出时强制推进 `last_consecutive` 可能丢失 gap 信息（正确性问题）**

- **触发条件**：`seq - last_consecutive > size`（注意是与 `last_consecutive` 的差，不是与 `end` 的差）
- **风险场景**：如果 `last_consecutive` 因持续 gap 长期停滞，而 `end` 持续推进，这个差值会快速增长。例如：`last_consecutive = 1000`，`end = 9000`，`size = 8192`，下一个顺序包 `seq = 9001` 到达时 `9001 - 1000 = 8001 < 8192`，还没溢出；但如果 gap 持续扩大到 `seq - last_consecutive > 8192`，就会触发强制推进
- **后果**：`[last_consecutive+1, seq-size]` 区间内的 gap 信息被丢失，对应的 NACK 可能还在 pending 列表中，但 `nack_window` 已无法查询到这些序列号，`missing_seq_numbers()` 不再返回它们，NACK 重试会在 deadline 耗尽后才停止
- **严重程度**：正确性问题。当前 `size = 8192` 看起来很大，但在持续高丢包（`last_consecutive` 长期停滞）场景下并非不可触发，需要关注

**5. 窗口大小固定，不感知当前帧的包数规模**

- **当前取值**：`size = 8192`（`log2_size_minus_6 = 7`）
- **问题场景**：
  - 1080p60 场景下，单帧可能包含 50-150 个包，窗口足够
  - 4K60 场景下，单帧可能包含 300-500 个包，如果 jitter buffer 保留 3-5 帧，需要 1500-2500 个序列号的窗口，当前窗口足够
  - 真正的风险不在于单帧包数，而在于 `last_consecutive` 停滞时窗口被 gap 撑满（见缺口 4）
- **影响**：内存固定（1KB），不随分辨率变化，当前参数下不是主要问题
- **严重程度**：低优先级，当前参数下不太可能成为瓶颈

#### 值得借鉴（Moonlight）

**OOS 状态与 speculative 模式的联动（RtpVideoQueue.c:queuePacket）**

Moonlight 在 `queuePacket` 里，每次收到 OOS 包时设置 `receivedOosData = true` 并记录时间戳，同时关闭 speculative RFI 模式。只有在 `SPECULATIVE_RFI_COOLDOWN_PERIOD_US`（5 分钟）内没有再收到 OOS 包，才重新开启 speculative 模式。

这个机制的核心价值：**OOS 包的到达是网络乱序的直接证据，而乱序网络下 speculative 预测"帧不可恢复"极易误判**。我们没有等效的 OOS 状态追踪，`nack_window` 的 OOS 信息没有被传递给任何上层决策逻辑。

**`missingPackets` 实时维护（RtpVideoQueue.c:RtpvAddPacket）**

Moonlight 在每个包到达时实时维护 `missingPackets`：收到比当前最高序列号更高的包时 `missingPackets += gap`，收到之前缺失的包时 `missingPackets--`。这让它可以在任意时刻精确知道"当前 FEC block 还缺多少包"，从而做 speculative 预测（`missingPackets > totalPackets - neededPackets`）。

我们的 `nack_window` 也维护了类似信息，但只在 `maybe_run_nack_maintenance` 被调用时才查询，不是实时的，也没有被用于 speculative 预测。

**单路径 vs 双路径 gap 检测**

Moonlight 是单线程同步流水线，gap 检测只有一条路径（`RtpvAddPacket` 里的 `missingPackets` 计数）。我们有两条并行路径（`detect_forward_gap` + `nack_window`），去重依赖 `nack_scheduler` 的 pending 列表。这在正常情况下工作，但在 OOS 包到达、背压丢包、`last_consecutive` 停滞等边界场景下，两条路径的状态可能出现不一致，需要 `nack_scheduler` 的去重逻辑完全兜底。

### 2.3 NACK 触发与重传调度（nack.rs / nack_scheduler.rs / nack_policy.rs）

这一层负责把 gap 信息转化为 RTCP NACK 报文，并管理每个缺失序列号的重试生命周期。入口是 `nack_window` 产出的缺失序列号列表，出口是通过 `rtcp_port.send_rtcp` 发出的 `TransportLayerNack` 报文，以及 gap 过期后触发的 chain broken / keyframe 请求。

#### 现有能力

**三条触发路径**

| 路径 | 触发时机 | 来源标记 |
|------|----------|----------|
| `observe_forward_gap_and_nack` | 顺序 gap 首次出现（`detect_forward_gap` 检测到） | `rtpGap` |
| `maybe_run_nack_maintenance` | 每个包到达时，基于 `nack_window.missing_seq_numbers(nack_skip_last_n)` | `rtpWindow` |
| `observe_sample_loss_and_nack` | `sample_builder` 产出带 `media_dropped_packets > 0` 的帧后补发 | `sampleLoss` |

三条路径共用同一个 `NackScheduler`，去重由 `observe_missing_sequences_with_policy` 保证：已在 pending 中的序列号不重复插入。

**NackScheduler 的 pending 状态机**

每个缺失序列号以 `PendingNack` 条目存入 `BTreeMap`，记录首见时间、上次发送时间、deadline、重试间隔、优先级、帧归属元数据等。生命周期有三种终止路径：

- `deadline` 到期：`poll()` 里 `now_ms >= pending.deadline_at_ms`
- `maxAge` 到期：`poll()` 里 `age_ms >= pending.max_age_ms`
- `retryBudget` 耗尽：`poll()` 里 `retry_count >= max_retry_count`

重试时按 `priority`（高优先）+ `first_seen_at_ms`（早优先）排序，每次最多发 `burst_count` 个。

**NackObservePolicy 的分层参数**

三条路径各自构造 `NackObservePolicy`，参数按 cloud/startup/local 模式和帧重要性（keyframe / reference / delta）分档：

- `rtpGap`：deadline 最短，burst_count 最大（比 rtpWindow 多 1），用于首次 gap 的快速响应
- `rtpWindow`：中等参数，负责持续重试
- `sampleLoss`：deadline 和 max_age 由 `dynamic_repair_deadline` 根据 `repairability` 动态计算，burst_count 也随 repairability 浮动

cloud 模式下，所有路径的 deadline 和 max_age 都通过 `cloud_startup_head_hole_deadline_at_ms` / `cloud_nack_max_age_ms` 按运行时 RTT 放宽，RTT 超过 100ms / 200ms / 300ms 时分三档额外加宽。

**`with_cloud_latency_admission_policy` 准入过滤**

三条路径在调用 `observe_missing_sequences_with_policy` 之前都经过这个函数，它可能把 `nack_disposition` 改为以下跳过状态：

- `SkippedChainBroken`：`waiting_for_recovery_keyframe` 时的非 anchor gap，或 sampleLoss 低 repairability 的 reference 帧
- `SkippedLowValue`：cloud 高 RTT 下的低价值 gap，或 estimated_recovery_arrival 接近 deadline 的低价值 gap
- `SkippedTooLate`：`now_ms >= deadline_at_ms`，或 estimated_recovery_arrival > deadline

`SkippedLowValue` 有 250ms 抑制窗口，同一序列号在窗口内不重复上报；keyframe / reference 帧可绕过 `SkippedLowValue` 直接走 `Attempted`。

**`PacketRecoveryDisposition` 语义**

四种结果贯穿整个 NACK 生命周期，最终写入 `XbxEngineVideoNackObservation` 供上层观测：

- `Attempted`：正常发出 NACK，等待恢复
- `SkippedTooLate`：deadline 已过或预计到达时间超过 deadline
- `SkippedLowValue`：价值不足，主动放弃修复
- `SkippedChainBroken`：参考链已断，修复无意义

**gap 过期后的 chain broken 处理**

`maybe_run_nack_maintenance` 和 `observe_forward_gap_and_nack` 在收到 `SkippedNackBatch` 或 `ExpiredNackBatch` 后，调用 `timeline_state.mark_gap_expired`，返回值 `chain_broken: bool` 表示这次过期是否打断了参考链。若 `chain_broken` 为 true，进入 `maybe_trigger_reference_chain_recovery`：

1. 调用 `flush_non_keyframe_pending` 清空 pending 中所有非 keyframe 条目
2. 判断 `should_soft_request_recovery_keyframe`（需要有干净锚点且输出正常）
3. soft request：只发信号，不阻塞帧提交；hard request：设 `waiting_for_recovery_keyframe = true`，阻塞后续非 keyframe 帧提交

**`waiting_for_recovery_keyframe` 的退出路径**

状态由 `resolve_recovery_keyframe_action` 在每帧产出时计算：

- 收到完整无丢包的 keyframe → `(false, Submit)`，状态清除
- 收到带丢包的 keyframe → `(false, DropAndRequestKeyframe)`，状态清除但帧丢弃并再次请求
- `sustaining_recovery_active` 且健康 delta continuation → `(false, Submit)`，允许在恢复保活阶段提交
- `!hard_recovery_gap_risk` → `(false, Submit)`，timeline 认为风险可控时提前退出等待
- 其余情况 → `(true, WaitKeyframe)`，继续等待

没有独立的超时重试机制：如果 keyframe 请求发出后服务端长时间无响应，`waiting_for_recovery_keyframe` 状态会持续阻塞，直到下一个 keyframe 到达或 `sustaining_recovery_active` / `!hard_recovery_gap_risk` 条件满足。

**`repairability` 估算**

`sampleLoss` 路径在构造 policy 前调用 `estimate_repairability`，综合以下因素：

- 帧重要性基础值（keyframe 0.95 / reference 0.8 / delta 0.62）
- `sample_loss_burst_count`（连续 sampleLoss 次数，每次 -0.04）
- `nack_late_ewma`（历史 late 恢复率，权重 0.35）
- `missing_ratio`（缺失序列号数 / media_dropped_packets，超出 1.0 的部分 -0.08）
- `nack_recovery_ewma_ms`（历史恢复时延，≤16ms +0.08，≤24ms +0.04）
- 当前修复阶段（startup +0.06，recovery -0.04）
- `waiting_for_recovery_keyframe` 时额外 -0.06

结果 clamp 到 [0.25, 1.0]，用于调整 deadline 窗口（`dynamic_repair_deadline`）和 burst_count。

#### 缺口

**1. `maybe_run_nack_maintenance` 只在包到达时触发，没有独立定时器**

高丢包后网络短暂静默期间，pending NACK 的重试完全停止，直到下一个包到来才恢复。`poll()` 里的 deadline / maxAge / retryBudget 判断都依赖被调用，但调用者只有 `recv_frame_inner` 的包处理路径。在包稀疏场景下，一个本可在 10ms 内重试的 NACK 可能因为 50ms 内没有新包到达而延迟。

**2. `rtpGap` 和 `rtpWindow` 对同一 gap 的 deadline 不一致，且首次插入后不可更新**

`observe_forward_gap_and_nack`（`rtpGap`）和 `maybe_run_nack_maintenance`（`rtpWindow`）各自独立计算 `deadline_at_ms`，但 `nack_scheduler` 里同一序列号只保留首次插入的 deadline（`pending.contains_key` 跳过重复插入，不更新任何字段）。如果 `rtpGap` 先以较短的 deadline 插入，后续 `rtpWindow` 的更宽松 deadline 不会生效，该序列号可能过早过期。反之，如果 `rtpWindow` 先插入，`rtpGap` 的更激进参数（更短 retry_interval、更大 burst_count）也不会生效。

**3. `sampleLoss` 路径的序列号反查精度有限**

`collect_missing_sequences_for_sample` 通过 `recent_rtp_packets` 里的 RTP timestamp 反查序列号范围，再向两侧各扩展 `expand`（2~12）个序列号，然后在 `nack_window` 里查缺失。当帧的包分布不连续（乱序、背压丢包）或 `recent_rtp_packets` 滑动窗口（512 条）已覆盖不到该帧时，fallback 到 `collect_recent_missing_sequences`（最近 N 个缺失序列号），可能漏报或误报。这个路径没有帧级别的精确序列号范围，是结构性限制。

**4. `waiting_for_recovery_keyframe` 的 keyframe 请求有重试上限，耗尽后停止**

`set_waiting_for_recovery_keyframe(true)` 时，`next_recovery_keyframe_retry_at_ms` 被设为 `now + RECOVERY_KEYFRAME_RETRY_TIMEOUT_MS`（700ms）。`maybe_run_nack_maintenance` 每次调用时触发 `maybe_retry_waiting_recovery_keyframe`：超时后重新发送 keyframe 请求，之后每 `RECOVERY_KEYFRAME_RETRY_INTERVAL_MS`（450ms）重试一次，最多 `RECOVERY_KEYFRAME_RETRY_MAX_COUNT`（8 次）。

缺口在于：**8 次重试耗尽后，`next_recovery_keyframe_retry_at_ms` 被清空，不再发送任何请求**，`waiting_for_recovery_keyframe` 状态仍然为 true，帧提交继续被阻塞，但没有任何机制再次触发恢复。总等待时间上限约为 700ms + 8 × 450ms = 4.3 秒，之后进入永久阻塞。

**5. OOS 状态未被 NACK 决策路径消费**

`on_nack_window_add_outcome` 在 OOS 包到达时更新了 `recent_oos_active_until_ms`、`frame_oos_flags`，并通过 `update_dynamic_nack_skip_last_n` 动态调整 `nack_skip_last_n`。但 `oos_recently_active` 和 `frame_seen_oos` 这两个查询函数在 `nack.rs` 和 `nack_policy.rs` 里没有任何调用点——OOS 状态只用于日志，没有影响 `repairability` 估算、`with_cloud_latency_admission_policy` 的准入判断，也没有影响 deadline 计算。乱序网络下 repairability 可能被高估，导致 deadline 被 `dynamic_repair_deadline` 不必要地拉长。

#### 值得借鉴（Moonlight）

Moonlight 的 NACK 相关策略建立在两个我们不具备的前提上：**每帧包数（`bufferDataPackets`）和冗余包数（`bufferParityPackets`）在帧头里已知**，以及 **FEC 恢复是同步完成的**。这使得它可以在包到达时实时维护 `missingPackets` 计数，并在任意时刻精确判断"这帧还能不能恢复"。我们的 RTX 是事后补发，没有前置冗余，也没有帧级包数声明，以下机制因此不适用：

- **speculative RFI**（`missingPackets > totalPackets - neededPackets` 时提前通知服务端帧丢失）：依赖精确的 FEC 容量上限，我们没有等效信息
- **`receivedOosData` 冷却**：附属于 speculative 预测，用于防止乱序包到达后误判帧不可恢复；我们没有 speculative 预测，这个冷却没有对应场景
- **`useFastQueuePath` 帧级路径锁定**：解决 FEC 序列号对齐问题，我们的 `sample_builder` 基于 RTP timestamp，不存在这个问题

**真正值得参考的是两个设计思路：**

**`waitingForNextSuccessfulFrame` 节流（VideoDepacketizer.c）**

Moonlight 在帧丢失后设置 `waitingForNextSuccessfulFrame = true`，等到下一个完整帧到达 depacketizer 的 lastPacket 处理时才清除，避免网络不稳定时频繁发请求加剧拥塞。我们的 `should_soft_request_recovery_keyframe` 在有干净锚点且输出正常时走 soft request（不阻塞），否则走 hard request，在语义上覆盖了这个场景。

**`consecutiveFrameDrops` 兜底（VideoDepacketizer.c）**

连续丢帧达到上限时强制发 IDR 请求并重置计数，作为所有恢复路径都失败后的最终保障。我们有类似机制：`maybe_retry_waiting_recovery_keyframe` 在 700ms 后开始重试，每 450ms 一次，最多 8 次。但 8 次耗尽后进入永久阻塞（见缺口 4），Moonlight 的 `consecutiveFrameDrops` 则会持续计数并在每次达到上限时重置重试，没有总次数上限。

### 2.4 Jitter Buffer 与帧重组（sample_builder）

这一层负责把乱序到达的 RTP 包重新排列成完整的媒体帧。入口是 `sample_builder.push(rtp)`，出口是 `sample_builder.pop()` 返回的 `Sample`，携带 `prev_dropped_packets`（丢包计数）和 `packet_timestamp`（帧 RTP timestamp）。

#### 现有能力

**SampleBuilder 的核心数据结构**

使用第三方库 `rtc-media 0.9.0` 的 `SampleBuilder<H264Packet>`，内部维护三个游标：

- `filled`：已收到的包的序列号范围 `[head, tail)`
- `active`：当前正在尝试组装的帧的序列号范围
- `prepared`：已组装完成、等待 `pop()` 取走的帧队列

底层是一个大小为 `u16::MAX + 1`（65536 槽）的环形数组，按序列号直接寻址，O(1) 插入。

**两个积压上限参数**

| 参数 | 默认值 | 语义 |
|------|--------|------|
| `max_late_packets` | `jitter_buffer_max_packets`（默认 1024） | `filled` 窗口序列号跨度超过此值时触发强制产帧 |
| `max_late_timestamp` | 由 `jitter_buffer_max_delay`（默认 30ms）× 90kHz 换算 | `filled` 窗口内最旧包与最新包的 RTP timestamp 差超过此值时触发强制产帧 |

两个条件任一满足即触发 `purge_buffers`，强制把当前 `active` 帧产出（即使不完整）。这两个参数是**积压深度的上限**，不是计时器——正常流式到达时不会触发，只有缓冲区里同时积压了跨越多帧的包时才生效。`jitter_buffer_min_delay`（默认 20ms）作为下限保证 `max_late_timestamp` 不低于最小值。

**产帧逻辑（`build_sample`）**

每次 `push` 后调用 `purge_buffers`，每次 `pop` 前调用 `build_sample(false)`：

1. 从 `active.head` 开始向前扫描，找到 RTP marker bit 置位的包（`H264Packet::is_partition_tail` 直接返回 marker bit）或 timestamp 切换点，确定当前帧的边界 `consume = [head, tail)`
2. 若 `consume.tail` 位置的包还未到达（`buffer[consume.tail].is_none()`），返回 `PendingTimestampPacket`，等待下一帧第一个包到达后再产帧——**这是正常路径下的固有延迟来源**，额外等待时间约等于一个帧间隔（60fps ≈ 16.7ms，30fps ≈ 33ms）
3. 若 `active.head` 的包不是 partition head（`!is_partition_head`），说明帧头丢失，丢弃这段包并累加 `dropped_packets`
4. 若扫描中遇到 gap（`GapInSegment`），停止并等待
5. 组装成功：把 `[consume.head, consume.tail)` 的所有包通过 `H264Packet::depacketize` 拼接成 `Sample.data`，`dropped_packets` 写入 `prev_dropped_packets` 并清零

**`prev_dropped_packets` 的语义**

`Sample.prev_dropped_packets` 记录的是**上一帧产出到本帧产出之间**被 `purge_buffers` 强制丢弃的包数（含 padding 包）。`source.rs` 里用 `media_dropped_packets = prev_dropped_packets.saturating_sub(prev_padding_packets)` 得到真实媒体丢包数，作为 `observe_sample_loss_and_nack` 的输入。

**`sample_builder` 的重置时机**

idle timeout 触发时（`source.rs:1389`），`sample_builder` 被整体替换为新实例：

```rust
self.sample_builder = build_sample_builder(self.max_late_packets, self.jitter_buffer_max_delay);
```

这会清空所有缓冲中的包，丢弃所有正在组装的帧。

**驱动方式**

`recv_frame_inner` 是一个 `loop`，每次迭代：

1. 检查 `should_run_nack_maintenance_tick()`（间隔 10ms），满足则运行 NACK 维护
2. 尝试 `sample_builder.pop()`，有帧则进入帧处理路径
3. 无帧则 `tokio::time::timeout(read_timeout, rx.recv())`，`read_timeout` 由 `nack_maintenance_timeout` 限制为最多 10ms（保证 NACK tick 不被阻塞）

#### 缺口

**1. 正常路径固有延迟：必须等下一帧第一个包**

`build_sample` 在找到 marker bit 包（帧尾）后，不会立即产帧，而是检查 `buffer[consume.tail].is_none()`——如果下一帧的第一个包还没到，返回 `PendingTimestampPacket` 继续等待。这个设计是为了确认帧边界（用下一帧的 timestamp 计算本帧 duration），但代价是每帧都要额外等待约一个帧间隔（60fps ≈ 16.7ms）。

这是 `SampleBuilder` 的设计约束，不是配置问题。在低延迟场景下，这个额外的帧间隔等待是可观测的固定开销。

**2. 丢包场景下 `purge_buffers` 强制产帧时机不可控**

`purge_buffers` 在 `too_old` 触发时（`filled` 窗口内最旧和最新包的 timestamp 差 > 30ms × 90kHz），对当前帧调用 `build_sample(true)`，跳过 `PendingTimestampPacket` 检查强制产出。

问题在于：`too_old` 比较的是 `filled` 窗口的 timestamp 跨度，不是挂钟等待时间。如果当前帧有丢包（gap），后续帧的包持续到达，`filled` 窗口的 timestamp 跨度会随之增长，最终触发强制产帧。触发时机取决于后续帧的到达速度，而不是"等了多久"，在高帧率场景下可能比预期更早触发。

**3. `prev_dropped_packets` 跨帧累积，无法精确归因**

`dropped_packets` 在 `build_sample` 成功时清零，但在两次成功产帧之间可能有多次 `purge_buffers` 调用，每次都累加。最终 `prev_dropped_packets` 反映的是上一帧到本帧之间所有被丢弃的包，不区分属于哪一帧。`collect_missing_sequences_for_sample` 通过 RTP timestamp 反查序列号时，这个跨帧累积会导致 NACK 目标序列号不准确（2.3 节缺口 3 的根因之一）。

**4. 帧头丢失时整段丢弃，没有通知 NACK 层**

`build_sample` 检测到 `!is_partition_head` 时，直接丢弃 `consume` 区间的所有包并累加 `dropped_packets`，但不会触发任何 NACK 请求。这些包的序列号已经被 `nack_window` 标记为已收到（因为它们确实到达了），所以 `maybe_run_nack_maintenance` 也不会为它们发 NACK。实际上丢失的是帧头包，但 NACK 层对此无感知。

#### 值得借鉴（Moonlight）

Moonlight 没有独立的 jitter buffer 层。`RtpvAddPacket` 完成 FEC 恢复后，`stageCompleteFecBlock` 把包按序列号排列到 `completedFecBlockList`，再由 `submitCompletedFrame` 逐包提交给 `VideoDepacketizer`。整个过程是同步的，没有等待窗口——FEC 凑够了就立刻产帧，凑不够就等下一个包或判定不可恢复。我们的 `SampleBuilder` 等待下一帧第一个包的固有延迟，在 Moonlight 的同步模型里不存在。这是架构差异，没有直接可借鉴的实现。

**帧头丢失的处理方式**值得参考：Moonlight 在 `VideoDepacketizer.c` 里检测到帧头缺失时，会调用 `dropFrameState` 并设置 `waitingForIdrFrame`，明确触发恢复流程。我们的 `sample_builder` 在帧头丢失时只是静默丢弃并累加计数，没有向上层传递"帧头丢失"这个语义，上层只能通过 `prev_dropped_packets > 0` 间接感知，无法区分"帧头丢失"和"帧中间丢包"这两种需要不同处理策略的情况。

### 2.5 帧准入与恢复决策（source.rs）

这一层是链路的出口门控，负责把 `sample_builder` 产出的 `Sample` 转化为 `AssembledVideoFrame` 或触发恢复流程。入口是 `sample_builder.pop()` 返回的帧，出口是 `AssembledVideoFrame`（提交给下游解码）或 `continue`（丢弃并触发 NACK / keyframe 请求）。

#### 现有能力

**帧处理的完整决策链**

每帧经过以下顺序的判断，任一环节拒绝则 `continue` 丢弃：

```
sample.pop()
  → H264 inspection（NAL 解析、SPS/PPS 检查、slice header 验证）
  → InspectionAdmission 判断
  → media_dropped_packets 计算（prev_dropped_packets - prev_padding_packets）
  → sample_loss_burst_count 更新
  → frame_importance 分类（keyframe / reference / delta）
  → resolve_recovery_keyframe_action → RecoveryKeyframeAction
  → 按 action 分支：Submit / DropAndRequestKeyframe / WaitKeyframe
  → 通过：observe_frame → mark_frame_complete_candidate → return AssembledVideoFrame
```

**H264 Inspection 与 InspectionAdmission**

`h264_inspector.inspect_access_unit` 解析每帧的 NAL 结构，产出：
- `is_idr`：是否 IDR 帧
- `bootstrap_ready`：SPS + PPS + IDR VCL 齐全，可独立解码
- `delta_continuation_ready`：committed SPS/PPS 已有，slice header 有效，可作为 delta 帧继续
- `slice_headers_valid`：slice header 解析无误
- `config_changed`：SPS/PPS 发生变化（触发 `reference` 重要性）

`resolve_inspection_admission` 的准入逻辑：
- `!slice_headers_valid` → `AwaitRecoveryKeyframe`（无条件拒绝）
- `bootstrap_ready` → `Accept`
- `first_frame_acquired || decoder_bootstrap_no_output_continuation_allowed || sustaining_recovery_continuation_allowed` 且 `delta_continuation_ready` → `Accept`
- 其余 → `AwaitRecoveryKeyframe`

**`resolve_recovery_keyframe_action` 的帧级决策**

在 inspection 通过后，根据 `waiting_for_recovery_keyframe`、`media_dropped_packets`、`is_keyframe`、`hard_recovery_gap_risk` 决定帧的命运：

| 条件 | 结果 |
|------|------|
| `is_keyframe && media_dropped_packets > 0` | `DropAndRequestKeyframe`（带丢包的 keyframe 不能喂给解码器） |
| `is_keyframe && media_dropped_packets == 0` | `Submit`，同时清除 `waiting_for_recovery_keyframe` |
| `media_dropped_packets > 0`（非 keyframe） | `DropAndRequestKeyframe` |
| `waiting_for_recovery_keyframe && !first_frame_acquired` | `WaitKeyframe` |
| `waiting_for_recovery_keyframe && sustaining_recovery_active` | `Submit`（恢复保活阶段允许健康 delta 通过） |
| `waiting_for_recovery_keyframe && !hard_recovery_gap_risk` | `Submit`（timeline 认为风险可控） |
| `waiting_for_recovery_keyframe && hard_recovery_gap_risk` | `WaitKeyframe` |
| 其余 | `Submit` |

**VideoTimelineState 的 ChainState 状态机**

`timeline_state` 维护一个 `ChainState`，贯穿整个帧处理流程：

```
Healthy ──gap出现──→ Repairing ──gap过期/chain broken──→ Broken
                                                          ↓
                                              on_recovery_keyframe_requested
                                                          ↓
                                                      Recovering
                                                          ↓
                                              on_clean_keyframe_ingress（干净IDR）
                                                          ↓
                                                  SustainingRecovery
                                                          ↓
                                              passes_stable_recovery_gate
                                              （≥2帧 + ≥120ms 无新 gap）
                                                          ↓
                                                       Healthy
```

`Broken` 和 `Recovering` 对应 `waiting_for_recovery_keyframe() == true`，阻塞非 keyframe 帧提交。`SustainingRecovery` 是恢复保活阶段，允许健康 delta 帧通过，直到稳定窗口满足后回到 `Healthy`。

**`has_hard_recovery_gap_risk` 的判断**

`resolve_recovery_keyframe_action` 里的 `hard_recovery_gap_risk` 来自 `timeline_state.has_hard_recovery_gap_risk()`，满足以下任一条件即为 true：
- `chain_state == Broken`
- gaps 里有 `severity == Hard` 且未 Resolved/Expired 的条目
- `frame_recovery_ledger` 里有 `UnrecoverableReferenceChain` 条目
- `chain_debt_reason` 是 hard recovery reason（`awaitingRecoveryKeyframe`、`referenceChainUnrecoverable`、`bootstrapMissingSps` 等）

gap 的 `severity` 由 `classify_gap` 决定：reference/keyframe 帧的 gap 为 Hard，匿名 delta gap 且 close_reason 为 `awaitingRecoveryKeyframe` 也为 Hard，其余为 Soft。

**`sample_loss_burst_count` 与 `repairability` 的联动**

`media_dropped_packets > 0` 时 `sample_loss_burst_count` 递增，keyframe 到达时清零，连续 6 帧无丢包（`SAMPLE_LOSS_BURST_CLEAR_CLEAN_SAMPLE_COUNT`）后也清零。这个计数直接影响 2.3 节的 `estimate_repairability`（每次 -0.04），是 NACK 策略感知连续丢包压力的唯一来源。

**`AssembledVideoFrame` 的输出元数据**

通过准入的帧携带：
- `is_keyframe`、`config_changed`、`value`（`FrameValue`）
- `budget`（`FrameBudgetContext`，来自 `take_frame_recovery_ledger`）
- `frame_playout_deadline_at_ms`、`frame_recovery_disposition`、`frame_unrecoverable_reason`（来自 NACK 层写入的 ledger）
- `h264`（`H264AccessUnitInspection`，供下游解码器使用）
- `assembled_at`（`Instant`，用于延迟测量）

#### 缺口

**1. `DropAndRequestKeyframe` 路径下 `observe_sample_loss_and_nack` 返回 false 时无 keyframe 请求**

当 `collect_missing_sequences_for_sample` 和 `collect_recent_missing_sequences` 都返回空时，`observe_sample_loss_and_nack` 返回 false，此时只发出 `PacketLossDetected` 信号，不发 NACK，也不触发 keyframe 请求。这意味着：帧有丢包（`media_dropped_packets > 0`），但 NACK 层找不到对应的缺失序列号（可能因为 `nack_window` 已经把这些序列号标记为已收到，或者 `recent_rtp_packets` 窗口已滑过），丢包无法被修复，也没有触发 keyframe 请求。`PacketLossDetected` 信号只是一个上报，不会在 source 层直接触发恢复动作。

**2. `SustainingRecovery` 阶段的稳定退出条件不感知解码器状态**

`passes_stable_recovery_gate` 要求：`stable_recovery_started_at_ms` 起算 ≥120ms 且 `stable_recovery_clean_frame_streak ≥ 2`。`stable_recovery_clean_frame_streak` 只计数 `mark_frame_complete_candidate` 被调用的帧（即通过 source 层准入的帧），不区分这些帧是否真正被解码器接受。如果解码器在 `SustainingRecovery` 阶段连续失败，timeline 仍然会在条件满足后回到 `Healthy`，导致后续丢包不再触发恢复流程。

**3. `resolve_recovery_keyframe_action` 的返回值第一项含义需要澄清**

函数签名返回 `(bool, RecoveryKeyframeAction)`，第一项是 `next_waiting_for_recovery_keyframe`。对于 `DropAndRequestKeyframe` 和 `Submit` 分支，返回值第一项均为 `false`——这意味着这两个分支都会清除 `waiting_for_recovery_keyframe` 状态。特别是 `DropAndRequestKeyframe` 分支（`is_keyframe && media_dropped_packets > 0` 或 `media_dropped_packets > 0`），清除状态后帧被丢弃，但 `waiting_for_recovery_keyframe` 已经是 false，后续 delta 帧不会被阻塞，只依赖 `observe_sample_loss_and_nack` 触发的 NACK 和 chain broken 路径来重新进入等待状态。这个行为是有意为之（decoder safety 职责与恢复升级职责分离），但在 NACK 层未能找到缺失序列号时（缺口 1），可能导致恢复流程完全缺失。

#### 值得借鉴（Moonlight）

Moonlight 的帧准入逻辑在 `VideoDepacketizer.c` 里，与我们的结构最接近。核心差异：

**Moonlight 的帧准入是流式的，我们的是批量的**

Moonlight 逐包处理，在 `processRtpPayload` 里实时判断每个 NAL 的类型和帧边界，一旦检测到帧头缺失或 streamPacketIndex 不连续，立即调用 `dropFrameState`。我们在 `sample_builder.pop()` 之后才能看到完整帧，无法在包级别做早期拒绝。

**`waitingForNextSuccessfulFrame` 的语义比我们的 `SustainingRecovery` 更保守**

Moonlight 在帧序号不连续时设置 `waitingForNextSuccessfulFrame = true`，在 depacketizer 处理到下一个完整帧的最后一个包（`lastPacket`）时清除——即帧完整到达 depacketizer 即可，不等 `submitDecodeUnit` 的解码结果。我们的 `SustainingRecovery` 在干净 IDR 进入后就允许 delta 帧通过，语义上与此相近，但 `SustainingRecovery` 的退出条件（≥2 帧 + ≥120ms）不感知解码器状态（见缺口 2）。
