# Browser Render Policy Compiler Unification RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 已完成（手工 trace 四条路径待补）
- Current State: done
- Owner: Codex
- Last Updated: 2026-05-15

## Background

- Rust 侧 `crates/xbox-streaming/src/policy` 已形成稳定的策略编译模型：
  - `Config` 表达用户偏好，允许保留 `Auto / Default`
  - `Context` 表达运行上下文与能力事实
  - `Plan` 表达无 `Auto` 的最终决策
  - `Projection` 表达给 runtime / UI 消费的最小合同
- 浏览器侧 `src/streaming/runtime/browser-runtime.ts` 目前直接把 `RuntimeLaunchSpec`、display options、WebGL2 capability、display degrade、SR 档位、fallback 状态编译成多次 `RendererRuntimeConfig` patch。
- `src/player/domain/media.ts` 的 `RendererRuntimeConfig` 同时承载用户显示配置、策略决策、renderer attach 参数、SR 会话状态与 fallback 状态。
- `src/player/app/media/PlaybackService.ts` 需要二次解释 `pipelineType / mode / superResolutionEnabled / superResolutionInactiveAfterFailure` 才能决定最终 renderer。
- 结果是浏览器 runtime 传参膨胀，诊断字段与配置字段混杂，SR 与 display degrade 合同容易漂移。

## Goal

- 借鉴 Rust policy 编译思路，为浏览器侧 render runtime 建立二段策略编译层。
- 将浏览器侧 renderer 决策收敛为显式 `BrowserRendererPlan`，再投影为底层 `RendererAttachSpec`。
- 让 `PlaybackService` 只消费最终 renderer attach 合同，不再读取混合语义的策略字段。
- 让 diagnostics 从 `BrowserRendererPlan` 投影，避免配置字段和观测字段互相污染。
- 保持 Rust 侧 `policy` 继续负责启动期、跨 runtime、可持久化、可复用的公共策略。

## Scope

- In scope:
  - `crates/xbox-streaming/src/policy/render/*`
  - `crates/xbox-streaming/src/policy/projection.rs`
  - `src-tauri/src/mods/streaming/types.rs`
  - `src/shared/rpc/streaming.ts`
  - `src/streaming/runtime/browser-runtime.ts`
  - 新增 `src/streaming/runtime/browser-render-policy.ts`
  - `src/streaming/runtime/super-resolution-ladder.ts`
  - `src/player/domain/media.ts`
  - `src/player/app/media/PlaybackService.ts`
  - `src/player/infra/render/*`
  - 浏览器 diagnostics / stream panel view model 中 renderer 相关字段
- Out of scope:
  - Rust-owned native video / wgpu presenter 调度重构
  - WebRTC transport、BWE、NACK、PLI、recovery owner 策略重构
  - FSR1 shader 算法替换
  - 新增 FSR2/FSR3、时域超分或 motion vector 管线
  - 设置页交互重组

## Target Architecture

### 1. Rust Public Policy Layer

Rust 侧继续输出启动期公共策略：

- `RenderConfig`：用户偏好与持久化配置
- `RenderPlan`：启动期最终 render plan
- `RenderPlanProjection`：前端 runtime 可消费的最小投影

首轮只扩展稳定、启动期可确定的字段：

- `pipelinePreference: auto | video | webgl2`
- `superResolutionPreference: off | fsr1Experimental`
- `fallbackProcessing: usm | cas`
- `initialTargetFps`
- `displayOptions`
- `videoFormat`

这些字段不承载浏览器运行期事实，例如 WebGL2 context 是否可用、video 实际尺寸、context lost、SR attach 失败。

### 2. Browser Local Policy Compiler

新增 `src/streaming/runtime/browser-render-policy.ts`，提供纯函数：

```ts
export function resolveBrowserRendererPlan(input: BrowserRendererPolicyInput): BrowserRendererPlan
```

输入只包含：

- Rust/Tauri 下发的 `StreamingRenderProjection`
- `StreamingRuntimeProjection`
- 浏览器能力事实：`webgl2Supported / rendererCapabilityReason`
- 当前 display state：`displayDegradeLevel / visibilityBudgetActive`
- 自适应渲染结果：`shaderPreset / sharpenStrength / processingMode / format`
- SR 状态：`enabled / tierPlan / attachFailed / fallbackReason`

输出是无 `auto`、无二义性的 `BrowserRendererPlan`：

```ts
export type BrowserRendererKind = 'video' | 'webgl2' | 'webgl2_sr'

export interface BrowserRendererPlan {
  kind: BrowserRendererKind
  source: 'auto' | 'userOverride' | 'capabilityFallback' | 'srFallback'
  targetFps: number
  display: {
    format: RendererRuntimeConfig['format']
    brightness: number
    contrast: number
    saturation: number
  }
  sharpening: {
    mode: 'none' | 'usm' | 'cas' | 'fsr1_rcas'
    preset?: 'clarityL0' | 'clarityL1' | 'clarityL2' | 'clarityL3'
    strength?: number
    processingMode?: 'quality' | 'performance'
  }
  sr?: {
    algorithm: 'fsr1'
    outputTier: '1080p' | '1440p' | '2160p'
    outputWidth: number
    outputHeight: number
    rcasStops: number
  }
}
```

### 3. Renderer Attach Contract

新增或收窄底层 attach 类型：

```ts
export interface RendererAttachSpec {
  kind: 'video' | 'webgl2' | 'webgl2_sr'
  targetFps: number
  format: RendererRuntimeConfig['format']
  brightness: number
  contrast: number
  saturation: number
  processing?: 'usm' | 'cas'
  processingMode?: 'quality' | 'performance'
  shaderPreset?: 'clarityL0' | 'clarityL1' | 'clarityL2' | 'clarityL3'
  sharpenStrength?: number
  sr?: {
    outputWidth: number
    outputHeight: number
    rcasStops: number
  }
}
```

`PlaybackService` 使用 `kind` 直接创建 renderer，SR fallback 后更新 `BrowserSuperResolutionState`，再请求 policy compiler 重新产出计划。

### 4. Browser SR State

将浏览器 runtime 当前散落的 SR 变量收成一个对象：

```ts
interface BrowserSuperResolutionState {
  enabled: boolean
  tierPlan: SuperResolutionTierPlan | null
  rcasStopsBase: number
  rcasStopsEffective: number
  attachFailed: boolean
  fallbackReason: string | null
  latestVideoDimensions: { width: number, height: number } | null
}
```

首轮按既有行为迁移。`webgl2_sr` 激活时 RCAS stops 仍经 `resolveDynamicSuperResolutionRcasStops`（拥塞、显示档位、入站码率 vs 基线），在固定 tier 合同上软化低码率下的锐化；`applyDynamicSrRcasForDisplayDegrade === false` 仅用于单测/显式关闭路径。

## Plan

1. 建立浏览器 render policy 类型与纯函数。
   - 新增 `browser-render-policy.ts`
   - 覆盖 `video / webgl2 / webgl2_sr / capabilityFallback / srFallback / visibility targetFps=0`
   - 单测锁定现有行为
2. 将 `browser-runtime.ts` 的 renderer 参数展开迁移到 `resolveBrowserRendererPlan()`。
   - `applyDisplayDegradeLevel()` 只准备 policy input
   - runtime 保存最近一次 `BrowserRendererPlan`
   - diagnostics 从 plan 投影
3. 收拢 SR runtime state。
   - 替换 `srOutputFrozen / srRcasStopsBase / srRcasStopsEffective / srAttachFailed / srFallbackReason / latestVideoDimensions`
   - `freezeSuperResolutionOutputIfNeeded()` 只更新 `BrowserSuperResolutionState`
   - fallback event 只更新 SR state，再触发 policy 重算
4. 收窄 player renderer attach 合同。
   - 引入 `RendererAttachSpec`
   - `PlaybackService` 按 `kind` 选择 renderer
   - 保留兼容适配层，降低一次性改动风险
5. 扩展 Rust render projection。
   - 在 `RenderPlanProjection` 增加启动期稳定字段
   - Tauri DTO 与 `src/shared/rpc/streaming.ts` 同步
   - 浏览器 policy input 改为消费 projection 字段
6. 按 SR RFC 修正合同漂移。
   - SR active 后固定使用 `FSR1 EASU + RCAS`；RCAS stops 基线由 tier ladder 定，运行期可由 policy 动态调节（拥塞等）
   - display degrade 同时影响标准 `webgl2` USM/CAS 与 SR 路径的 RCAS stops
   - diagnostics 明确 `renderPipelineKind = webgl2_sr`

## Validation

- [x] `pnpm vitest run src/streaming/runtime/browser-render-policy.test.ts`
- [x] `pnpm vitest run src/streaming/runtime/browser-runtime.test.ts src/streaming/diagnostics.test.ts`
- [x] `pnpm vitest run src/player/infra/render/Renderers.test.ts`
- [x] `pnpm lint:fix`
- [x] `cargo test -p xbox-streaming policy::`
- [x] `cargo check -p xbxrc`
- [ ] 手工 trace 验证：SR 开启、WebGL2 不可用、SR attach 失败、visibility budget 四条路径 diagnostics 一致

## Risks

- 浏览器 runtime 当前承担连接、恢复、显示、诊断多种职责，直接搬迁容易引入状态更新时序回归。
- `RendererRuntimeConfig` 被多处复用，收窄 attach 合同需要保留兼容层逐步迁移。
- Rust projection 扩展会影响 Tauri DTO、前端 RPC 类型与测试快照，字段命名需要一次定准。
- SR RFC 仍处于进行中，RCAS 固定化需要和现有实验行为对齐。

## Progress

- [x] Step 1: 建立浏览器 render policy 类型与单测
- [x] Step 2: 迁移 `browser-runtime.ts` renderer 参数展开
- [x] Step 3: 收拢浏览器 SR runtime state
- [x] Step 4: 收窄 player renderer attach 合同
- [x] Step 5: 扩展 Rust render projection 与 Tauri / RPC 类型
- [x] Step 6: 修正 SR 固定 RCAS 合同并补 diagnostics

## Execution Notes

- Date: 2026-05-15 | Status: planned
- Update: RFC 创建。确认采用 Rust `Config -> Context -> Plan -> Projection` 思路，浏览器侧新增本地二段 render policy compiler。
- Decision: Rust policy 负责启动期公共策略，浏览器 policy 负责浏览器能力、实际视频尺寸、SR fallback、display degrade 等运行期事实。
- Risk/Blocker: 当前 `browser-runtime.ts` 状态集中，实施时需要先用纯函数单测锁住现有行为，再逐步迁移调用点。
