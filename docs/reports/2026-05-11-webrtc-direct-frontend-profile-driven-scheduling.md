# `webrtc-direct` 前端画像驱动调度 Report

> 说明：本 Report 对应 RFC 首轮实现闭环。

## Summary

- Related RFC: [`docs/rfcs/2026-05-11-webrtc-direct-frontend-profile-driven-scheduling.md`](docs/rfcs/2026-05-11-webrtc-direct-frontend-profile-driven-scheduling.md)
- 在浏览器直连运行时侧引入可单测的画像模块，将统一 warmup、带宽判定、质量/显示 dwell 与渲染自适应中的绝对 fps 门槛改为「基线 + 动态 overlay + 相对内容帧率」驱动，并补齐 snapshot / diagnostics / 关键 trace 的 `frontEnd*` 观测字段。

## Delivered

- [`src/streaming/runtime/browser-runtime-profile.ts`](src/streaming/runtime/browser-runtime-profile.ts)：baseline 分类、`buildRuntimeProfileClassification`、FPS 上沿窗口、`resolveExpectedContentFps`、`ProfilePolicyPreset` / `EffectiveFrontEndPolicy`、`evaluateProfileBandwidthState`、`explainFrontEndQualityUpshiftBlock`。
- [`src/streaming/runtime/browser-runtime.ts`](src/streaming/runtime/browser-runtime.ts)：连接与 `checkMediaStalled` 主链接入上述策略；重连 warmup 与 ICE 重启对齐画像；`resolveAdaptiveRenderProfile` / `classifyRecoveryCause` / `resolveRenderCause` 使用相对帧率或策略阈值。
- [`src/streaming/types.ts`](src/streaming/types.ts)、[`src/streaming/diagnostics.ts`](src/streaming/diagnostics.ts)：扩展 `frontEndProfileBaseline` 等字段。
- [`src/streaming/runtime/browser-runtime-profile.test.ts`](src/streaming/runtime/browser-runtime-profile.test.ts)、[`src/streaming/diagnostics.test.ts`](src/streaming/diagnostics.test.ts) 用例更新。

## Changes

- **Warmup**：`homeLan` / `homeRelay` / `cloud` 使用不同 `warmupDurationMs`；重连时按当前 `targetType` + 已知 `transportPath` 重建 `startup` 画像并设置 `warmupUntilMs` / `displayWarmupUntilMs`。
- **带宽状态**：`decodeFps/presentFps` 与 `expectedContentFps` 比较改为比率门槛；保留 loss / feedbackInterval / bitrate / age 等网络项。
- **观测**：`snapshotStats` 输出 `frontEndProfile*`、`frontEndExpectedContentFps`、`frontEndPolicyPreset`、`frontEndWarmupUntilMs`、`frontEndUpshiftBlockedReason`；部分 trace 事件附带 preset / 画像字段。

## Validation

- `pnpm vitest run src/streaming/runtime/browser-runtime-profile.test.ts src/streaming/diagnostics.test.ts`
- `pnpm lint:fix`

## Risks

- `expectedContentFps` 仍依赖近窗上沿与 `stats.fps` 启发式，极端抖动场景需实机继续调参。
- `xbxengine` 路径未填充 `frontEnd*`（字段可选），与 Rust 侧统一产出若需对齐需后续联调。

## Follow-up

- 实机验证：`home-lan` + 原生 30fps 源在稳定 inbound 下可升回 `L0`，且 cloud 会话无过早升档回归。
- 若后端提供显式「内容目标帧率」字段，应接入为 `resolveExpectedContentFps` 的最高优先级来源。
