# 超分档位重设计：只升不降，按「协商档位升一档 / 真实源 / 显示预算」决策 RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后单独产出 Report。

## Status

- Completion: 未完成
- Current State: planned
- Owner: render / streaming runtime
- Last Updated: 2026-10-23

## Background

### 触发

`2026-10-22-fsr1-downscale-skips-rcas`（已修）暴露的是表象：1440p 源 + 1080p 档位时 FSR1 比 CAS 更糊，原因是 `noUpscale` 分支只做双线性缩小、RCAS 被跳过。补跑 RCAS 只是把「降采样」做得好一点，**没有回答为什么超分会去降采样**。

超分的语义是「分辨率重建」：它只能在 `输出 > 源` 时提供信息增益。当前设计却允许、甚至常态性地产生 `输出 < 源`。

### 现状事实（代码考古结论）

**档位函数** `src/streaming/runtime/super-resolution-ladder.ts:119-145`：

```
base       = min(configuredTier, actualSourceTier)       // :135
preferred  = {720→1080, 1080→1440, 1440→1440, 2160→2160} // :80-91
output     = min(preferred, configuredTier)              // :93-98 clamp
```

`configured` 来自 `targetVideoWidth/Height`，即用户分辨率偏好经设备画像推导出的**请求值**（`crates/xbox-streaming/src/policy/session/compiler.rs:158-216`，`projection.rs:183-184`），同时写进 `negotiation.target_resolution_width`（`crates/xbxengine/core/src/api/runtime/lifecycle.rs:1129`）；`actualSource` 来自 `video.videoWidth/Height`（`browser-runtime.ts:1696-1716`）。

**当前 (configured, source) → output 全表**（实测语义）：

| configured \ source | 720p | 1080p | 1440p | 2160p |
| --- | --- | --- | --- | --- |
| 720p | 720p 同档 | 720p **降采样** | 720p **降采样** | 720p **降采样** |
| 1080p | 1080p 放大 | 1080p 同档 | 1080p **降采样** | 1080p **降采样** |
| 1440p | 1080p 放大 | 1440p 放大 | 1440p 同档 | 1440p **降采样** |
| 2160p | 1080p 放大 | 1440p 放大 | 1440p 放大 | 2160p 同档 |

16 格中 6 格是降采样，且降采样格正好是「主机给的比我们要的好」这种**不该被惩罚**的情形。同时 `1080p 请求 + 1080p 源` 输出 1080p，等于**超分根本不介入**——用户开了超分却没有任何升档。

**另外还有三条独立的降采样钳制**，任何一条都能把已经算对的档位再压回去：

1. `clampSrOutputToPresentTarget`（`browser-render-policy.ts:310-322`）：SR 输出被等比压到 presentTarget 以内。
2. `resolvePresentTargetResolutionCap`（`browser-render-policy.ts:263-282`）：全屏时 presentTarget 上限 = `min(configured, source)`。
3. `resolveEffectiveSrOutputSize`（`SuperResolutionWebGL2Renderer.ts:329-345`）：renderer 侧再按 `renderOutputWidth/Height` 夹一次。

叠加 display degrade（L1/L2 收紧 `maxOutputPixels` 与 DPR，`browser-render-policy.ts:230-253`）后，「固定档位」实际只是**上限语义，不是像素承诺**：同一档位在不同显示状态下渲染像素数完全不同。

**其它相关缺口**：

- launch 时用 `resolveSuperResolutionTierPlan(tw, th, tw, th)`（`browser-runtime.ts:341`），把 configured 当成 source，canvas 从第一帧就是 configured 尺寸。
- 冻结只在**首个** `media.videoReady` 发生且永不重算（`browser-runtime.ts:1596-1635`），而 05-12 RFC 写的是「拿到**稳定**源分辨率后冻结」，代码没有稳定性窗口。
- `2160p` 档在旧表里不可达（`preferred` 表里 1440→1440），属于死档。

### 为什么这是设计问题

- 「configured」被同时当成**内容供给上限**、**渲染目标**和**GPU 预算**，三种语义混在一个变量里。
- 档位只回答「输出多大」，不回答「这一步在做放大还是缩小」。调用方无法区分「真超分」与「降采样兜底」，诊断、trace、回退判定都失去依据。
- 05-18 的收紧本意是避免「configured 1440 / actual 1080 硬推 2160」，但把「升一档」改成了「补到 configured」，顺带产生反方向的洞：源比请求更好时反而被压。05-12 RFC 至今仍是 planned，`docs/reports/` 无对应 Report，口径只能靠代码反推。

## Goal

1. 建立**只升不降**的超分不变量：SR 输出永不小于真实源；缩小只在「显示预算本身小于源」时发生，并被明确标记为 present 降采样，而不是 SR。
2. 档位由 `(协商要求 R, 真实源 S, 显示预算 D)` 三元组显式决策：**在 `min(R, S)` 之上升一档**，输出带 `mode` 的计划而不是裸尺寸。
3. 把三条降采样钳制改成「不得低于源」的守卫，消除「算对了又被压回去」。
4. 让 trace / 诊断能直接回答：这次 SR 在放大还是在缩小、被谁钳制、为什么选这个档。
5. 修正 `2160p` 死档与「configured 当 source」的临时档位。

## Scope

- In scope:
  - `src/streaming/runtime/super-resolution-ladder.ts`（决策函数重写）
  - `src/streaming/runtime/browser-super-resolution-state.ts`（冻结状态与重算条件）
  - `src/streaming/runtime/browser-render-policy.ts`（`clampSrOutputToPresentTarget` / `resolvePresentTargetResolutionCap` 的 SR 分支）
  - `src/streaming/runtime/browser-runtime.ts`（displayContext 组装、冻结时机、trace）
  - `src/player/infra/render/SuperResolutionWebGL2Renderer.ts`（`resolveEffectiveSrOutputSize` 下限守卫、`noUpscale` 分支语义）
  - 对应测试：`super-resolution-ladder.test.ts`、`browser-render-policy.test.ts`、`browser-runtime.test.ts`、`Renderers.test.ts`
- Out of scope:
  - 协商层反哺（把「主机实际下发 > 我们请求」反馈给 `session.device.max_*` / SDP bitrate profile）。
  - 精确像素输出（放弃 4 档量化，直接按显示像素渲染）。
  - CAS / USM / linear 回退链本身。
  - Rust 侧设备画像与 resolution preference 枚举。
  - display degrade 档位定义。

## Design

### 1. 三个输入，三种语义

| 符号 | 含义 | 来源 | 语义 |
| --- | --- | --- | --- |
| `R` | 协商要求 | `targetVideoWidth/Height` | 我们向主机**要**的分辨率 |
| `S` | 真实源 | `video.videoWidth/Height` | 主机**实际给**的分辨率，信息量上限 |
| `D` | 显示预算 | 容器 CSS × DPR，再受 display degrade 上限约束 | 屏幕上**需要**多少像素，GPU 预算上限 |

`R` 与 `S` 共同决定「升档起点」，`D` 只决定「升到多高是浪费」。三者职责单一，不再互相兼任。

### 2. 决策函数

```
R_t = tier(R), S_t = tier(S), D_t = tier(D)

base   = min(R_t, S_t)          // 升档起点：两者较低者
target = nextTier(base)         // 720→1080, 1080→1440, 1440→2160, 2160→2160
output = min(D_t, max(S_t, target))

mode = output >  S_t ? 'upscale'
     : output == S_t ? 'passthrough'
     :                 'present-downscale'
```

为什么起点取 `min(R_t, S_t)` 而不是 `R_t`：

- **请求 1080 可能返回 1440**。若起点取 `R`，`1080` 请求会被推成 `1440` 目标，而源已经是 `1440`——白算一遍；若起点取 `max`，源比请求好时会被当成「需要降档」。取 `min` 时：源更好 → 起点仍是请求档，升一档正好落在源上或源以下 → `passthrough`，不降采样。
- **升档幅度恒为一档**。`min` 保证 `target ≤ nextTier(R_t)`，因此不会出现「请求 1440、源只有 1080，却硬推 2160」的旧问题。
- `max(S_t, target)` 是**只升不降**的兜底：源已经超过目标时直接输出源，不缩。

三个分支的语义：

- `upscale`：真超分，EASU + RCAS。
- `passthrough`：源已经够（`S ≥ target`），EASU 无意义，走 1:1 RCAS-only；**绝不做 bilinear 缩小**。
- `present-downscale`：显示预算本身小于源（`D < S`），此时缩小是正确的显示行为，继续走「降采样进中间 FBO → RCAS」（`32aadc75` 的实现），但 trace 必须标记为显示预算导致，而不是 SR 档位导致。

### 3. 不变量

| ID | 不变量 | 说明 |
| --- | --- | --- |
| I1 | `output ≥ S`，除非 `D < S` | 超分永不降低分辨率 |
| I2 | `output ≤ D` | 不做无显示收益的过度渲染 |
| I3 | `mode='upscale' ⟺ output > S` | 模式由像素关系唯一决定，不是标签 |
| I4 | 任何钳制都不得把 `output` 压到 `S` 以下（`present-downscale` 除外） | 消除「算对又被压回」 |
| I5 | `output ≤ nextTier(max(R_t, S_t))` | 升档幅度不超过一档，不会跨档跳跃 |
| I6 | 同一 `(R_t, S_t, D_t)` 内输出恒定 | 保留「会话内不动态抖动」的历史合同 |

### 4. 代表场景

先看不设显示预算（`D = 2160p`）的纯档位行为：

| R \ S | 720p | 1080p | 1440p | 2160p |
| --- | --- | --- | --- | --- |
| **720p** | 1080p ↑ | 1080p = | 1440p = | 2160p = |
| **1080p** | 1080p ↑ | **1440p ↑** | **1440p =** | 2160p = |
| **1440p** | 1080p ↑ | 1440p ↑ | 2160p ↑ | 2160p = |
| **2160p** | 1080p ↑ | 1440p ↑ | 2160p ↑ | 2160p = |

（↑ 放大 / = 同档 passthrough；**加粗**两格就是本次要修的语义：`1080/1080` 从「不升档」变为真超分，`1080/1440` 从「降采样到 1080」变为 passthrough 1440。）

显示预算的作用（`R=1080p, S=1080p`）：

| D | output | mode | 说明 |
| --- | --- | --- | --- |
| 1080p | 1080p | passthrough | 窗口/屏只有 1080p，不做无收益超分 |
| 1440p | 1440p | upscale | 典型场景：1080p 流补到 1440p 屏 |
| 2160p | 1440p | upscale | 4K 屏也最多升一档，不跳档 |
| 720p | 720p | present-downscale | 显示预算小于源，合法缩小 |

### 5. 钳制链重构

| 位置 | 现状 | 新语义 |
| --- | --- | --- |
| `resolveSuperResolutionTierPlan` | `min(preferred, configured)` | 决策函数（§2），返回 `{mode, baseTier, outputTier, outputWidth, outputHeight, reason}` |
| `clampSrOutputToPresentTarget` | 无条件等比压到 presentTarget | 仅在 `D < S` 时允许压到 `D`；否则下限锁在 `max(S, output)` |
| `resolvePresentTargetResolutionCap` | 全屏 `min(configured, source)` | SR 路径改为只看 `D`（display degrade 上限）；非 SR 路径保持原样（控制 blast radius） |
| `resolveEffectiveSrOutputSize` | tier 夹到 budget | 夹到 budget 但下限 `S`；若 budget < S，说明是 `present-downscale`，允许 |
| launch 临时档位 | `plan(tw, th, tw, tw)` | 未冻结时不下发 SR 输出尺寸，attach 后等首个 `videoReady` 冻结再建 canvas |

### 6. 冻结与重算

- 冻结时机：首个 `media.videoReady` + 一次 `T+1000ms` 复核（`video.videoWidth` 与冻结时不一致则重算一次）。解决「metadata 首帧尺寸异常锁错档」。
- 允许重算的触发：`R_t` / `S_t` / `D_t` 任一跨档变化，且距上次重算 ≥ 1s，且当前不在 recovery episode。会话内上限 2 次，避免 canvas resize 抖动。
- 容器尚未完成布局（宽 < 320 或高 < 180）时不把该尺寸当显示预算，避免把档位误判成 `present-downscale`。
- 冻结结果写 trace `superResolutionTierFrozen`，字段扩展为：
  `{ requestedTier, sourceTier, displayTier, baseTier, outputTier, mode, reason, clampedBy, rcasStopsBase }`。

### 7. RCAS stops

- 新模型下 `upscale` 只可能命中三对档位（`720→1080`、`1080→1440`、`1440→2160`），因此**沿用既有逐对调优值**（0.72 / 0.88 / 1.02），不做比例化重算，避免顺带改变已调好的观感。
- `passthrough` 用 `0.88`；`present-downscale` 用 `0.72`（缩小场景锐化更保守，避免 ringing）。
- 动态调节（display 档位 / 拥塞 / 码率）保持现状，不改档位，只改 stops。

### 8. 诊断

- `superResolutionTierFrozen` 增加 `mode` / `clampedBy` / `baseTier`。
- 新增一次性事件 `superResolutionSourceAboveRequested`：`S_t > R_t` 时记录，附 `requestedTier/sourceTier`，供后续判断「协商层是否该改请求」（本次不动作）。
- 运行时快照 `renderSuperResolution*` 增加 `mode` 涉及跨层快照类型，本轮先只在 trace 落 `mode`，快照字段留到后续需要时再加。

## Plan

1. **Step 1**：重写 `super-resolution-ladder.ts` 为三元组决策函数 + `mode` + `reason`，补全 4×4×4 矩阵单测（不变量 I1–I5）。
2. **Step 2**：改造 `browser-render-policy.ts` 的两处钳制与 `SuperResolutionWebGL2Renderer.resolveEffectiveSrOutputSize` 下限守卫（I4），补 policy / renderer 回归。
3. **Step 3**：`browser-runtime.ts` displayContext 组装、冻结时机与重算、launch 临时档位、trace 字段。
4. **Step 4**：`Renderers.test.ts` 对齐 `present-downscale` 语义（保留 `32aadc75` 的降采样+RCAS 实现，只改触发条件与命名）。
5. **Step 5**：`pnpm lint:fix`、`pnpm exec vitest run` 相关套件、`pnpm build`；真机验证三个场景。

## Validation

- [ ] ladder 矩阵单测：`R×S×D = 4×4×4`，断言 I1–I5 全部成立
- [ ] 回归：`1080p 请求 + 1440p 源` → `output=1440p / mode=passthrough`（当前为 1080p 降采样）
- [ ] 回归：`1080p 请求 + 1080p 源 + 1440p 显示` → `output=1440p / mode=upscale`（当前为不升档）
- [ ] 回归：`2160p 请求 + 1440p 源 + 2160p 显示` → `output=2160p / mode=upscale`（当前死档）
- [ ] policy 测试：全屏 + DPR2 + SR，presentTarget 不得把 SR 输出压到源以下
- [ ] renderer 测试：`output ≥ source` 时不进入降采样分支；`D < S` 时进入并跑 RCAS
- [ ] `pnpm exec vitest run src/streaming/runtime src/player/infra/render` 全绿
- [ ] `pnpm lint:fix`、`pnpm build` 通过
- [ ] 真机：1440p 源 + 1080p 请求（原 bug 场景）、1080p 源 + 1440p 请求（真超分）、4K 屏 + 1080p 流
- [ ] trace 可复盘：`superResolutionTierFrozen` 含 `mode` 与 `clampedBy`

## Risks

- **GPU 成本上升**：`1080/1080` 从「不升档」变为渲染 1440p，`1440/1440` 在 4K 屏上变为渲染 2160p。这是本设计的目的，但弱机可能掉帧。缓解：`D` 上限自动吸收（degrade 到 L1/L2 时 D 变小，SR 自动退回 passthrough），且 SR 仍默认关闭。
- **与 display degrade 的交互**：L1/L2 收紧像素预算后 SR 输出随之变小，可能重新落回 `present-downscale`。需要在 trace 里区分「degrade 导致」与「档位导致」。
- **canvas resize 抖动**：允许重算带来 canvas 尺寸变化，可能引发一次掉帧。缓解：重算次数上限 + recovery 期间禁止。
- **协商层未对齐**：`S_t > R_t` 说明主机无视请求，而 SDP bitrate profile 与 `maxFrameSize` 仍按 `R` 计算（`src/player/app/session/SessionService.ts:426-440`），实际码率预算偏小。本次只记录诊断，不修协商（用户已明确不动协商层）。
- **历史合同冲突**：05-12 RFC 的「升一档」表被 05-18 改成「补到 configured」，本次回到「升一档 + 只升不降」；需同步把 05-12 / 05-18 的档位口径标记为 superseded，避免第四次漂移。

## Open Questions

1. ~~SR 的目标上限~~ → **2026-10-23 修订**：改为 `target = min(configured, display)`，`D` 不再只是上限而是目标。原「恒升一档」在真机上暴露为「720p 源 + 1440p 屏只出 1080p」，用户口径要求补到 1440p。
2. ~~是否反哺协商层~~ → 已定：本次不做，只在 trace 记录 `sourceAboveRequested`。
3. `2160p` 档：新规则下它可达（`1440p 源 + 4K 显示`），保留。

## Progress

- [x] Step 1: `super-resolution-ladder.ts` 重写为三元组决策 + `mode` / `reason`，`super-resolution-ladder.test.ts` 覆盖 11 组（含 4×4 不变量矩阵）
- [x] Step 2: `browser-render-policy.ts` 新增 `resolveSuperResolutionDisplayBudget`，`clampSrOutputToPresentTarget` 加源下限，SR 路径不再受 `min(configured, source)` 全屏上限；`resolveEffectiveSrOutputSize` 加源下限守卫
- [x] Step 3: `browser-runtime.ts` 冻结接显示预算、跨档重算（1s 驻留 / 上限 2 次 / recovery 内不重算）、1s 复核定时器、trace 增 `baseTier/displayTier/mode/reason/clampedBy` 与 `superResolutionTierMismatch`
- [x] Step 4: `Renderers.test.ts` 未改断言（`present-downscale` 实现保留，仅注释与语义对齐）
- [ ] Step 5: `pnpm lint:fix` / `pnpm exec vitest run` / `pnpm build` 已过；**真机三场景待补**

## Execution Notes

- Date: 2026-10-23 | Status: in-progress（实现完成，真机验证待补）
- Update: 初稿基于代码考古与 05-12 / 05-15 / 05-18 RFC 追溯；按用户口径把决策函数从「补到协商档位」修正为「`min(R,S)` 起点升一档 + 只升不降 + `D` 上限」；随后完成代码落地。
- Decision: 只改档位设计，不动协商层；`2160p` 档保留（4K 显示下可达）；RCAS stops 沿用逐对调优值而非比例化重算，避免顺带改变观感。
- Risk/Blocker: 真机未验证。`1080/1080` 由「不升档」变为渲染 1440p、`1440/1440` 在 4K 屏变为渲染 2160p，弱机帧率风险需实测；display degrade 与档位的交互只能在真机 trace 里确认。

- Date: 2026-10-23（当日二次修订） | Status: in-progress
- Update: 真机 trace（`runtime-trace-1792733722209-1.jsonl`）显示主机中途从 2560x1440 降到 1280x720，而 `superResolutionTierFrozen` 只发出 1 次（1440p passthrough），降档后没有重算，面板与渲染器长期不一致。据此做两处修正：
  1. `targetTier = min(configured, display)`（仅当它不高于 `base` 时退回 `nextTier`），720p 源 + 1440p 屏直接出 1440p，新增 reason `upscale-to-display-budget` 与 `720p→1440p` 的 RCAS stops `0.78`；
  2. SR 冻结跟随源尺寸变化重算：`PlaybackService` 监听 `video.resize` 重新广播 `media.videoReady`，`browser-runtime` 在 `stats.browserWebRtc` 采样里发现帧尺寸变化时也触发 `freezeSuperResolutionOutputIfNeeded`。
- Decision: 「只升不降」不变；变的是升档幅度上限——由「一档」改为「到配置/显示预算」。
- Risk/Blocker: 720p→1440p 是 2x 放大，EASU 重建会偏软；弱机 GPU 成本上升，需要真机确认帧率与 display degrade 交互。
