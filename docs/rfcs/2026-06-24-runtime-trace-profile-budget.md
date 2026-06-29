# Runtime Trace Profile / Budget / Dimension RFC

## Status

- Completion: 完成
- Current State: complete
- Owner: agent
- Last Updated: 2026-06-24

## Background

- 控制台日志需要保持清爽，诊断主信息应沉淀到 runtime trace。
- 旧 `runtime_trace_mode` 使用 `minimal/standard/verbose/trace` 多档强度，release 又被强制 `off`，导致生产问题缺少关键证据。
- 现有 trace 只有 category，没有 profile、dimension、importance，难以稳定组合开启诊断面，也缺少文件预算。

## Goal

- 将 trace profile 收敛为 `off | production | dev`。
- release 默认 `production` 关键 trace 常开，debug 默认 `dev`。
- 为每行 trace 增加 `traceProfile/dimension/importance`，保留 `traceMode` 兼容字段。
- 为生产与 dev 分别建立文件滚动和保留预算。
- 同步更新 `analyze-runtime-logs` skill 与脚本，让后续分析按 v3 schema 读取。

## Scope

- In scope:
  - `src-tauri/src/mods/runtime_trace/*`
  - config 默认值、归一化与 live apply
  - runtime trace RPC / TS 类型
  - 设置页 trace mode 选项文案
  - `.agents/skills/analyze-runtime-logs` schema / playbook / scripts
- Out of scope:
  - 媒体、传输、恢复控制策略本身
  - 新 UI 维度开关；维度组合走开发开关或隐藏配置

## Plan

1. 落地 Rust profile、dimension、importance、metadata 与生产/dev 预算。
2. 接入配置和启动路径，release 中 `dev` 降级为 `production`，生产忽略 dimensions。
3. 更新前端 RPC 类型和设置文案。
4. 更新 analyze-runtime-logs skill、schema 文档和脚本测试。
5. 跑 Rust / Python / TS 验证并收口。

## Validation

- [x] `cargo fmt`
- [x] `cargo test -p xbxrc runtime_trace --lib`
- [x] `cargo test -p xbxrc config --lib`
- [x] `cargo test -p xbxengine diagnostics --lib`
- [x] `cargo check -p xbxrc`
- [x] `PYTHONPYCACHEPREFIX=/private/tmp/codex_pycache python3 -B -m unittest discover .agents/skills/analyze-runtime-logs/tests`
- [x] `python3 /Users/guo.xu/.codex/skills/.system/skill-creator/scripts/quick_validate.py .agents/skills/analyze-runtime-logs`
- [x] `pnpm lint:fix`
- [x] `./node_modules/.bin/vitest run src/player/infra/render/Renderers.test.ts src/shared/gamepad/wait-pad-neutral.test.ts src/streaming/diagnostics.test.ts`
- [x] `git diff --check`
- [x] `git diff --cached --check`

## Risks

- 生产 trace 默认常开会增加少量磁盘写入；通过 16MB x 5 文件预算控制上限。

## Progress

- [x] Step 1: Rust trace profile、metadata、filter 和预算已实现。
- [x] Step 2: config / startup / live apply 已接入。
- [x] Step 3: TS RPC 类型和设置文案已更新。
- [x] Step 4: analyze-runtime-logs skill / schema / playbook / summarize 脚本已同步 v3 profile、budget、dimension、importance。
- [x] Step 5: 代码侧、skill 与前端 lint 验证已完成。

## Execution Notes

- Date: 2026-06-24 | Status: in-progress
- Update: `RuntimeTraceRecorder` 已输出 schema v3 envelope，新增 `traceProfile/dimension/importance`，生产 profile 固定保留 key/essential，dev 支持 `XBX_TRACE_DIMENSIONS` 和 hidden `runtime_trace_dimensions`。
- Decision: `native_video` 现有 `record_log` 无 level 的结构化事件按 `native_video/key` 处理，避免生产 trace 丢失关键显示证据。
- Date: 2026-06-24 | Status: in-progress
- Update: 验证已通过 `cargo fmt`、`cargo test -p xbxrc runtime_trace --lib`、`cargo test -p xbxrc config --lib`、`cargo test -p xbxengine diagnostics --lib`、`cargo check -p xbxrc`、`python3 -B -m unittest discover .agents/skills/analyze-runtime-logs/tests`、`git diff --check`、`git diff --cached --check`。
- Risk/Blocker: `pnpm lint:fix` 被既有 `Renderers.test.ts:1040`、`wait-pad-neutral.ts:58`、`streaming/diagnostics.ts:246` 阻塞。
- Date: 2026-06-24 | Status: validation-followup
- Update: `.agents/skills/analyze-runtime-logs` 已同步日志实现：Quick Start 增加 v3 profile/dimension/importance 读法，`log-schema.md` 记录 `off/production/dev`、旧模式映射、预算轮转、`traceBudgetNotice`，`analysis-playbook.md` 增加 profile 检查步骤，`summarize_runtime_trace.py` 输出 `traceModes/traceProfiles/dimensions/importance/traceBudgetNotices`，并补充 schema v3 黑盒测试。
- Validation: `PYTHONPYCACHEPREFIX=/private/tmp/codex_pycache python3 -B -m unittest discover .agents/skills/analyze-runtime-logs/tests`、`python3 /Users/guo.xu/.codex/skills/.system/skill-creator/scripts/quick_validate.py .agents/skills/analyze-runtime-logs`、`git diff --check`、`git diff --cached --check`。
- Date: 2026-06-24 | Status: complete
- Update: 前端既有 lint 已收口：`Renderers.test.ts` 的未用 video frame callback mock 参数改为 `_callback`，`wait-pad-neutral.ts` 的 abort 回调改成先声明后赋值，`diagnostics.ts` 移除未使用的 `displaySupplyDominates`。
- Validation: `pnpm lint:fix`、`./node_modules/.bin/vitest run src/player/infra/render/Renderers.test.ts src/shared/gamepad/wait-pad-neutral.test.ts src/streaming/diagnostics.test.ts`。
- Report: [`docs/reports/2026-06-24-runtime-trace-profile-budget.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/reports/2026-06-24-runtime-trace-profile-budget.md)
