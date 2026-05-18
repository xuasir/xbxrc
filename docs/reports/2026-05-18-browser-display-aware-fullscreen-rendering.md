# Browser Display-Aware Fullscreen Rendering Report

> 说明：本 Report 对应 RFC 首轮实现闭环。

## Summary

- Related RFC: [`docs/rfcs/2026-05-18-browser-display-aware-fullscreen-rendering.md`](docs/rfcs/2026-05-18-browser-display-aware-fullscreen-rendering.md)
- 浏览器侧 render policy 现在会编译 `presentTarget`，把显示尺寸、DPR、全屏状态、刷新率与源分辨率一起投影到 renderer attach / runtime snapshot / diagnostics；标准 `webgl2` renderer 也已拆开 source texture size 与 output present target size。

## Postscript

- fullscreen `presentTarget` 后续再加了一层 configured/source 上限约束：`1080p` 模式在 `1920x1200 @ 1.5 DPR` 这类显示器上不再自动抬到 `2880x1620` backing，而是保持 `1920x1080` 输出合同，避免不同本机 DPR 把同一 `1080p home LAN` 线路拉成不同的本地 GPU 负载。
- 浏览器刷新率读取不再只依赖 `window.screen.frameRate`；`browser-runtime` 现在在缺失该字段的平台上用 `requestAnimationFrame` 采样估算显示刷新率，并把估算结果写回 display-aware render policy。
- SR fixed tier 语义也同步收紧为“只把低于目标档位的源补到目标档位”：`1080 configured + 1080 source` 保持 `1080p`，`1440 source` 也保持 `1440p`，同档路径统一走 `RCAS-only`。

## Delivered

- [`src/streaming/runtime/browser-video-display.ts`](docs/../src/streaming/runtime/browser-video-display.ts)：新增 `resolveDisplayViewport()`，统一 fullscreen `Contain` / 显式 aspect-ratio 的 CSS viewport 与 backing size 计算。
- [`src/streaming/runtime/browser-render-policy.ts`](docs/../src/streaming/runtime/browser-render-policy.ts)：`BrowserRendererPlan` 新增 `presentTarget`，`planToRendererRuntimeConfigPatch()` / `planToRendererAttachSpec()` 同步投影显示上下文。
- [`src/player/domain/media.ts`](docs/../src/player/domain/media.ts)：新增 `RendererPresentTarget`，把 present target 合同接入 `RendererAttachSpec` / `RendererRuntimeConfig` / `StreamStats`。
- [`src/player/infra/render/Renderers.ts`](docs/../src/player/infra/render/Renderers.ts)：标准 `webgl2` 现在区分 source resolution 与 output resolution，shader 的采样 texel 基于 source，默认 framebuffer 输出基于 present target。
- [`src/player/infra/render/SuperResolutionWebGL2Renderer.ts`](docs/../src/player/infra/render/SuperResolutionWebGL2Renderer.ts)：SR canvas 跟随同一 viewport 合同更新 CSS 显示尺寸。
- [`src/streaming/runtime/browser-runtime.ts`](docs/../src/streaming/runtime/browser-runtime.ts)、[`src/streaming/diagnostics.ts`](docs/../src/streaming/diagnostics.ts)、[`src/streaming/types.ts`](docs/../src/streaming/types.ts)：新增 `renderDisplay* / renderPresentTarget* / renderViewport* / renderSource*` 观测字段。

## Changes

- **Present target**：标准 `webgl2` 不再把 canvas backing store 锁死在 `video.videoWidth/video.videoHeight`；当 policy 提供 `renderOutputWidth/renderOutputHeight` 时，source texture 继续按源尺寸上传，present framebuffer 按显示目标输出。
- **Fullscreen 16:9 viewport**：浏览器 display helper 与 renderer attach 共用 `resolveDisplayViewport()`；`Contain` 全屏场景会按源宽高比精确拟合，`1920x1200` 容器上的 `1920x1080` 视口仍保持不变，但 fullscreen backing size 现在还会继续受 configured/source 上限裁剪。
- **Diagnostics**：`snapshotStats()` 和 stream diagnostics 现在能直接回答“当前显示是否全屏、刷新率是多少、present target 多大、viewport 多大、源分辨率多大”。

## Validation

- `./node_modules/.bin/vitest run src/streaming/runtime/browser-render-policy.test.ts src/streaming/runtime/browser-video-display.test.ts src/player/infra/render/Renderers.test.ts src/streaming/runtime/browser-runtime.test.ts`
- `./node_modules/.bin/vitest run src/streaming/diagnostics.test.ts`
- `pnpm lint:fix`
- `pnpm build`

## Risks

- `vitest` 在当前环境仍被本地 `@rollup/rollup-darwin-arm64` optional dependency 的签名问题阻断，本轮只完成了 `tsc --noEmit` 与目标文件级 `eslint` 静态验证。
- 若后续要让 SR backing store 进一步跟随更细粒度的 display budget，而不仅是 configured target 上限，仍需单独评估 `FSR1` 质量与成本。

## Follow-up

- 可在真实 `1080p home LAN + Windows AMD APU + fullscreen` 场景补一轮视觉验证，确认普通 `webgl2` 与 `webgl2_sr` 的锐化观感和 GPU 占用都符合预期。
