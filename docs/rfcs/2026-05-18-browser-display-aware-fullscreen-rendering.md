# Browser Display-Aware Fullscreen Rendering RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 已完成
- Current State: completed
- Owner: Codex
- Last Updated: 2026-05-18

## Background

- 当前标准 `webgl2` renderer 会把 canvas backing store 直接绑定到 `video.videoWidth/video.videoHeight`。
- 这条合同在 `1080p home LAN + Windows AMD APU + 全屏` 场景下容易放大两类问题：
  - 输出目标分辨率和显示目标分辨率脱节，导致全屏链路出现额外缩放与软化
  - 浏览器 render policy 无法感知显示尺寸、DPR、刷新率与全屏状态，无法给出稳定的 present target
- 当前浏览器 display helper 只覆盖 DOM `video` 的 object-fit 与 aspect-ratio 视口，`webgl2` canvas 和 SR canvas 没有进入同一套显示合同。

## Goal

- 让标准 `webgl2` renderer 将 source texture size 与 output present target size 拆开。
- 为浏览器全屏模式引入精确 `16:9` 视口合同，并让 DOM `video`、标准 `webgl2` canvas、SR canvas 共用该合同。
- 将 `display size / DPR / refresh rate / fullscreen state` 纳入浏览器 render policy 与 diagnostics。

## Scope

- In scope:
  - `src/streaming/runtime/browser-render-policy.ts`
  - `src/streaming/runtime/browser-runtime.ts`
  - `src/streaming/runtime/browser-video-display.ts`
  - `src/player/domain/media.ts`
  - `src/player/infra/render/Renderers.ts`
  - `src/player/infra/render/SuperResolutionWebGL2Renderer.ts`
  - `src/streaming/types.ts`
  - 相关 diagnostics / tests
- Out of scope:
  - Rust 侧 native presenter
  - transport / bitrate / BWE 逻辑重写
  - SR 算法替换
  - 16:9 之外的新用户显示模式设计

## Proposed Contract

### 1. 显示上下文进入浏览器 render policy

- `BrowserRendererPolicyInput` 新增显示上下文：
  - display width / height
  - `devicePixelRatio`
  - fullscreen state
  - display refresh rate
- `BrowserRendererPlan` 新增 `presentTarget`：
  - output backing width / height
  - CSS viewport width / height
  - source width / height
  - viewport mode

### 2. player attach 合同携带 present target

- `RendererAttachSpec` 新增 `presentTarget` 字段。
- `PlaybackService` 与 renderer 按 attach spec 接收目标输出尺寸和视口尺寸。
- 标准 `webgl2` renderer 保持 source texture 按视频源尺寸上传，output canvas 按 present target 输出。

### 3. 全屏 16:9 精确视口

- 当格式为 `Contain` 且处于浏览器全屏时，优先计算精确 16:9 视口。
- 视口计算结果同时驱动：
  - DOM `video` 的 CSS 宽高
  - `webgl2` canvas 的 CSS 宽高
  - present target backing store 尺寸

### 4. diagnostics

- snapshot / diagnostics 新增：
  - `renderDisplayWidth/Height`
  - `renderDisplayRefreshHz`
  - `renderDisplayFullscreen`
  - `renderViewportWidth/Height`
  - `renderPresentTargetWidth/Height`
  - `renderSourceWidth/Height`

## Risks

- output target 脱离 source size 后，标准锐化 shader 需要显式区分 source resolution 与 output resolution。
- 全屏视口和 backing store 改为显示感知后，测试夹具里需要补更多 DOM 尺寸与 DPR 场景。
- SR renderer 与标准 renderer 共用 present target 合同后，需确保现有 SR output tier 语义保持稳定。

## Plan

1. 先补 render policy / renderer / display helper 失败测试。
2. 再扩展 `BrowserRendererPlan` 与 `RendererAttachSpec`。
3. 落地标准 `webgl2` source/output 分离。
4. 将 16:9 视口和 display context 接入 runtime snapshot 与 diagnostics。
5. 运行针对性 `vitest`、`pnpm lint:fix`，按影响面决定是否补 `pnpm build`。

## Validation

- [x] `pnpm vitest run src/streaming/runtime/browser-render-policy.test.ts`
- [x] `pnpm vitest run src/streaming/runtime/browser-video-display.test.ts`
- [x] `pnpm vitest run src/player/infra/render/Renderers.test.ts`
- [x] `pnpm vitest run src/streaming/runtime/browser-runtime.test.ts`
- [x] `pnpm vitest run src/streaming/diagnostics.test.ts`
- [x] `pnpm lint:fix`
- [x] `pnpm build`

## Progress

- [x] Step 1: 用户确认优先执行 `present target / 全屏 16:9 视口 / display-aware render policy`
- [x] Step 2: 补失败测试
- [x] Step 3: 实现 attach / policy / renderer 变更
- [x] Step 4: 补 diagnostics 与验证
