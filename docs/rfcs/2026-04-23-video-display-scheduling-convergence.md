# RFC: Video Display Scheduling Convergence

Completion: 未完成
State: planned
Owner: Codex
Created: 2026-04-23

## Background

当前 `RTP -> display` 中后段存在三类结构性问题：

1. `pacer` 与 host presenter 都在做“这一帧何时播/是否还值得播”的决策，形成重复调度。
2. core `present_latest_render_frame()` 只是把帧提交给 host，但控制面容易把它误当成真实显示进展。
3. `render state -> runtime tick -> host slot/latest frame` 存在多层中间暂存，stall 时旧帧会持续污染恢复路径。

这会导致典型坏结果：

- decode 已经产出，但 display 不动
- core 统计显示“在 present”，用户看到的却仍是旧画面
- backlog、overwrite、stale drop 持续放大
- 关键帧恢复路径被旧帧占据

## Goals

1. 收敛 `P1`：移除 core render/runtime 边界上的历史帧堆积，改成 latest-only 交付语义。
2. 收敛 `P2`：弱化 core `pacer` 的最终播放时序职责，让 host display tick 成为唯一最终显示时钟。
3. 将“host submit”与“actual display present”语义彻底分离，避免控制面继续误判显示进展。
4. 在 stall / recovery 场景下统一执行“清旧保新、优先恢复”的路径。

## Non-Goals

1. 不改动 RTP 组帧、NACK、SampleBuilder 主链。
2. 不重写现有 host presenter 的平台实现细节。
3. 不在本 RFC 中处理全部 P0 诊断补充，只定义 P1/P2 所需的最小前置契约。

## Decision Summary

### 1. Host display tick 是唯一最终显示时钟

最终“哪一帧真的上屏、何时上屏”统一由 host presenter 的 display tick / render tick 决定。

### 2. Core 不再承担最终播放 sleep 调度

`pacer` 从“播放调度器”降级为“准出控制器”：

- 保留 `Drop`
- 保留“可提交/不可提交”判断
- 去掉“为了某个预计播放点主动 sleep 再提交”的职责

### 3. Core render state 改为 latest-only

`XbxRenderState` 不再维护普通多帧 pending 队列语义，改为：

- `latest_renderable_frame`
- 可选 `recovery_priority_frame`（仅在恢复关键帧保留确有必要时引入）

### 4. Runtime tick 只交付 latest

runtime tick 不再 drain 一串历史 pending render frame，而是每拍只向 host 交付“当前最值得显示的 latest frame”。

### 5. 控制面只认真实 display 进展

owner / display supply / scheduling owner 统一以 host 回传的 actual display present 指标作为显示进展依据，不再用 host submit 代替 display present。

## Impacted Modules

- `crates/xbxengine/core/src/media/video/render/renderer.rs`
- `crates/xbxengine/core/src/media/video/render/actor.rs`
- `crates/xbxengine/core/src/media/video/render/pacer.rs`
- `crates/xbxengine/core/src/media/video/pacer/actor.rs`
- `crates/xbxengine/core/src/api/runtime/sync.rs`
- `crates/xbxengine/core/src/api/runtime/lifecycle.rs`
- `crates/xbxengine/core/src/transport/rtc/stack/runtime_port.rs`
- `crates/xbxengine/core/src/transport/rtc/policy/display_supply.rs`
- `crates/xbxengine/core/src/transport/rtc/policy/video_scheduling_owner.rs`
- `src-tauri/src/mods/native_video/scheduling.rs`
- `src-tauri/src/mods/native_video/presenters.rs`
- `src-tauri/src/mods/native_video/mod.rs`

## Required Preconditions

P1/P2 改造开始前，必须先补齐最小语义拆分，否则控制面无法正确验证收敛效果：

1. 新增并区分 `host_submit_epoch` 与 `display_present_epoch`
2. 新增并区分 `submit_age_ms` 与 `display_age_ms`
3. host presenter 在真实 display tick 消费后回写 `display_present_*`

这部分属于 P0 前置，但仅实现最小字段与回写，不在本 RFC 中展开完整诊断设计。

## Implementation Plan

### Phase P1-A: Render state latest-only

目标：去掉 core render/runtime 边界上的历史帧堆积。

实施：

1. 在 `renderer.rs` 将 `pending_frames` 普通队列语义改成 `latest_renderable_frame` 单槽语义。
2. 保留 overwrite 统计，但 overwrite 的含义改成“latest 被更新覆盖”，不再表示普通多帧队列挤压。
3. `render actor` 只提交 latest，不再推动历史帧顺序排空。
4. 如果恢复路径确实需要保留单个关键恢复候选，增加单独字段，不与普通 latest 混用。

完成标准：

- core render state 任意时刻只保留一张普通可显示帧
- 不再存在 runtime drain 多张历史 render frame 的常规路径

### Phase P1-B: Runtime latest-only delivery

目标：runtime tick 只把“当前最佳帧”交给 host。

实施：

1. 在 `sync.rs` 修改 `present_latest_render_frame()`，从 drain-all 改为 fetch-latest。
2. `runtime_port.rs` 对外接口从 `drain_pending_render_frames()` 收敛为 latest 读取接口。
3. `lifecycle.rs` 保持“先 present，再 snapshot”顺序不变，但 snapshot 改为围绕 latest-only 语义取样。

完成标准：

- 每个 runtime tick 最多向 host 交付一张普通显示帧
- 旧 render frame 不再跨 tick 累积排空

### Phase P2-A: Pacer 降级为准出控制器

目标：去掉 core 对最终播放时序的二次决定。

实施：

1. 在 `pacer.rs` 将决策模型从 `Drop / Sleep / SubmitNow` 收敛为：
   - `Drop`
   - `Ready`
   - 可选 `HoldForRecovery`（仅用于短暂恢复保护，不承担最终显示排程）
2. 删除围绕预计播放点的主动 sleep 提交逻辑。
3. 保留 lateness / pressure / catch-up 相关判断，但它们只决定“是否进入 latest renderable”，不再决定最终显示时刻。
4. `pacer actor` 改为事件驱动或轻量轮询，只负责将 ready frame 推到 render latest，不再维护播放倒计时行为。

完成标准：

- core 不再因为预计 display 时间而主动 sleep 等待提交
- host 成为唯一最终显示节拍拥有者

### Phase P2-B: Host display tick 单点定时

目标：让 host presenter 成为唯一最终上屏决策点。

实施：

1. `native_video` presenter 保留现有 display tick / render tick 模型。
2. macOS native path 继续由 `ScheduledFrameSlot.take_ready_frame(...)` 在 display tick 消费。
3. WGPU path 继续由 render loop tick 消费 latest frame。
4. host presenter 回写真实 display present 事件，不再让 core submit 行为被误当成“显示进展”。

完成标准：

- 无论 native slot 还是 wgpu latest-frame path，最终显示进展都只由 host tick 定义
- core 与 host 对“已经显示”的判断语义一致

### Phase P2-C: Stall / recovery 统一策略

目标：显示停滞时，立即清旧保新，给恢复路径让路。

实施：

1. 当 `display_present_epoch` 连续多个 host tick 不前进或 `display_age_ms` 超阈值时，进入 display-stall 模式。
2. display-stall 模式下：
   - render state 只保 latest
   - runtime 只交付 latest
   - pacer 丢弃无价值旧 delta
   - ingress / decode 优先为新关键恢复路径腾入口
3. owner / display supply 统一基于 `actual display present` 驱动该模式，而不是基于 host submit。

完成标准：

- stall 时不会继续排空历史 display backlog
- 新关键恢复帧能更快穿过中后段到达 host

## API / Data Contract Changes

1. `XbxRenderState`
   - 删除普通 `pending_frames` 队列语义
   - 新增 `latest_renderable_frame`
   - 可选新增 `recovery_priority_frame`

2. `RuntimePort`
   - 将“drain pending render frames”接口改为 latest-only 读取/交换接口

3. host metrics
   - 新增 `host_submit_epoch`
   - 新增 `display_present_epoch`
   - 新增 `submit_age_ms`
   - 新增 `display_age_ms`

4. `PacerDecision`
   - 删除 `Sleep(duration)` 语义
   - 收敛为 readiness / drop 导向

## Validation Plan

### Unit / Contract Tests

1. `renderer` 测试：
   - 连续提交多帧时只保留 latest
   - overwrite 统计符合 latest-only 语义

2. `runtime sync` 测试：
   - 单次 tick 最多向 host 交付一张普通帧
   - 不再跨 tick 排空历史 render frame

3. `pacer` 测试：
   - 不再产生 sleep-based submit
   - lateness / catch-up 只影响 `Drop` 或 `Ready`

4. `display_supply / owner` 测试：
   - display progress 仅由 `display_present_epoch` 驱动
   - host submit 前进但 display 不前进时，能正确识别 stall

### Integration / Runtime Validation

1. 构造 “decode 持续有输出但 host display 不前进” 场景，确认：
   - system 不再排空历史 render frame
   - stall 后能更快切到最新恢复帧

2. 构造 “host tick 正常但新帧间歇到达” 场景，确认：
   - latest-only 不引入额外闪烁
   - no-pending 与 retain old frame 统计仍准确

3. runtime trace 对比改造前后：
   - render overwrite 比例
   - display age 尾部
   - stall 后首个新关键帧到 display 的时间

## Risks

1. latest-only 过于激进，可能让极低抖动场景下的局部平滑度下降。
2. 去掉 pacer sleep 后，若 host tick 反馈不足，可能短时间增加 latest 覆盖频率。
3. submit 与 display present 拆分后，现有控制面阈值需要重新校准。
4. native presenter 之间语义不完全一致，收敛时需要保证 macOS native 与 WGPU 都遵守同一指标契约。

## Mitigations

1. latest-only 先只用于普通显示帧；恢复关键帧保留单独保护槽，避免误伤恢复。
2. 所有控制面阈值迁移时优先基于 `display_present_*` 做兼容过渡。
3. 为 host presenter 增加统一 contract test，确保不同平台都正确回写 display present。
4. 改造按 P1 再 P2 顺序推进，避免一次性同时动 render/runtime/host 三个调度面。

## Progress Checkpoints

- [ ] 补齐 `host_submit_*` / `display_present_*` 最小前置指标
- [ ] 完成 render latest-only 改造
- [ ] 完成 runtime latest-only 交付改造
- [ ] 完成 pacer readiness-only 改造
- [ ] 完成 owner/display-supply 指标切换
- [ ] 完成 host presenter display-present 回写统一
- [ ] 完成 stall / recovery 收敛验证
