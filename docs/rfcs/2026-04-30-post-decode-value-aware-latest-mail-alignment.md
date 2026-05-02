# 解码后 Value-Aware Latest-Mail 对齐 RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 未完成
- Current State: planned
- Owner: Codex
- Last Updated: 2026-04-30

## Background

- 现有主线已经把 `decode` 之后的执行链收敛成 latest-only mailbox / latest-slot 方向，基线 RFC 为 [`2026-04-24-post-decode-latest-only-mailbox-convergence.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-04-24-post-decode-latest-only-mailbox-convergence.md)。
- 最近几轮播放期与启动期 trace 表明，问题已经可以拆成两段：
  - `decode` 之前：
    - `continuationAcceptedWhileAwaitingIdr`
    - `bootstrapMissingIdr`
    - `episodeId=null`
    - startup / priming 与 recovery episode 责任归属不稳
  - `decode` 之后：
    - `pacerSubmit submitted`
    - `rendererSubmit`
    - `hostMailboxAccepted / hostFramePresented`
    - `supersededAfterDecode`
    - `mailboxOverwrite`
- 第一段属于恢复链缺主、bootstrap 与 episode binding 问题。
- 第二段不是简单的“显示侧 bug”，而是模型失配：
  - 前段在按恢复价值建模：`fresh anchor / recovery continuation / steady continuation / disposable`
  - 后段 `latest-mail` 仍主要按“最新帧覆盖旧帧”执行
  - trace 里能看到 decode 后链路确实在跑，但覆盖、替换、投递的语义不能稳定表达“为什么这帧该保留、为什么那帧该被淘汰”
- 当前系统因此存在两个口径：
  - 前段是恢复链价值模型
  - 后段是 latest-only 执行模型
- 两者之间缺少单一合同，结果是：
  - 前段认为“这帧有恢复价值”
  - 后段仍可能把它当普通 latest 候选覆盖掉
  - `supersededAfterDecode` 只表达结果，不表达价值合同是否被满足

## Goal

- 为 `decode -> pacer -> renderer -> host` 定义统一的 post-decode 价值合同。
- 保留单槽 / latest-only 执行主线，不引入双分叉或第二套恢复状态机。
- 将“恢复价值判定”收口到 pacer 单点，而不是散落在 renderer / host / owner 多处。
- 让 `supersededAfterDecode`、`mailboxOverwrite`、`hostMailboxAccepted` 都能表达覆盖与保留的价值原因。
- 让 `decode` 前后的语义边界稳定：
  - `decode` 前解决顺序、bootstrap、usable IDR、episode binding
  - `decode` 后只解决显示价值和 latest-only 执行对齐

## Scope

- In scope:
  - [`crates/xbxengine/core/src/media/video/decode/video_decode.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/media/video/decode/video_decode.rs)
  - [`crates/xbxengine/core/src/media/video/decode/actor.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/media/video/decode/actor.rs)
  - [`crates/xbxengine/core/src/media/video/pacer/actor.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/media/video/pacer/actor.rs)
  - [`crates/xbxengine/core/src/media/video/render/renderer.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/media/video/render/renderer.rs)
  - [`crates/xbxengine/core/src/media/video/render/actor.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/media/video/render/actor.rs)
  - [`crates/xbxengine/core/src/runtime_stats_sink.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/runtime_stats_sink.rs)
  - [`src-tauri/src/mods/xbxengine/trace_projection.rs`](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/mods/xbxengine/trace_projection.rs)
  - decode / pacer / renderer / host scheduling 相关测试与 trace DTO
- Out of scope:
  - startup / priming recovery bridge
  - usable IDR 供给不足本身
  - PLI / FIR / decoder reset / reconnect 升级条件
  - `decode` 之前的 H264 bootstrap、SampleBuilder、RTP reorder 重写
  - 引入双队列显示模型或第二套 post-decode 恢复状态机

## Problem Split

### A. `decode` 之前的问题

- 核心问题：恢复链缺主。
- 典型症状：
  - `boundEpisodeId=null`
  - `continuationAcceptedWhileAwaitingIdr`
  - `bootstrapMissingIdr`
  - startup / priming 长时间不切入正式 recovery episode
- 处理方式：
  - 归入恢复链 RFC 与启动期桥接方案
  - 不在本 RFC 内直接修改

### B. `decode` 之后的问题

- 核心问题：价值模型失配。
- 典型症状：
  - decode 后已经 `submitted -> rendererAccepted -> hostAccepted`
  - 但高价值恢复帧与普通 steady 帧使用同一覆盖语义
  - `supersededAfterDecode` / `mailboxOverwrite` 无法表达价值合同
- 处理方式：
  - 本 RFC 定义统一 post-decode 价值合同
  - latest-only 继续保留
  - 覆盖规则改成 value-aware latest-only

## Design

### 1. 单一 post-decode 合同

每个 decoded frame 从 `decode -> pacer` 开始必须携带统一价值元数据：

- `presentation_role`
  - `fresh_anchor`
  - `recovery_continuation`
  - `steady_continuation`
  - `disposable`
- `recovery_epoch`
- `recovery_owner_rtp_timestamp`
- `value_rank`
- `supersede_policy`

原则：

- `presentation_role` 是 decode 后唯一的高层价值语义。
- renderer / host 不再自行发明新的恢复价值分类。
- owner / recovery policy 不直接消费 renderer latest-slot 的局部覆盖结果来反推价值。

### 2. pacer 作为唯一价值收口层

- pacer 是 decode 后唯一价值决策层。
- pacer 负责：
  - 比较当前候选与新候选的显示价值
  - 生成最终 `submit / replace / drop / hold` 决策
  - 产出结构化覆盖原因
- renderer / host 负责：
  - 执行 latest-only handoff
  - 报告执行态事实
  - 不再升级价值语义

结果：

- 前段恢复模型与后段执行模型之间只有一个对齐点：pacer。
- renderer / host 不形成第二套恢复推理链。

### 3. value-aware latest-only

latest-only 保留，覆盖规则改成价值感知单槽：

1. `fresh_anchor` 高于所有其他角色
2. 同一 `recovery_epoch` 下：
   - `recovery_continuation` 高于 `steady_continuation`
   - 更新的 owner chain continuation 高于旧 continuation
3. `steady_continuation` 只覆盖同级或更低级
4. `disposable` 只能覆盖 `disposable`
5. 同价值下再比较：
   - 更新的 `rtp_timestamp`
   - 更新的 `frame_seq`
   - 更接近当前 display 时点的候选

禁止行为：

- `disposable` 覆盖 `fresh_anchor`
- 跨 `recovery_epoch` 的普通 steady 帧覆盖恢复锚点
- renderer / host 在 pacer 已做高价值保留后再次按本地规则降级

### 4. supersede contract

`supersededAfterDecode`、`mailboxOverwrite`、`host pending replaced` 必须升级为可归因覆盖事件，至少包含：

- `dropped_frame_seq`
- `dropped_rtp_timestamp`
- `dropped_presentation_role`
- `kept_frame_seq`
- `kept_rtp_timestamp`
- `kept_presentation_role`
- `same_recovery_epoch`
- `same_recovery_owner_chain`
- `supersede_reason`

`supersede_reason` 建议收口为：

- `higherRole`
- `newerWithinSameRole`
- `newerWithinSameRecoveryChain`
- `anchorProtection`
- `postAnchorClimbProtection`
- `displayDeadlineExpired`
- `localHardCapProtection`

### 5. renderer / host 的职责收缩

- renderer latest-slot：
  - 保留单槽暂存
  - 只执行 pacer 已经决定好的值
  - overwrite 只记录执行态，不改变价值层级
- host pending/displayed mailbox：
  - 保留单槽 pending + displayed
  - 继续负责真实上屏与 visible fact
  - 不新增独立恢复价值判断

这条约束直接避免：

- render 分叉一套价值模型
- host 再分叉一套显示保护模型
- owner 继续从执行结果侧反推恢复阶段

### 6. 与前段模型的接口

前段只输出统一价值事实，不直接控制 latest-mail：

- recovery / bootstrap / owner 负责定义：
  - 当前帧是否属于 `fresh_anchor`
  - 是否属于当前 recovery chain 的 `recovery_continuation`
  - 是否只是 steady continuation
- pacer 负责把这些事实落成 decode 后唯一执行合同

这意味着：

- 前段仍然专注于恢复链
- 后段专注于显示交付
- 二者通过 `presentation_role + epoch + owner_rtp` 对齐

## Compatibility

- 兼容现有 latest-only 主方向，不回退到历史队列模型。
- 兼容现有 `CleanAnchorCommitted -> DisplayStable` 双 gate。
- 兼容现有 `decodePacerSubmit`、`frameDropped`、`hostMailbox*` 观测链。
- 现有 trace 字段可以先增量扩展，不需要一轮内替换全部 phase / state 字符串。

## Plan

1. 定义统一 `post-decode frame meta` 与 `presentation_role` 合同。
2. 将 pacer comparator 改成 value-aware latest-only comparator。
3. 将 renderer / host 覆盖事件升级为结构化 supersede contract。
4. 清理 owner / policy 对 renderer latest-slot 局部结果的隐式依赖。
5. 用 trace 与定向测试验证“单槽执行 + 单点价值收口”没有长出第二套分叉。

## Validation

- [ ] `cargo test -p xbxengine media::video::decode -- --nocapture`
- [ ] `cargo test -p xbxengine media::video::pacer::actor -- --nocapture`
- [ ] `cargo test -p xbxengine media::video::render::renderer -- --nocapture`
- [ ] `cargo test -p xbxrc --lib native_video::scheduling -- --nocapture`
- [ ] 新增定向测试：`fresh_anchor` 不被低价值 steady continuation 覆盖
- [ ] 新增定向测试：同 recovery chain 的 continuation 可以续写显示
- [ ] 新增定向测试：`disposable` 不能覆盖当前 recovery anchor
- [ ] 新增定向测试：`supersededAfterDecode` 输出 kept/dropped 双方价值合同
- [ ] 用 runtime trace 回放验证 pacer/renderer/host 不再各自产生平行价值语义

## Risks

- 如果 `presentation_role` 定义过粗，会把 pacer comparator 再次做成隐式 if-else 集合，后续难维护。
- 如果 renderer / host 仍保留隐式二次比较，系统会继续存在“双决策点”。
- 如果 owner / policy 继续读取执行层 overwrite 结果来反推恢复阶段，价值合同仍会分叉。

## Progress

- [ ] Step 1: 补齐 post-decode 统一价值字段与 DTO
- [ ] Step 2: 重写 pacer comparator 为 value-aware latest-only
- [ ] Step 3: renderer / host overwrite 事件结构化
- [ ] Step 4: 清理 session/owner 对局部 overwrite 的反向依赖
- [ ] Step 5: trace 回放与定向测试收口

## Execution Notes

- Date: 2026-04-30 | Status: planned
- Update: 新建增量 RFC，将问题明确拆成 `decode` 前恢复链缺主 与 `decode` 后价值模型失配 两部分。
- Decision: `decode` 后继续保留 single-track latest-only，不引入双分叉或第二套恢复状态机。
- Decision: pacer 作为 decode 后唯一价值决策层，renderer / host 只保留执行态 latest-mail 语义。
- Risk/Blocker: 现有 `supersededAfterDecode / mailboxOverwrite` 仍只有结果语义，缺少 kept/dropped 双边价值合同，后续实现前需要先统一 DTO。
