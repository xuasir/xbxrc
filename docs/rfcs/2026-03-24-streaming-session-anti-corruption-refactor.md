# Streaming Session Anti-Corruption Refactor RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 已完成
- Current State: completed
- Owner: TBD
- Last Updated: 2026-04-09

## Background

- 当前 streaming session 流程已经出现明确的腐化征兆：
  - `src-tauri/src/mods/streaming/service.rs` 在 adapter 层重复承载 startup error 策略与字符串语义解析。
  - `src/streaming/useStreamExecution.ts` 同时消费 startup event、progress 和 runtime phase，形成三路并发状态机写同一批 UI 状态。
  - startup 失败与 progress 失败走了两套错误呈现路径，结构化错误和裸字符串并存。
  - `SessionProgressSnapshot.retryCount` 仍是占位字段，但前端仍把它当成真实 contract 暴露。
- 如果继续在现有结构上叠加 home / cloud / retry / runtime 逻辑，后续每次改动都会同步扩大这几个腐化点。

## Goal

- 收口 session startup / progress / runtime 三段边界，避免多层重复承载策略。
- 让 startup error 与 progress error 统一走结构化 contract，而不是继续依赖 message string。
- 清理已经失真的 contract 字段，避免“字段还在但语义已死”。

## Scope

- In scope:
  - `crates/xbox-streaming/src/session/flow.rs`
  - `crates/xbox-streaming/src/session/scheduler.rs`
  - `src-tauri/src/mods/streaming/service.rs`
  - `src-tauri/src/mods/streaming/types.rs`
  - `src/shared/rpc/streaming.ts`
  - `src/streaming/session.ts`
  - `src/streaming/useStreamExecution.ts`
  - `src/pages/XStreamMainView.vue`
  - session flow / anti-corruption 相关文档与任务追踪
- Out of scope:
  - WebRTC / ICE / transport 细节重写
  - UI 视觉重设计
  - 与 streaming session 无关的 TS / lint 历史遗留问题

## Problem Statement

### 1. Adapter duplication

- `StreamingService` 当前既做 adapter，又做 startup error 语义分类、message key 决策和 diagnostic summary 拼装。
- 这会导致 domain flow 每新增一种失败语义，都需要在 tauri adapter 再补一次映射逻辑。

### 2. Multi-writer UI state

- `useStreamExecution` 里 startup event、progress 轮询、runtime host callback 同时写 `statusText` / `lifecyclePhase` / `sessionUiPhase` / `error*`。
- 现状可以工作，但后续非常容易出现不同状态源互相覆盖的问题。

### 3. Split error pipeline

- startup 失败通过 `StreamingStartupError` 走结构化映射。
- progress 失败仍直接把 `progress.errorMessage` 原样显示给前端。
- 同一类 session 错误目前没有统一 contract。

### 4. Dead contract field

- `SessionProgressSnapshot.retryCount` 仍固定为 `0`，但前端仍保留并暴露该字段。
- 当前真实重试语义已经迁到 `boundedRetry`，旧字段继续存在只会误导调用方。

## Plan

1. 下沉 startup error 语义
2. 统一 session error contract
3. 收口 UI 状态写入入口
4. 清理或实现死字段 contract
5. 补测试与文档回写

## Proposed Changes

### Step 1. 下沉 startup error 语义到 domain / shared contract

- 目标：
  - 让 `StreamingService` 不再依赖 message string 做业务语义判断。
- 做法：
  - 在 `xbox-streaming` domain 层补稳定的 startup failure reason / startup diagnostic contract。
  - `StreamingService` 仅把 domain 结构映射到 tauri/shared 类型，不再自行 classify。

### Step 2. 统一 startup / progress 失败 contract

- 目标：
  - 页面侧不再区分“这是 startup error 还是 progress failure”来决定是否能拿到结构化语义。
- 做法：
  - 为 `SessionProgressSnapshot` 增加与 startup error 对齐的结构化 error projection，或引入统一 session error payload。
  - `applySessionProgress()` 不再直接展示裸 `errorMessage`。

### Step 3. 收口 `useStreamExecution` 的状态写入

- 目标：
  - 避免 startup event、progress、runtime callback 三路直接写同一批 ref。
- 做法：
  - 引入单一 reducer / 单一 apply 函数，明确三类状态源的优先级与覆盖规则。
  - `statusText`、`lifecyclePhase`、`sessionUiPhase`、`error*` 统一经同一个入口更新。

### Step 4. 处理 `retryCount` 腐化字段

- 方案二选一，以实现成本和当前 owner 为准：
  - A. 接上真实语义，让 `retryCount` 真正反映 startup / session retry 次数。
  - B. 从 shared contract 和前端移除该字段，避免对外暴露假数据。
- 默认建议：
  - 如果短期没有明确 consumer，优先走 B。

### Step 5. 补 anti-corruption 守护测试

- 需要新增或调整测试，至少覆盖：
  - startup failure reason 映射不再依赖 tauri adapter 自行猜测
  - progress failure 与 startup failure 使用统一 contract
  - UI reducer 在 startup / progress / runtime 三类输入下的优先级稳定
  - `retryCount` 的删除或真实语义在 contract 上一致

## Validation

- [x] `cargo fmt -p xbox-streaming`
- [x] `cargo fmt -p xbxrc`
- [ ] `cargo test -p xbox-streaming session_flow -- --nocapture`
- [ ] `cargo test -p xbxrc streaming -- --nocapture`
- [x] `cargo check -p xbox-streaming`
- [x] `cargo check -p xbxrc`
- [x] `pnpm exec eslint src/streaming/session.ts src/streaming/useStreamExecution.ts src/shared/rpc/streaming.ts`
- [ ] `pnpm exec vue-tsc --noEmit` 或记录既有阻塞项

实际已执行的定点测试：

- `cargo test -p xbox-streaming remote_console_not_ready_error_carries_structured_hint -- --nocapture`
- `cargo test -p xbox-streaming startup_timeout_error_carries_structured_hint -- --nocapture`
- `cargo test -p xbox-streaming home_server_registration_retry_exhausted_is_terminal_host_issue -- --nocapture`
- `cargo test -p xbox-streaming failed_progress_server_registration_signal_carries_structured_hint -- --nocapture`
- `cargo test -p xbox-streaming failed_progress_unknown_error_defaults_to_runtime_hint -- --nocapture`
- `cargo test -p xbox-streaming progress_without_error_has_no_structured_hint -- --nocapture`
- `cargo test -p xbox-streaming recovering_progress_network_signal_maps_network_hint -- --nocapture`
- `cargo test -p xbox-streaming home_session_headers_include_user_agent_and_follow_home_resolution -- --nocapture`
- `cargo test -p xbox-streaming home_session_display_target_is_dynamic_for_1440_profile -- --nocapture`
- `cargo test -p xbox-streaming cloud_session_headers_keep_custom_image -- --nocapture`
- `cargo test -p xbxrc host_registration_retry_exhausted_maps_to_host_issue -- --nocapture`
- `cargo test -p xbxrc domain_progress_hint_maps_to_structured_progress_error -- --nocapture`
- `cargo test -p xbxrc fallback_progress_registration_message_maps_structured_error -- --nocapture`
- `cargo test -p xbxrc fallback_progress_without_raw_error_keeps_structured_error_empty -- --nocapture`
- `cargo test -p xbxrc fallback_progress_network_message_maps_retryable_network_error -- --nocapture`
- `pnpm exec vue-tsc --noEmit` → 仍被既有问题阻塞：`src/App.vue`、`src/pages/Setting.vue`

## Risks

- 如果一步到位同时改 domain / adapter / page reducer，改动面会比较大，需要严格控制 write set。
- 如果只是“把字符串判断挪个地方”而没有收口 shared contract，本次重构会变成形式上的搬家。
- `retryCount` 如果直接删除，需要确认没有隐藏 consumer。

## Progress

- [x] Step 1: 已完成审查，明确 4 个腐化点与受影响层。
- [x] Step 2: 已将 progress failure 接入结构化 error contract，前端不再直接消费裸 `errorMessage`。
- [x] Step 3: 已把 `useStreamExecution` 的核心 UI 状态收口到 reducer + 单一 apply 入口，并给 runtime/startup 的晚到事件加失败态保护。
- [x] Step 4: 已从 session progress shared contract、Tauri DTO、frontend health snapshot 移除假 `retryCount`；bounded retry 继续保留真实计数。
- [x] Step 5: 已补 domain/Tauri 守护测试，并完成 session client profile 修正与回归覆盖。

## Execution Notes

- Date: 2026-03-24 | Status: completed
- Update: Step 1 已完成 startup error hint 下沉，Step 2 已完成 progress error 结构化投影；当前前端失败展示已优先消费统一 contract。
- Update: Step 3 已完成，`useStreamExecution` 的 `statusText` / `lifecyclePhase` / `sessionUiPhase` / `error*` / `startupBoundedRetry` 现在统一经 reducer 写入。
- Update: Step 4 已完成，session progress contract 不再对外暴露假 `retryCount`；唯一保留的 retry 计数 contract 是 bounded retry。
- Update: 已补 domain/Tauri 守护测试，覆盖“无错误不生成 hint”“recovering 网络信号映射”“fallback progress 字符串兜底映射”“bounded retry 提取”。
- Update: 已修 session 画像组装：cloud/home 的 `clientAppId` 统一为 `www.xbox.com`，UA 统一跟随 device profile 输出，home 的 display target 改为跟随当前 device profile 分辨率，不再写死 1080p，`x-ms-device-info` 不再输出 `unknown` 设备模型。
- Update: 本任务已完成，最终交付见 [`docs/reports/2026-03-24-streaming-session-anti-corruption-refactor.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/reports/2026-03-24-streaming-session-anti-corruption-refactor.md)。
- Update: `pnpm exec vue-tsc --noEmit` 仍然只暴露既有阻塞项：`src/App.vue` 与 `src/pages/Setting.vue`，本任务未新增新的类型错误。
- Decision: 先做 contract 与 owner 收口，再做页面状态 reducer；不接受继续在 adapter 或页面里追加策略分支。
- Risk/Blocker: 仓库当前仍有与本任务无关的历史 TS 类型错误，执行时需要区分新问题与既有阻塞。
