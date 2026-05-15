# Home Direct-First TURN Fallback And ICE Alignment Report

> 说明：本 Report 仅用于复杂任务完全完成后的最终总结，不用于记录执行中的阶段性进度；中间过程应持续回写到对应 RFC。

## Summary

- Related RFC: [`docs/rfcs/2026-03-24-home-direct-first-turn-fallback-and-ice-alignment.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-03-24-home-direct-first-turn-fallback-and-ice-alignment.md)
- 已完成 home rust-owned 串流的“先直连、失败后单次 fallback TURN”主线，并把远端 ICE candidate 归一化语义进一步对齐到 `参比实现`。

## Delivered

- `src/streaming/runtime/runtime-host.ts` 现在会在 `home + rust-owned + turnSource==='fallback'` 时首轮去掉 TURN，并在预连接失败后单次切回 fallback TURN。
- `src/streaming/types.ts` 与 `src/streaming/useStreamExecution.ts` 已把 `turnSource` 透传到 runtime launch spec，保证 runtime host 能按 metadata 正确编排尝试策略。
- `crates/xbox-streaming/src/session/signaling/ice.rs` 已对齐候选重排、Teredo 补偿、foundation/priority 重写与 `a=end-of-candidates` 收敛，并补回归测试。
- `crates/xbxengine/core/src/api/runtime/lifecycle.rs`、`crates/xbxengine/core/src/transport/rtc/connection/builder.rs`、`crates/xbxengine/core/src/transport/rtc/connection/lifecycle.rs` 已补充 ICE/TURN 诊断日志，便于区分直连首轮和 fallback 次轮。

## Changes

- direct-first/fallback 的切换 owner 放在 runtime host 层，不改动 session create 主线与既有 recovery owner。
- fallback 重试仅允许在首轮尚未 `connected` 时触发一次，避免和 transport reconnect 叠加。
- runtime host 增加启动尝试与 fallback 切换日志，同时补齐 fallback launch 失败时的本地清理，避免残留 runtime 状态。
- ICE normalize 现在按输出顺序重写 foundation，并让首个候选 priority 对齐 `2130706431`、其余候选降为 `1`，与 `参比实现` 行为保持一致。

## Validation

- `cargo fmt --all`
- `cargo check -p xbxengine`
- `cargo test -p xbox-streaming session::signaling::ice -- --nocapture`

## Risks

- 本轮前端静态检查未运行：当前环境缺少 `node` / `pnpm`，后续需在具备 Node 工具链的环境补跑。
- direct-first/fallback 目前采用同一 session 内重启 runtime 的最小实现；若后续 trace 证明服务端要求重建 session，需再调整重试 owner。

## Follow-up

- 用新的 home trace 复现一次，确认首轮日志显示 `turn=direct`，失败后次轮显示 `turn=fallback`。
- 在具备 Node 工具链的环境执行前端 lint / typecheck，收口 TS 静态验证。
