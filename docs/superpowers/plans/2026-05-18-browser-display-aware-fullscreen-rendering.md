# Browser Display-Aware Fullscreen Rendering Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让标准浏览器 `webgl2` 路径按显示目标而不是源视频尺寸输出，并在全屏场景稳定使用 16:9 精确视口，同时把显示尺寸、DPR、刷新率纳入浏览器 render policy 与 diagnostics。

**Architecture:** `browser-runtime` 收集显示上下文并编译成 `BrowserRendererPlan.presentTarget`，`RendererAttachSpec` 负责把 present target 送进 player 层。标准 `WebGL2VideoRenderer` 拆开 source texture size 与 output target size，SR renderer 和 DOM display helper 共享同一套 16:9 视口计算。

**Tech Stack:** Vue 3 + TypeScript + WebRTC + WebGL2

---

### Task 1: 任务登记与执行合同

**Files:**
- Create: `docs/rfcs/2026-05-18-browser-display-aware-fullscreen-rendering.md`
- Modify: `docs/project-task.md`

- [ ] 写入任务条目与 RFC 背景、目标、范围、验证项
- [ ] 明确本轮只覆盖标准 `webgl2` present target、全屏 16:9 视口、display-aware render policy

### Task 2: render policy 测试先行

**Files:**
- Modify: `src/streaming/runtime/browser-render-policy.test.ts`
- Modify: `src/streaming/runtime/browser-runtime.test.ts`

- [ ] 为 `BrowserRendererPlan` 增加 present target / display context 失败测试
- [ ] 为 fullscreen 16:9 视口和显示刷新率投影增加 runtime snapshot 失败测试

### Task 3: renderer 与 display helper 测试先行

**Files:**
- Modify: `src/player/infra/render/Renderers.test.ts`
- Modify: `src/streaming/runtime/browser-video-display.test.ts`

- [ ] 为标准 `webgl2` 输出目标脱离源尺寸增加失败测试
- [ ] 为 16:9 精确视口计算增加失败测试

### Task 4: render policy 与 attach 合同实现

**Files:**
- Modify: `src/streaming/runtime/browser-render-policy.ts`
- Modify: `src/player/domain/media.ts`
- Modify: `src/streaming/runtime/browser-runtime.ts`
- Modify: `src/streaming/types.ts`

- [ ] 给 `BrowserRendererPolicyInput/Plan` 增加显示上下文与 present target
- [ ] 给 `RendererAttachSpec` 增加 present target / viewport 合同
- [ ] 让 runtime snapshot 和 diagnostics 挂出显示感知字段

### Task 5: renderer 与 display helper 实现

**Files:**
- Modify: `src/player/infra/render/Renderers.ts`
- Modify: `src/player/infra/render/SuperResolutionWebGL2Renderer.ts`
- Modify: `src/streaming/runtime/browser-video-display.ts`

- [ ] 标准 `webgl2` 分离 source size 与 output target size
- [ ] 全屏模式计算并应用精确 16:9 视口
- [ ] SR renderer 跟随同一 display viewport 合同

### Task 6: 验证

**Files:**
- Test: `src/streaming/runtime/browser-render-policy.test.ts`
- Test: `src/streaming/runtime/browser-runtime.test.ts`
- Test: `src/streaming/runtime/browser-video-display.test.ts`
- Test: `src/player/infra/render/Renderers.test.ts`

- [ ] 运行相关 `vitest`
- [ ] 运行 `pnpm lint:fix`
- [ ] 视编译影响决定是否补 `pnpm build`
