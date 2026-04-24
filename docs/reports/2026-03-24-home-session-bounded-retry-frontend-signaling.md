# Home Session Bounded Retry Frontend Signaling Report

- Related RFC: [`docs/rfcs/2026-03-24-home-session-bounded-retry-frontend-signaling.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-03-24-home-session-bounded-retry-frontend-signaling.md)

## Delivered

- home `waitingConsoleReady` 继续以 SmartGlass 显式注册为唯一 `consoleReady` gate，不回退到仅凭开机态放行。
- home `waitingSessionReady` 的 `ServerNeverRegistered / WaitingForServerToRegister` 现在被明确建模为 bounded retry 状态，包含 `retrying / exhausted` 两个结构化阶段。
- bounded retry 发起时会通过 startup 事件把 `boundedRetry` 结构化字段透传给前端；后续页面提示不再需要解析诊断字符串。
- 当唯一一次 bounded retry 后仍失败时，最终错误会稳定映射为主机端注册失败，并给出“重启主机或等待主机开机一会后再连”的用户提示。
- `XStreamMainView` 已在加载层接入 bounded retry 进行中的状态文案，并在失败层补充主机端帮助提示。

## Changed

- `crates/xbox-streaming/src/session/flow.rs`
  - 为 startup observer 新增 bounded retry 回调与结构化快照。
  - `decide_home_session_ready_recreate_retry()` 现在区分 `Retry` 与 `Exhausted`。
  - exhausted 时返回稳定的 `homeSessionBoundedRetryExhausted:*` 错误，而不是继续落回泛化的 `streamingStartFailed`。
- `src-tauri/src/mods/streaming/service.rs`
  - `StartupAttemptRecorder` 新增 bounded retry 状态缓存与 startup event 透传。
  - startup error 分类新增 `HostRegistrationRetryExhausted`，并补诊断摘要映射。
- `src-tauri/src/mods/streaming/types.rs` / `src/shared/rpc/streaming.ts`
  - 新增 `StreamingStartupBoundedRetry*` 类型，并在 `StreamingStartupEvent` / `StreamingStartupError` 中加入 `boundedRetry` 字段。
- `src/streaming/session.ts` / `src/streaming/useStreamExecution.ts`
  - 前端解析并保留 `boundedRetry` 状态，供后续页面提示直接消费。
- `src/pages/XStreamMainView.vue`
  - 加载层在 `boundedRetry.status=retrying` 时显示主机注册中的自动重试说明。
  - 失败层在 `boundedRetry.status=exhausted` 时补充主机端帮助提示。
- `src/i18n/locales/zh.json` / `src/i18n/locales/en.json`
  - 新增 retrying 状态文案和 retry exhausted 的主机端帮助文案。

## Validation

- `cargo fmt -p xbox-streaming`
- `cargo fmt -p xbxrc`
- `cargo test -p xbox-streaming home_server_registration_retry_exhausted_is_terminal_host_issue -- --nocapture`
- `cargo test -p xbox-streaming waiting_for_server_registration_retry_signal_is_bounded_retry -- --nocapture`
- `cargo test -p xbxrc host_registration_retry_exhausted_maps_to_host_issue -- --nocapture`
- `cargo check -p xbox-streaming`
- `cargo check -p xbxrc`
- `pnpm exec eslint src/pages/XStreamMainView.vue`

## Residual Risk

- `pnpm exec vue-tsc --noEmit` 仍被仓库既有的 `src/App.vue` / `src/pages/Setting.vue` 类型错误阻塞，本轮 streaming 相关改动没有新增额外 typecheck 错误。
