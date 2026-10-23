# CAS 锐化颗粒控制 RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 未完成（代码已落地，待真机观感 A/B）
- Current State: in-progress
- Owner: render / sharpening
- Last Updated: 2026-10-23

## Background

- 触发：用户反馈 CAS 观感有「颗粒感 / 油画感颗粒感」，尤其在暗部平坦区和低码率 1080p 流上。
- 数学根因（已核对 [Renderers.ts:365-376](../../src/player/infra/render/Renderers.ts#L365-L376)）：
  - 现有 CAS 段可化简为 `out = center + (center - mean4) * G`，其中 `G = 4√a / (5.2 - 4√a)`，`a = clamp(min(min4, 2-max4)/max4, 0, 1)`。
  - 平坦区 `a→1` 时 `G→3.33`：**噪声被放大 3.3 倍**；强边缘 `a→0` 时 `G→0`：**真实轮廓几乎不锐化**。
  - 噪声、压缩振铃、暗部 dither、8×8 DCT 块边界恰好全在「低对比区」，于是锐化器变成了「噪声增强器 + 边缘保持器」，这正是颗粒 + 油画感的组合。
- 实现侧放大因素：
  1. `mix(center, outColor, sharpenFactor / 2.0)` 未钳制；`resolveShaderPreset` 的 `clarityL3` 返回 `max(1.5, strength * 1.2)`，`strength = sharpenStrength / 25`，导致 `清晰` 预设（sharpness=3 → 75 → 3）实际 `t = 1.5`，即 **1.5× 外推、等效 5× 高通**。
  2. webgl2 CAS 没有邻域 ringing 钳制；SR 路径的 RCAS 有（[SuperResolutionWebGL2Renderer.ts:283-291](../../src/player/infra/render/SuperResolutionWebGL2Renderer.ts#L283-L291)），两条路径观感分裂。
  3. SR 路径 RCAS 的 `nz` 噪声项算完未参与合成（[SuperResolutionWebGL2Renderer.ts:258-260](../../src/player/infra/render/SuperResolutionWebGL2Renderer.ts#L258-L260)），等于少一层噪声抑制。
  4. 逐通道独立求 `min/max`，4:2:0 色度上采样后三通道增益不一致 → 边缘彩色斑点。
  5. `texel = 1/sourceResolution` 但 `uv` 取 `outputResolution`，输出尺寸 ≠ 源尺寸时采样落在非整数源像素上，核不对称、`min/max` 窗口被双线性抹平。

## Goal

- 在**边缘清晰度基本不变**的前提下，把平坦区高频增益从 3.33× 压到 ≤ 2.0×（可配置），并把低于噪声底的残差压到 ≤ 1.2×。
- 过冲（overshoot）限制在邻域 range 的 1.38× 以内，消除密集纹理区的交错亮点/暗点。
- webgl2 CAS 与 SR RCAS 共用同一套锐化参数常量，两条路径观感一致。
- 强度语义可解释：`strength ∈ [0,1]` 线性映射，不再出现隐式外推。

## Scope

- In scope:
  - `src/player/infra/render/Renderers.ts`：CAS 段（`filterId == 2`）重写为「显式 gain + 噪声门限 + 邻域钳制」。
  - `src/player/infra/render/SuperResolutionWebGL2Renderer.ts`：RCAS 段接入同一套常量与 `nz` 门限。
  - `src/player/infra/render/sharpening-tuning.ts`（新增）：唯一参数源，GLSL 模板插值 + TS 参考实现共用。
  - `src/player/infra/render/Renderers.ts` 的 `resolveShaderPreset`：强度映射收敛到 0..1。
  - 单测：`Renderers.test.ts` uniform 断言更新；新增 `sharpening-math.test.ts`。
- Out of scope:
  - FSR1 EASU 本身、SR 档位决策（见 `2026-10-23-super-resolution-tier-redesign`）。
  - 原生 video + SVG `feConvolveMatrix` 的 USM 路径。
  - 饱和度 / 对比度 / 亮度的语义变更。
  - 码率自适应降锐（`adaptive.sharpnessScale`）策略本身。

## Plan

### P0 最小改动（直接解决颗粒与过冲）

1. 新增 `sharpening-tuning.ts`，导出：
   ```ts
   export const CAS_GAIN_MAX = 2.0        // 平坦区高频增益上限（现状 3.33）
   export const CAS_NOISE_FLOOR = 1.5 / 255  // 噪声底，低于此的 luma 残差视为噪声
   export const CAS_GATE_RAMP = 2.0       // 软门限斜坡倍数
   export const CAS_RING_PAD = 0.38       // 与 SR 路径对齐
   export const CAS_RING_BLEND = 0.92     // 与 SR 路径对齐
   export const CAS_CHROMA_BLEND = 0.85   // 1.0 = 纯 luma 锐化
   ```
2. 重写 CAS 段为等价但可解释的形式：
   ```glsl
   float a = clamp(min(min4, 2.0 - max4) / max4, 0.0, 1.0);
   float rootA = sqrt(a);
   float g = min(4.0 * rootA / max(5.2 - 4.0 * rootA, 1e-3), CAS_GAIN_MAX);
   vec3 hp = center - mean4;
   float gate = smoothstep(CAS_NOISE_FLOOR, CAS_NOISE_FLOOR * CAS_GATE_RAMP, abs(dot(hp, LUMINOSITY_FACTOR)));
   hp = mix(hp, vec3(dot(hp, LUMINOSITY_FACTOR)), CAS_CHROMA_BLEND);
   vec3 outColor = center + hp * g * gate * clamp(sharpenFactor, 0.0, 1.0);
   // 邻域钳制
   vec3 lo = min(min4, center), hi = max(max4, center);
   vec3 pad = (hi - lo) * CAS_RING_PAD + 1e-4;
   outColor = mix(outColor, clamp(outColor, lo - pad, hi + pad), CAS_RING_BLEND);
   outColor = clamp(outColor, 0.0, 1.0);
   ```
3. `resolveShaderPreset` 强度收敛：
   - `strength = sharpenStrength !== undefined ? sharpenStrength / 100 : sharpness / 10`，再 `clamp(0, 1)`。
   - `clarityL1/L2/L3` 的 `Math.max(...)` 魔法系数改为 `strength * {0.6, 0.8, 1.0}`。
   - 删除 `sharpenFactor / 2.0`。

### P1 噪声抑制与路径统一

4. SR RCAS 的 `nz` 接入：`lobe *= nz`（或 `pix = mix(e, pix, nz)`），并把 `CAS_RING_*` 常量替换为共享常量。
5. 两条路径统一使用 `LUMINOSITY_FACTOR` 门限，避免 CAS 路径继续逐通道独立。

### P2 尺度正确性（可选，独立提交）

6. 锐化 pass 固定渲染到源尺寸 FBO，再 blit 到输出，保证 `texel` 与采样 1:1、核对称。
7. `diagnostics` 增加 `renderSharpening: { mode, strength, gainMax, gate, ringClamp }`，便于 trace 复盘。

## Validation

- [ ] `pnpm exec vitest run src/player/infra/render` 全绿。
- [ ] 新增 `sharpening-math.test.ts`（TS 参考实现，导入同一常量）：
  - 平坦区 ±1/255 噪声输入，输出偏差 ≤ 1.2/255。
  - 单像素阶跃边缘，过冲 ≤ 邻域 range × 1.38。
  - `strength = 0` 时输出与输入逐像素相等（恒等）。
  - 断言生成的 GLSL 字符串包含 `CAS_GAIN_MAX.toFixed(4)`，防止常量漂移。
- [ ] `pnpm lint:fix`、`pnpm exec vue-tsc --noEmit`、`pnpm build` 通过。
- [ ] 截图 A/B：同一帧（暗部场景 + 密集纹理场景）在 `strength = 0.3 / 0.6 / 1.0` 三档对比，暗部噪声方差与边缘 MTF 各记一组数。
- [ ] 真机：低码率 1080p、暗部场景、窗口 / 全屏两种输出尺寸各一次。

## Risks

- 噪声门限过高会「抹细节」，需要默认保守 + 可调；`smoothstep` 软门限避免硬切造成的细节闪断。
- luma-only（`CAS_CHROMA_BLEND = 1.0`）会让彩色细节变软，默认取 0.85 折中。
- `CAS_GAIN_MAX = 2.0` 可能让部分用户觉得「不如以前清晰」，需要在 preset 层提供更高档位而不是改默认。
- 两条路径同时改，回归面变大 → P0 先只改 webgl2 CAS 并合入，P1 再动 SR。
- 单测用 TS 参考实现校验 GLSL 有漂移风险，用「常量插值 + 断言」双保险兜住。

## Progress

- [x] Step 1 (P0-1): 新增 `sharpening-tuning.ts`（`CAS_GAIN_MAX=2.0`、噪声底 1.5/255、ring pad/blend 与 SR 对齐）
- [x] Step 2 (P0-2): 重写 webgl2 CAS 段为「显式 gain + luma 门限 + 邻域钳制」，USM 分支同步钳制强度
- [x] Step 3 (P0-3): 收敛 `resolveShaderPreset` 强度映射到 0..1，删除 `/2` 外推与 `Math.max` 魔法系数
- [x] Step 4 (P1): SR RCAS 接入 `nz`（含除零下限）与共享 ring 常量
- [x] Step 5 (P2): 锐化 pass 固定源尺寸 FBO + 双 pass 呈现；diagnostics 增 `renderSharpenStrength`
- [ ] Step 6: 真机观感 A/B（暗部 / 密集纹理 / 低码率 1080p / 窗口与全屏）

## Execution Notes

- Date: 2026-10-23 | Status: in-progress（代码完成，待真机观感验证）
- Update:
  - 新增 `sharpening-tuning.ts`（唯一参数源）、`sharpening-math.ts`（TS 参考实现）、`sharpening-math.test.ts`（8 项）。
  - `Renderers.ts`：CAS 段改为 `out = center + residual * gain * gate * strength`，`gain = min(4√a/(5.2-4√a), 2.0)`；
    luma 软门限 `smoothstep(1.5/255, 3/255)`；`CAS_CHROMA_BLEND=0.85` 收敛色度；ring clamp `pad 0.38 / blend 0.92`。
  - `resolveShaderPreset`：`strength = sharpenStrength/100` 或 `sharpness/5`，钳到 0..1；`清晰` 预设从 1.5× 外推降到 0.8。
  - 双 pass：输出 ≠ 源时先在源尺寸 FBO 做 1:1 锐化，再升采样 + 色彩；FBO 不完整自动回落单 pass。
  - `SuperResolutionWebGL2Renderer`：`lobe *= nz` 恢复噪声抑制，`nzRange` 加 `1e-4` 下限防除零。
- Validation:
  - `vitest run` 163 项全绿（新增 8 项锐化数学 + 1 项双 pass 回归）。
  - 合成噪声量化：±2/255 平坦噪声下，旧公式噪声 std 放大 **2.93×**，新公式 **1.00×**。
  - `eslint src --fix`、`vite build` 通过；`vue-tsc --noEmit` 仅剩 4 条仓库既有错误（gamepad test ×2、updater event ×1、SR test `rafCallbacks` ×1）。
- Decision: 保持「锐化在色彩之前」的顺序不变，避免引入色彩语义变更；`qualityMode` 对 CAS 不再生效（原 5.2/5.6 双档被显式上限取代）。
- Risk/Blocker: 真机 A/B 未做；`CAS_GAIN_MAX=2.0` 下「清晰」预设是否足够锐需要肉眼确认，不足则抬 preset 系数而不是抬默认上限。
