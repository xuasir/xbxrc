# Browser FSR1 Super Resolution Experimental RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 未完成
- Current State: planned
- Owner: Codex
- Last Updated: 2026-05-12

## Background

- 当前浏览器侧只有两条清晰度手段：
  - `video` fallback 路径上的轻量滤镜
  - `webgl2` 路径上的 `USM/CAS` 锐化后处理
- 这两条路径本质都只是在已有视频帧上做后处理锐化，不承担分辨率重建职责。
- 用户现在希望引入一条独立于锐化的 `Super Resolution (Experimental)` 线路，目标是：
  - 浏览器侧完成分辨率升档
  - 算法固定为 `AMD FidelityFX Super Resolution v1`
  - 不做运行期动态分辨率调节
  - 主要基于“配置的目标分辨率”决定超分档位，但要承认云游戏存在“协商 1440、实际下发 1080”这类源分辨率低于配置档位的情况
  - 用户启用后即固定使用 SR，不再受链路健康和 display degrade 策略开关摆动

## Goal

- 在浏览器 `webgl2` 路径新增一条单独的 `SuperResolutionWebGL2Renderer`。
- 将 `FSR1 = EASU + 低强度 RCAS` 作为该 renderer 的唯一算法实现。
- 把“分辨率重建”和“锐化后处理”从语义上彻底拆开：
  - 标准线路继续是 `USM/CAS`
  - SR 线路固定是 `EASU + low RCAS`
- 固定一张“目标分辨率 -> 超分输出分辨率”映射表，不做运行期动态升降档。
- 明确 SR 的启用合同：用户开启后，浏览器侧固定尝试使用 SR；仅在技术性失败时回退到普通 `webgl2 + CAS/USM`。

## Algorithm Source

- 本 RFC 的 SR 算法固定使用 `AMD FidelityFX Super Resolution v1`。
- `EASU` 与 `RCAS` 的算法来源限定为 AMD 官方公开的 FSR1 实现与文档，不引入自定义 “fsr-like” 变体作为首轮主线。
- 浏览器侧要做的是：
  - 将 AMD 官方 FSR1 参考实现移植到当前 `WebGL2` renderer / shader pass
  - 接入本项目已有的 renderer lifecycle、framebuffer 管理、配置字段与失败回退合同
- 浏览器侧不做的事情：
  - 不把现有 `CAS` 分支改名为 `FSR`
  - 不自造一套与官方语义不一致的 `EASU/RCAS` 近似实现作为默认主线

官方来源：

- AMD GPUOpen FSR1 官方说明：
  - <https://gpuopen.com/manuals/fidelityfx_sdk/techniques/super-resolution-spatial/>
- AMD GPUOpen FSR1 官方源码仓库：
  - <https://github.com/GPUOpen-Effects/FidelityFX-FSR>

首轮实现约束：

- shader 代码可以按本仓库现状以内联字符串方式放入 `Renderers.ts`
- 但算法语义、核心采样与锐化步骤必须可追溯到上述 AMD 官方 FSR1 来源
- 若实现中需要对 shader 进行 `WebGL2 / GLSL ES` 适配，必须在代码注释中说明“这是 API / shader 语言移植，不是算法替换”

## Scope

- In scope:
  - [`src/player/domain/media.ts`](/Users/guo.xu/Documents/code/games/xbxrc/src/player/domain/media.ts)
  - [`src/player/domain/config.ts`](/Users/guo.xu/Documents/code/games/xbxrc/src/player/domain/config.ts)
  - [`src/player/infra/render/Renderers.ts`](/Users/guo.xu/Documents/code/games/xbxrc/src/player/infra/render/Renderers.ts)
  - [`src/streaming/runtime/browser-runtime.ts`](/Users/guo.xu/Documents/code/games/xbxrc/src/streaming/runtime/browser-runtime.ts)
  - [`src/streaming/types.ts`](/Users/guo.xu/Documents/code/games/xbxrc/src/streaming/types.ts)
  - 浏览器侧 `webgl2` renderer、runtime renderer 选择逻辑、diagnostics / trace 字段
- Out of scope:
  - Rust owned / native video 路径上的 SR
  - 传输分辨率、码率、BWE、display degrade 的动态联动重设计
  - FSR2/FSR3、时域超分、motion vector / depth / history buffer
  - 在 SR 线路上继续叠加现有 `clarityL*` 的第二层强锐化

## Proposed Contract

### 1. 独立 renderer

- 新增 `SuperResolutionWebGL2Renderer`
- 与现有 `WebGL2VideoRenderer` 并行存在，不把 SR pass 塞进标准 renderer 的 `if/else`
- 标准 renderer 继续负责：
  - `USM`
  - `CAS`
- SR renderer 固定负责：
  - `FSR1 EASU`
  - `low RCAS`

### 2. 启用方式

- 新增手动开关：`Super Resolution (Experimental)`
- 该开关仅表达用户意图，不与现有 `display_options` / `clarityL*` 混用
- 当用户开启该开关时：
  - 浏览器侧固定优先选择 `SuperResolutionWebGL2Renderer`
  - 不再因 `bandwidthState`、`renderCause`、`displayL0/L1/L2` 在运行期关闭 SR
- 唯一允许回退到普通 `webgl2 + CAS/USM` 的情况：
  - `webgl2` 不可用
  - SR shader 编译失败
  - framebuffer / texture 初始化失败
  - context lost 且无法恢复

### 3. SR 算法

- 算法固定为 `AMD FidelityFX Super Resolution v1`
- pass 固定为两段：
  1. `EASU`
  2. `low RCAS`
- SR 线路不再叠加现有 `USM/CAS` 标准锐化，避免 `EASU -> RCAS -> CAS/USM` 过锐

### 4. 分辨率合同

- SR 不根据运行期链路健康、display degrade 或瞬时 stats 动态升降档
- SR 的目标档位优先由“配置的目标分辨率”决定
- 但 SR 必须额外感知“实际收到的视频源分辨率档位”，避免出现：
  - configured target = `1440`
  - actual source = `1080`
  - 却仍强推 `2160` 输出
- 因此首轮合同改为：
  - `configured target` 决定“期望升档档位”
  - `actual source tier` 决定“允许升档上限”
  - `effective SR output target = nextTier(min(configured target tier, actual source tier))`
- 一旦 SR 成功初始化，对应会话固定走 SR renderer；但 `effective SR output target` 允许在“首次拿到稳定实际源分辨率”后确定一次，不做会话中的持续动态摆动

## Fixed Resolution Ladder

本 RFC 将“按目标分辨率升档”固定为“向上一个标准 16:9 档位提升”，但升档起点取 `configured target tier` 与 `actual source tier` 的较低者。

| Configured target | Actual source tier | Effective SR output target | Notes |
| --- | --- | --- | --- |
| `720` | `720` | `1080` | `720p -> 1080p` |
| `1080` | `1080` | `1440` | `1080p -> 1440p` |
| `1081` | `1080` | `1440` | `Auto (1080p HQ)` 视同 `1080p` |
| `1440` | `1440` | `2160` | `1440p -> 2160p` |
| `1440` | `1080` | `1440` | 云游戏协商 `1440` 但实际只下发 `1080` 时，不再硬推 `2160` |
| `1080` | `720` | `1080` | 配置高于实际源时，仍只升到下一档 |

补充约束：

- `1081` 只是传输侧的 `1080p HQ` 语义；在浏览器 SR 合同里与 `1080` 共用同一超分目标。
- `actual source tier` 由浏览器实际收到的稳定视频尺寸推导，例如 `video.videoWidth/video.videoHeight` 或运行期 `resolution` snapshot。
- `xhome_resolution` 当前只有 `720 / 1080 / 1081`，因此主机串流的 SR 输出只会命中：
  - `720 -> 1080`
  - `1080/1081 -> 1440`
- 若未来出现新的 transport target 档位，默认按“下一标准 16:9 档位”补齐，不在本 RFC 首轮内实现更复杂映射。
- 首轮只允许在“会话早期识别到实际源分辨率后”确定一次 `effective SR output target`，不做中途频繁改档。

## Renderer Design

### Standard Path

`video frame -> WebGL2VideoRenderer -> USM/CAS -> output canvas`

### Super Resolution Path

`video frame -> SuperResolutionWebGL2Renderer -> EASU pass -> RCAS pass -> output canvas`

### Pass Layout

1. Source texture
   - 输入视频帧纹理
2. EASU pass
   - 以 fixed SR output target 渲染到 offscreen framebuffer
3. RCAS pass
   - 读取 EASU 输出，做低强度锐化
4. Present
   - 输出到最终 canvas

### RCAS Strength

- RCAS 强度固定为低档，不复用当前标准 `CAS` 的高质量锐化强度
- 首轮实现不开放额外用户滑杆
- 如需调参，仅允许内部常量或很窄的 profile 切换

## Runtime Selection Rules

### Renderer Selection

建议新增显式 runtime 选择函数：

1. `superResolutionEnabled === true`
2. `pipelineType` 可落到 `webgl2`
3. 若 SR renderer 初始化成功：
   - 固定使用 `SuperResolutionWebGL2Renderer`
4. 若初始化失败：
   - 回退 `WebGL2VideoRenderer`
   - 继续使用标准 `USM/CAS`

### What Changes Compared To Current Plan

当前已确定与先前探索版不同的地方：

- 不再根据 `bandwidthState / displayL* / renderCause` 动态开关 SR
- 不再根据运行期 incoming frame size 动态决定 upscale scale
- 不再将 `displayL0` 或高码率当作 SR 生效前提
- SR 是否启用由用户开关决定，SR 输出档位由 target resolution 决定

## Config And Diagnostics Contract

### Renderer Config

建议新增字段：

- `superResolutionEnabled?: boolean`
- `superResolutionAlgorithm?: 'fsr1'`
- `superResolutionOutputTier?: '1080p' | '1440p' | '2160p'`
- `superResolutionFallbackProcessing?: 'usm' | 'cas'`

其中：

- `superResolutionEnabled` 表示用户意图
- `superResolutionAlgorithm` 首轮固定为 `'fsr1'`
- `superResolutionOutputTier` 由 configured target 推导，不由用户单独选择
- `superResolutionFallbackProcessing` 表示技术性失败时的普通 `webgl2` 回退算法

### Diagnostics

建议在 snapshot / diagnostics 中新增：

- `renderSuperResolutionEnabled`
- `renderSuperResolutionActive`
- `renderSuperResolutionAlgorithm`
- `renderSuperResolutionConfiguredTarget`
- `renderSuperResolutionOutputTarget`
- `renderSuperResolutionFallbackReason`
- `renderSharpenMode`

要求能直接回答：

- 用户是否开启 SR
- 当前是否真的跑在 SR renderer 上
- 当前会话按哪个 configured target 推导了哪个 SR output target
- 如果没跑 SR，是技术性失败还是用户没开

## Risks

- `1440 -> 2160` 会显著增加浏览器侧 `webgl2` 输出成本，首轮需要重点观察 GPU 占用与帧时间。
- 固定 SR 输出档位意味着弱链路或脏源画面不会自动减轻输出成本，实验功能必须保持默认关闭。
- `xhome 720 -> 1080` 与 `1080 -> 1440` 的体感收益可能强于 `1440 -> 2160`，首轮需要拆分验证，不能假设三个档位收益一致。
- 一旦启用后不再动态关闭 SR，技术回退路径必须足够软，不允许打断播放。

## Plan

1. 在 renderer config 和 diagnostics 中引入独立的 SR 合同字段。
2. 在 `Renderers.ts` 新增 `SuperResolutionWebGL2Renderer`，先搭建 `EASU -> RCAS` 双 pass 骨架。
3. 在 `browser-runtime` 中新增基于用户开关、configured target 与 actual source tier 的 renderer 选择逻辑。
4. 固化 configured target + actual source tier 到 SR output target 的映射表，不引入运行期动态 scale。
5. 补齐测试、trace 和文档，验证 `720 -> 1080`、`1080/1081 -> 1440`、`1440 -> 2160` 以及 `configured 1440 / actual 1080 -> output 1440`。

## Validation

- [ ] `Renderers.test.ts` 覆盖 `SuperResolutionWebGL2Renderer` 的 attach / update / destroy 生命周期
- [ ] `browser-runtime` 覆盖 renderer 选择逻辑：SR enabled、SR init fail、fallback to standard webgl2
- [ ] diagnostics / snapshot 覆盖 configured target + actual source tier 到 SR output target 的映射
- [ ] 验证 `xhome_resolution=1081` 与 `1080` 命中相同 `1440p` 输出档
- [ ] 验证云游戏 `configured=1440` 但 `actual source=1080` 时，输出目标被钳制为 `1440p`
- [ ] 对 `1440 -> 2160` 进行首轮性能观察，确认实验档不会导致明显不可接受的浏览器侧掉帧

## Progress

- [ ] Step 1: 定义 SR 合同字段
- [ ] Step 2: 落地 `SuperResolutionWebGL2Renderer`
- [ ] Step 3: 接入 fixed resolution ladder、actual source tier 钳制与 runtime 选择
- [ ] Step 4: 补齐 diagnostics / tests

## Execution Notes

- Date: 2026-05-12 | Status: planned
- Update: 确认首轮浏览器侧 SR 走独立 `SuperResolutionWebGL2Renderer`，算法固定为 `FSR1 EASU + low RCAS`。
- Decision: SR 启用后固定使用，不再按链路健康和 display degrade 动态开关；分辨率映射以 target resolution 为主，但要受 actual source tier 钳制，避免 `configured 1440 / actual 1080` 被误推到 `2160`。
- Risk/Blocker: `1440 -> 2160` 的浏览器侧性能成本尚未实测；同时需要确认“会话早期确定 actual source tier 一次”的稳定口径。
