# Streaming Session Anti-Corruption Refactor Report

- Related RFC: [`docs/rfcs/2026-03-24-streaming-session-anti-corruption-refactor.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-03-24-streaming-session-anti-corruption-refactor.md)

## Delivered

- startup error 语义已从 `src-tauri/src/mods/streaming/service.rs` 下沉到 domain hint，adapter 不再把字符串分类当成主路径。
- progress failure 已接入统一结构化 error contract，前端失败展示不再直接消费裸 `errorMessage`。
- `useStreamExecution` 已收口为 reducer + 单一 apply 入口，避免 startup/progress/runtime 三路并发写同一批 UI 状态。
- session progress 对外 contract 已移除假 `retryCount`，bounded retry 成为唯一保留的真实重试计数字段。
- session client profile 已重新对齐：cloud/home 的 `clientAppId` 统一为 `www.xbox.com`，UA 跟随 device profile 输出，home 的 display target 跟随当前 device profile 分辨率，`x-ms-device-info` 不再输出 `unknown` 设备模型。
- 已补 domain/Tauri 守护测试，锁住 progress hint 生成、fallback 结构化映射、bounded retry 提取和网络恢复语义。

## Changed

- `crates/xbox-streaming/src/session/flow.rs`
  - 新增 `SessionFlowStartupErrorHint` / `SessionProgressSnapshot.error_hint`。
  - 为 startup/progress 错误建立统一的结构化语义入口。
  - 删除 session progress 中占位的 `retry_count`。
- `src-tauri/src/mods/streaming/service.rs` / `src-tauri/src/mods/streaming/types.rs`
  - 新增 `StreamingSessionError`。
  - `start_session` / `get_session_progress` 统一投影结构化 progress error。
  - fallback progress 映射也会稳定生成结构化错误，而不是退回 UI 自行猜测。
- `src/shared/rpc/streaming.ts` / `src/streaming/session.ts` / `src/streaming/useStreamExecution.ts`
  - 前端 progress contract 改为消费 `error` 而非裸字符串。
  - `useStreamExecution` 的 `statusText` / `sessionUiPhase` / `lifecyclePhase` / `error*` / `startupBoundedRetry` 统一经 reducer 更新。
  - 删除 `SessionHealthSnapshot` 与 shared progress DTO 中的假 `retryCount`。
- `crates/xbox-streaming/src/policy/session/compiler.rs`
  - `clientAppId` 统一为 `www.xbox.com`。
  - cloud/home 都会输出 UA，且按 device profile 组装。
  - home `x-ms-device-info.displayInfo` 改为跟随当前 device profile 分辨率。
  - 设备信息改为输出稳定 `make/model/os/browser` 组合。

## Validation

- `cargo fmt -p xbox-streaming`
- `cargo fmt -p xbxrc`
- `cargo check -p xbox-streaming`
- `cargo check -p xbxrc`
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
- `pnpm exec eslint src/streaming/session.ts src/streaming/useStreamExecution.ts src/shared/rpc/streaming.ts`

## Residual Risk

- `pnpm exec vue-tsc --noEmit` 仍被仓库既有问题阻塞：`src/App.vue`、`src/pages/Setting.vue`，本任务未引入新的 TS 错误。
- 当前仓库没有前端测试 runner，`useStreamExecution` reducer 只能通过现有 ESLint/类型检查和链路侧守护测试间接保护。

## Follow-up

- 若后续继续演进 streaming flow，优先在 domain/shared contract 上加语义，不再回到 adapter/message string 分流。
- 若仓库后续接入前端测试基建，优先为 `useStreamExecution` reducer 抽纯函数单测，锁住 failed/closed 优先级与晚到事件保护。
