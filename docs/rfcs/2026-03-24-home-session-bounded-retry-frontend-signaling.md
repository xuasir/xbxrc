# Home Session Bounded Retry Frontend Signaling RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 已完成
- Current State: completed
- Owner: TBD
- Last Updated: 2026-04-09

## Background

- home ready 流程已经收敛为 SmartGlass-only，`waitingConsoleReady` 只认 SmartGlass 显式注册信号。
- home `waitingSessionReady` 已经存在一次 `ServerNeverRegistered / WaitingForServerToRegister` 的 bounded recreate，但前端还拿不到“bounded retry 已触发”的结构化状态。
- 当这次 bounded retry 之后仍然失败时，当前 UI 仍只会落到泛化的 `sessionReadyFailed`，没有明确告诉用户这是主机端注册/就绪问题。

## Goal

- 将 home `consoleReady` 路径明确固定为 SmartGlass ready gate。
- 将 `ServerNeverRegistered / WaitingForServerToRegister` 明确建模为 home session 的 bounded retry 信号，并把“retrying / exhausted”状态结构化透传给前端。
- 当唯一一次 bounded retry 仍失败时，向用户输出明确的主机端建议：重启主机，或等待主机开机一段时间后再连接。

## Scope

- In scope:
  - `crates/xbox-streaming/src/session/flow.rs`
  - `src-tauri/src/mods/streaming/service.rs`
  - `src-tauri/src/mods/streaming/types.rs`
  - `src/shared/rpc/streaming.ts`
  - `src/streaming/*`
  - `src/i18n/locales/*.json`
- Out of scope:
  - streaming 页面新的视觉提示组件
  - SmartGlass host 数据源本身
  - 非 home 场景的 startup 补偿

## Plan

1. 在 session flow 中把 home bounded retry 的 `retrying / exhausted` 状态显式化，并在 exhausted 时产出稳定错误信号。
2. 在 tauri / shared RPC 层补齐 bounded retry 结构化字段和错误分类。
3. 更新前端错误文案与状态消费，并补回归测试、报告和任务追踪。

## Validation

- [x] `cargo fmt -p xbox-streaming`
- [x] `cargo fmt -p xbxrc`
- [x] `cargo test -p xbox-streaming home_server_registration_retry_exhausted_is_terminal_host_issue -- --nocapture`
- [x] `cargo test -p xbox-streaming waiting_for_server_registration_retry_signal_is_bounded_retry -- --nocapture`
- [x] `cargo test -p xbxrc host_registration_retry_exhausted_maps_to_host_issue -- --nocapture`
- [x] `cargo check -p xbox-streaming`
- [x] `cargo check -p xbxrc`

## Risks

- 如果 bounded retry 状态设计得过于绑定当前实现，后续前端扩展提示方式时可能还要再调整字段。
- 如果 exhausted 判定不够严格，可能把非主机端故障误提示成“重启主机后再试”。

## Progress

- [x] Step 1: 已确认 SmartGlass-only ready gate 与单次 bounded recreate 现状。
- [x] Step 2: 已补 bounded retry 的 `retrying / exhausted` 结构化状态，并新增 exhausted 主机端错误分类。
- [x] Step 3: 已完成定向验证，并补齐 Report 与任务追踪。

## Execution Notes

- Date: 2026-03-24 | Status: completed
- Update: 已在 session flow 中增加 bounded retry observer 事件，tauri/shared RPC/front-end 共享类型新增 `boundedRetry` 字段；页面加载层在 bounded retry 发起时会显示“主机端仍在注册，正在自动重试一次连接”，而唯一一次 retry 仍失败时，失败层会稳定展示主机端建议。
- Decision: bounded retry 仍保持一次；前端状态通过结构化字段透传，不再依赖解析诊断字符串。
- Risk/Blocker: `pnpm exec vue-tsc --noEmit` 仍受仓库内既有的 `src/App.vue` / `src/pages/Setting.vue` 类型错误影响；本轮新增的 `src/pages/XStreamMainView.vue` 通过定向 `eslint` 检查。
