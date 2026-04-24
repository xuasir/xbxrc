# Session Flow Canonicalization Report

- Related RFC: [`docs/rfcs/2026-03-24-session-flow-canonicalization.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-03-24-session-flow-canonicalization.md)

## Delivered

- 新增 [`docs/stream-session-flow.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/stream-session-flow.md)，把当前 desktop app 的 canonical session flow 固定下来。
- 文档覆盖了：
  - 前端 -> Tauri -> domain flow -> scheduler -> runtime host bridge 的整条主链
  - home / cloud 分叉
  - startup phase owner
  - scheduler steady-state 边界
  - runtime handshake 边界
  - 当前最容易继续腐化的四类风险和对应约束

## Key Decisions

- `crates/xbox-streaming/src/session/flow.rs` 是 session startup orchestration 的唯一 owner。
- `crates/xbox-streaming/src/session/scheduler.rs` 是 steady-state poller 的唯一 owner。
- `src-tauri/src/mods/streaming/service.rs` 只做 adapter / projection / trace，不再被视为策略层。
- `src/streaming/useStreamExecution.ts` 与页面层只消费结构化状态，不再扩张第二套 startup 策略。

## Validation

- 文档已对照当前代码入口：
  - `src/streaming/useStreamExecution.ts`
  - `src/streaming/session.ts`
  - `src-tauri/src/mods/streaming/service.rs`
  - `crates/xbox-streaming/src/session/flow.rs`
  - `crates/xbox-streaming/src/session/scheduler.rs`
  - `src-tauri/src/mods/xbxengine/runtime_state.rs`

## Residual Risk

- 这次只完成梳理，没有做代码层 anti-corruption refactor；后续如果继续在 Tauri service 或页面里塞策略，文档本身不会自动阻止腐化。
- 仓库里仍存在与本轮无关的既有 TS 类型错误，本次未处理。
