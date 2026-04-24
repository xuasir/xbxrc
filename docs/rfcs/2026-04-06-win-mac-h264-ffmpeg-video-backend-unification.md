# Win/mac H.264 FFmpeg Video Backend Unification RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 已完成
- Current State: completed
- Owner: TBD
- Last Updated: 2026-04-09

## Background

- 当前 canonical 视频主线仍固定为 `openh264` 软解 + 最小 headless `wgpu` render backend，现状记录见 [crates/README.md](/Users/guo.xu/Documents/code/games/xbxrc/crates/README.md#L71)。
- `xbxengine` 已具备较完整的恢复控制面、decoder reset / keyframe request 升级路径与 runtime stats 投影，但解码主控仍偏单一路径，软解与平台硬解没有统一到同一个候选链路模型。
- 现有实现中，`crates/xbxengine/core/src/media/video/decode/video_decode.rs` 已包含 macOS `VideoToolbox` 硬解入口，但它与整体解码控制面仍是手写局部实现；Windows 侧尚无等价路径。
- `src-tauri/src/mods/native_video/*` 已承担 present / host feedback / `wgpu` 导入职责，尤其 macOS 已具备 `CVPixelBuffer -> Metal/wgpu` 导入与 CPU upload fallback 路径，这为 FFmpeg 硬解 surface 接入提供了现实基础。
- Moonlight 的最大参考价值不在于“使用 FFmpeg”本身，而在于以下控制面设计：
  - 统一 FFmpeg 解码主控，平台差异收敛到 backend/frontend renderer 适配层
  - 候选链路选择不是单看 API 可创建，而是以 test decode + test render 验活为准
  - 解码、pacing、render、overlay、reset 生命周期显式解耦
  - 连续失败后不是简单局部 reset，而是重跑 decoder/backend 选择并请求关键帧重新收敛
- 本任务希望在 `Win/mac`、仅 `H.264`、仅桌面主线前提下，引入 FFmpeg 统一替换现有软解与平台硬解入口，并保留 `xbxengine` 现有 recovery / stats / render orchestration 主权。

## Goal

- 为 `Win/mac` 建立统一的 FFmpeg H.264 视频 backend 主线，覆盖：
  - 软件解码替换现有 `openh264` 主线
  - 平台硬解统一收口到 FFmpeg hwaccel 模型
- 将“硬解优先于软解”从实现偏好提升为强约束：只要平台硬解 backend probe 成功并且 render bridge 验活成功，就必须优先选用硬解；软解仅作为 probe / render 验活失败后的回落路径。
- 将“硬解 0 拷贝 present/import”从后续优化提升为交付要求：macOS 必须走 `CVPixelBuffer -> Metal/wgpu/native presenter`，Windows 必须补齐 `D3D11 surface -> native_video/wgpu` 的 direct import，不接受长期停留在 `hwframe_transfer_data -> BGRA -> CPU upload` 作为目标方案。
- 将 decoder/backend 选择、验活、运行期错误观测与 reset/recreate 统一纳入 `xbxengine` 视频控制面，而不是继续分散在单个平台局部实现中。
- 保持现有 `wgpu` / native video present 体系、recovery policy、runtime stats 合同尽量不破裂，只替换 decode backend 及其与 render 的桥接方式。
- 明确打包策略为“应用自带 FFmpeg 运行时”，不依赖宿主 FFmpeg 环境。
- 强制约束：`Win/mac` 主线必须默认“硬解优先于软解”，软件解码只允许作为 probe/runtime failure 后的 fallback。
- 强制约束：`Win/mac` 主线都必须落实零拷贝 decode-present/import 路径；CPU upload / copyback 只能作为开发兜底或故障回退，不能作为最终主线交付形态。

## Scope

- In scope:
  - `crates/xbxengine/core/src/media/video/decode/*` 的视频解码 backend 重构
  - 新增 FFmpeg H.264 software decode backend
  - 新增 FFmpeg `VideoToolbox` / `D3D11VA` H.264 hardware decode backend
  - 候选链路、启动前 probe、decoder/backend 失败分类、reset/recreate 机制
  - `DecodedFrame` / `XbxRenderFrame` 与现有 `native_video`/`wgpu` 导入路径的桥接
  - `Win/mac` 两端的硬解 surface descriptor / native handle contract，以及对应的零拷贝 import/present 主线
  - `src-tauri` 打包与运行时库加载策略，限定 `Win/mac`
  - 现有 runtime stats 中与 decoder backend、hardware failure、decoder reset、decode/output drop 相关的观测收口
- Out of scope:
  - Linux FFmpeg / VAAPI / Vulkan Video
  - H.265 / AV1 / HDR / 10-bit
  - 音频解码或音频 backend 重构
  - 重写现有 `native_video` 双窗口/present 架构
  - 直接切换为“宿主系统 FFmpeg”依赖

## Plan

1. 先在 `xbxengine` 内建立统一视频 backend 抽象，明确 software decode、hardware decode、surface contract、probe result、failure reason 与 candidate 选择模型。
2. 以 FFmpeg software H.264 decode 替换现有 `openh264` 软解主线，并保持输出仍可走当前 `wgpu` CPU upload 路径，先跑通最小替换闭环。
3. 将现有 macOS `VideoToolbox` 手写路径迁移为 FFmpeg `VideoToolbox` backend，并新增 Windows `D3D11VA` H.264 backend；两端默认候选链都必须保持“硬解优先，软解仅 fallback”。
4. 按 Moonlight 思路补齐启动前 test decode / test render 验活、连续 backend failure 后的 decoder/backend 重选、窗口/显示变化触发的 recreate 语义。
5. 补齐 `Win/mac` 两端硬解 surface 的零拷贝 import/present 主线：macOS 保持 `CVPixelBuffer -> Metal/wgpu/layer`，Windows 新增 `D3D11 texture/surface -> wgpu/native_video` 直连，不再以 `av_hwframe_transfer_data -> BGRA` 为交付目标。
6. 在 `src-tauri` 层补齐 FFmpeg 运行时随包分发与加载路径配置，确保发布包不依赖宿主 FFmpeg 环境。

## Module Migration Checklist

### A. Decode 主控收口到统一 candidate/probe 模型

- 目标文件：
  - [`crates/xbxengine/core/src/media/video/decode/video_decode.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/media/video/decode/video_decode.rs)
  - [`crates/xbxengine/core/src/media/video/decode/actor.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/media/video/decode/actor.rs)
  - [`crates/xbxengine/core/src/media/video/types.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/media/video/types.rs)
- 当前问题：
  - `video_decode.rs` 同时承担 decode 主控、恢复状态机、macOS VideoToolbox 特例实现，缺少“候选链路 -> probe -> 选中 backend -> 运行期重建”的明确边界。
  - `create_hardware_video_decoder()` 当前只有 macOS 特例，没有统一的 Windows/macOS/software 候选排序和 failure reason 语义。
- 迁移动作：
  - 将 `video_decode.rs` 拆为“主控状态机”和“backend 实现”两层。
  - 新增 `VideoBackendCandidate` / `VideoBackendProbeResult` / `VideoBackendFailureReason` 等显式类型。
  - 把现有 `XbxHardwareVideoDecoder` 升级为同时覆盖 software/hardware backend 的统一 trait，而不是“硬解特例 trait + 软解隐式主线”。
  - 明确 runtime 主控只消费统一的 `decode()/reset()/probe()` 合同，不再直接知道 `VideoToolbox` 细节。

### B. 现有 macOS 手写 VideoToolbox 路径的收编与退役

- 目标文件：
  - [`crates/xbxengine/core/src/media/video/decode/video_decode.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/media/video/decode/video_decode.rs)
  - [`crates/xbxengine/core/src/api/backend.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/api/backend.rs)
- 当前问题：
  - `MacOsVideoToolboxDecoder` 直接嵌在 `video_decode.rs` 内，既影响 decode 主控清晰度，也使后续 FFmpeg `VideoToolbox` 接入存在双实现并存风险。
  - `MacOsCVPixelBufferDescriptor` 已是现成桥接合同，后续 FFmpeg `VideoToolbox` 最好复用该 descriptor，而不是再造一套 macOS native handle 类型。
- 迁移动作：
  - 将现有 `MacOsVideoToolboxDecoder` 视为阶段性替代实现，RFC 落地后最终目标是由 FFmpeg `VideoToolbox` backend 取代。
  - 保留并复用 `MacOsCVPixelBufferDescriptor` 作为 present bridge 合同。
  - 在切换阶段允许“旧 VT backend 仅作为开发 fallback 或对照实现”短暂存在，但不得继续作为默认主线。

### C. Render frame / surface contract 扩展

- 目标文件：
  - [`crates/xbxengine/core/src/media/video/render/renderer.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/media/video/render/renderer.rs)
  - [`crates/xbxengine/core/src/api/backend.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/api/backend.rs)
  - [`crates/xbxengine/core/src/media/video/types.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/media/video/types.rs)
- 当前问题：
  - `XbxEngineRenderPixelData` 已支持 `Rgba/Bgra/Nv12/Descriptor`，但 descriptor 语义当前偏向 macOS `CVPixelBuffer`。
  - Windows `D3D11VA` 接入后，必须明确是先走 copyback 到 `Nv12/Bgra`，还是新增 D3D11 surface descriptor。
- 迁移动作：
  - `macOS` 继续以 `MacOsCVPixelBufferDescriptor` 为零拷贝 descriptor 主线。
  - `Windows` 不再把 `copyback/CPU upload` 视为可交付主线，必须扩展 descriptor 合同以承载 `D3D11VA` 输出 surface，并直接喂给 render/native presenter。
  - 明确“decode backend 负责 frame 生产，render state 继续只负责 latest-slot / overwrite / present signal，不承担平台解码逻辑”。

### D. `native_video` / `wgpu` present bridge 对齐

- 目标文件：
  - [`src-tauri/src/mods/native_video/mod.rs`](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/mods/native_video/mod.rs)
  - [`src-tauri/src/mods/native_video/presenters.rs`](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/mods/native_video/presenters.rs)
  - [`src-tauri/src/mods/native_video/wgpu_renderer.rs`](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/mods/native_video/wgpu_renderer.rs)
  - [`src-tauri/src/mods/native_video/native_video_policy.rs`](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/mods/native_video/native_video_policy.rs)
- 当前问题：
  - `native_video` 已有成熟的 macOS `CVPixelBuffer -> layer/Metal/wgpu` 入口，但 Windows 仍缺与 D3D11 surface 对应的 present 策略。
  - 现有 presenter policy 假设的 descriptor 类型主要是 macOS native direct；FFmpeg 接入后需要重新定义 Win/mac/software 三类 frame 的初始 presenter 选择。
- 迁移动作：
  - `native_video` 需要明确区分“CPU upload fallback”与“native zero-copy”两类 present 主线。
  - macOS 保持 descriptor/native direct 主线；Windows 必须补齐 `D3D11` native import/direct present 或等价的 `wgpu` 零拷贝 import 路径。
  - 在 policy 中新增“FFmpeg software fallback / FFmpeg macOS native zero-copy / FFmpeg windows native zero-copy”三类显式 pipeline 来源，用于 trace 和 runtime stats 对齐。

### E. Recovery / stats / diagnostics 口径保持 Rust-owned 主权

- 目标文件：
  - [`crates/xbxengine/core/src/session/recovery.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/session/recovery.rs)
  - [`crates/xbxengine/core/src/api/runtime/lifecycle.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/api/runtime/lifecycle.rs)
  - [`crates/xbxengine/core/src/diagnostics/stats.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/diagnostics/stats.rs)
  - [`crates/xbxengine/protocol/src/runtime.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/protocol/src/runtime.rs)
- 当前问题：
  - 现有恢复逻辑已经深度依赖 decoder stall、hardware failure streak、decoder reset count、decode/output drop 等观测。
  - 如果 FFmpeg backend 自行吞掉错误或用另一套 failure 分类，会直接污染现有恢复策略。
- 迁移动作：
  - 明确 FFmpeg backend 只上报“观测和错误”，keyframe request / decoder reset / reconnect 仍由现有 runtime lifecycle / recovery coordinator 决策。
  - 对齐 runtime stats 中的 `video_decoder_backend_name`、`video_decoder_hardware_failure_streak`、`latest_video_decoder_hardware_failure_status`、`video_decoder_reset_count` 等字段，让迁移前后可横向对比。
  - 为 probe failure、runtime decode failure、render bridge failure 建立不同的 failure reason，避免所有错误都挤进 `decoderBackendFailure`。

### F. 打包与运行时库装载边界

- 目标文件：
  - [`src-tauri/Cargo.toml`](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/Cargo.toml)
  - [`src-tauri/build.rs`](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/build.rs)
  - [`src-tauri/tauri.conf.json`](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/tauri.conf.json)
- 当前问题：
  - 当前 `src-tauri` 打包配置还没有面向第三方视频动态库的显式处理。
  - 本 RFC 已明确不依赖宿主 FFmpeg 环境，因此需要把“开发期系统 SDK”与“发布期应用私有 runtime”拆开设计。
- 迁移动作：
  - 为 FFmpeg 增加 feature gate 与构建发现逻辑。
  - 为 Win/mac 分别补齐运行时库复制、bundle 布局、加载路径与签名说明。
  - 在 RFC 落地阶段优先支持“开发期可用本机 SDK，发布期必须随包带 runtime”的双模式，不直接要求全员先手工配置宿主 FFmpeg 环境。

## Milestones

### M1. 抽象层落地，不切主线

- 目标：
  - 建立统一 backend/candidate/probe/failure reason 抽象。
  - 不改变当前默认解码行为。
- 完成标准：
  - `video_decode` 主控与 backend 实现完成结构拆分。
  - 现有 macOS `VideoToolbox` 特例被隔离到独立 backend 模块。

### M2. FFmpeg software H.264 闭环

- 目标：
  - 引入 FFmpeg software H.264 decode backend。
  - 在现有 `wgpu` CPU upload 路径上完成最小替换闭环。
- 完成标准：
  - 能在 Win/mac 跑通 H.264 首帧。
  - recovery / stats / diagnostics 不回退。
  - 旧 `openh264` 主线被移除，不再保留并行 fallback。

### M3. macOS FFmpeg VideoToolbox 收编

- 目标：
  - 用 FFmpeg `VideoToolbox` backend 替代现有手写 `MacOsVideoToolboxDecoder` 主线角色。
  - 复用当前 `MacOsCVPixelBufferDescriptor` 与 native direct present 桥接。
- 完成标准：
  - macOS probe / decode / present / reset/recreate 跑通。
  - 旧手写 VT 路径完全移除，不再保留并行 fallback。

### M4. Windows FFmpeg D3D11VA 第一阶段

- 目标：
  - 新增 Windows `D3D11VA` backend。
  - 第一阶段跑通 probe + decode + recovery/recreate；允许暂时使用 copyback/CPU upload 作为过渡实现，但不作为最终交付目标。
- 完成标准：
  - probe 失败能自动回落 software decode。
  - runtime failure 能通过现有 recovery 触发 recreate。

### M5. Win/mac 零拷贝主线收口

- 目标：
  - `Win/mac` 都落实“硬解优先 + 零拷贝主线”。
  - Windows 去掉 `D3D11VA -> av_hwframe_transfer_data -> BGRA` 的交付依赖，切到 native surface import/direct present。
- 完成标准：
  - macOS 默认命中 `ffmpeg-videotoolbox + CVPixelBuffer descriptor/native present`。
  - Windows 默认命中 `ffmpeg-d3d11va + D3D11 surface zero-copy import/present`。
  - software decode 与 CPU upload 只在 probe/runtime failure 时作为 fallback 出现，并且 telemetry 可观测。

### M6. 打包/发布闭环

- 目标：
  - Win/mac 发布包随带 FFmpeg runtime。
  - 运行时不依赖宿主 FFmpeg。
- 完成标准：
  - Windows bundle 可在干净环境启动。
  - macOS bundle 的 dylib 布局、签名链和加载路径已验证。
  - `docs/project-task.md`、RFC 进度和最终 report 对齐。

## Validation

- [ ] `Win/mac` 两端都能在不安装宿主 FFmpeg 的情况下完成打包、启动与 H.264 串流首屏展示。
- [ ] FFmpeg software decode 能在 `wgpu` CPU upload 路径上稳定替换现有 `openh264`，且现有 runtime stats / diagnostics 合同不回退。
- [ ] macOS `VideoToolbox` 与 Windows `D3D11VA` 都具备启动前 probe 机制，probe 失败时能自动回落 FFmpeg software decode，而不是启动后黑屏。
- [ ] 现有 keyframe request / decoder reset / recovery escalation 仍由 `xbxengine` 控制面主导，decoder/backend failure 可被正确观测并触发 recreate。
- [ ] `Win/mac` 默认主线都满足“硬解优先于软解”，软件解码只在 probe/runtime failure 后才接管。
- [ ] `Win/mac` 默认主线都满足“零拷贝 import/present”，CPU upload / copyback 只作为 fallback，并且 telemetry 可区分。
- [ ] `src-tauri` 发布包包含 FFmpeg 运行时，签名/加载路径在 `Win/mac` 可验证通过。

## Risks

- FFmpeg 引入会带来新的构建、打包、签名和运行时库管理复杂度，尤其 macOS `.app` 内动态库路径与签名链需要额外处理。
- 现有 `DecodedFrame` / `native_video` / `wgpu_renderer` 主要围绕 CPU upload 与 macOS `CVPixelBuffer` 设计，Windows `D3D11VA` surface contract 可能迫使 frame abstraction 与 render bridge 扩展。
- 若直接将平台差异下沉进 FFmpeg backend 而缺少统一 candidate/probe 层，容易复制当前手写路径分叉问题，只是把分叉转移到 FFmpeg 上。
- 现有 recovery policy 已深度依赖 decoder stall、hardware failure streak、decoder reset count 等观测，迁移时若漏掉统计口径，将直接影响恢复策略正确性。
- 仅支持 H.264 的阶段性方案需要明确写死边界，避免后续 H.265 / AV1 需求直接挤压本 RFC 范围。

## Progress

- [x] Step 1: 建立 FFmpeg 主控下的视频 backend / candidate / probe 抽象
- [x] Step 2: 用 FFmpeg software decode 替换现有 `openh264` 软解主线
- [x] Step 3: 接入 macOS `VideoToolbox` FFmpeg backend（Windows `D3D11VA` 仍留在 M4）
- [x] Step 4: 收口 reset/recreate、window/display 变化与 runtime stats 观测
- [ ] Step 5: 完成 `Win/mac` FFmpeg runtime 打包与发布验证

## Execution Notes

- Date: 2026-04-06 | Status: planned
- Update: 新建 RFC，范围明确限定为 `Win/mac + H.264`，目标是以 FFmpeg 统一替换现有软解和平台硬解入口，不同时扩展 Linux/H.265/AV1/HDR。
- Decision: 参考 Moonlight，优先借鉴“统一 FFmpeg 主控 + backend/frontend 组合、probe 验活、失败后重选链路、自愈 reset/recreate”四类设计；不照搬 Qt/SDL 事件循环和 renderer 具体实现。
- Decision: 发布策略明确采用“应用自带 FFmpeg 运行时”，不依赖宿主 FFmpeg 环境。
- Risk/Blocker: 当前 `native_video` 与 `wgpu_renderer` 的 platform import 能力对 macOS 更成熟，对 Windows `D3D11VA` surface 导入仍需补齐桥接设计；这是本 RFC 后续实现的第一优先级结构风险。
- Date: 2026-04-06 | Status: in progress
- Update: 已完成 M1 最小骨架重构：`video_decode.rs` 仅保留主控/恢复/队列逻辑；新增 [`crates/xbxengine/core/src/media/video/decode/backend.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/media/video/decode/backend.rs) 承载统一 backend trait、candidate/probe/failure reason 与工厂；新增 [`crates/xbxengine/core/src/media/video/decode/backend_macos_videotoolbox.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/media/video/decode/backend_macos_videotoolbox.rs) 承载现有 macOS VideoToolbox 实现与 FFI。
- Decision: M1 不切默认主线、不引入 FFmpeg、不改恢复策略；当前候选链维持为 `macOS videotoolbox -> software-placeholder -> noop`，用于先把结构边界立住。
- Validation: `cargo check -p xbxengine`、`cargo test -p xbxengine request_decoder_reset_calls_hardware_decoder_reset --lib -q` 已通过；现阶段仍存在仓库既有 `dead_code/unused` 警告，本轮未扩展到非目标模块处理。
- Date: 2026-04-06 | Status: in progress
- Update: 已完成 M2/M3 代码接入：新增 [`crates/xbxengine/core/src/media/video/decode/backend_ffmpeg.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/media/video/decode/backend_ffmpeg.rs)、[`crates/xbxengine/core/src/media/video/decode/backend_ffmpeg_software.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/media/video/decode/backend_ffmpeg_software.rs) 与 [`crates/xbxengine/core/src/media/video/decode/backend_ffmpeg_macos_videotoolbox.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/media/video/decode/backend_ffmpeg_macos_videotoolbox.rs)。当前候选链已改为 `macOS: ffmpeg-videotoolbox -> ffmpeg-software -> legacy videotoolbox`，`Win: ffmpeg-software -> noop`。
- Decision: M2 software backend 当前输出 `BGRA` CPU 内存帧，以最小改动复用现有 `wgpu` upload 路径；M3 macOS backend 使用 FFmpeg `AV_HWDEVICE_TYPE_VIDEOTOOLBOX + AV_PIX_FMT_VIDEOTOOLBOX`，并继续复用 `MacOsCVPixelBufferDescriptor` 直连宿主 present。
- Validation: `cargo check -p xbxengine`、`cargo test -p xbxengine request_decoder_reset_calls_hardware_decoder_reset --lib -q`、`cargo test -p xbxengine backend_failure_then_clean_bootstrap_frames_recover_pipeline_to_nominal --lib -q` 已通过。当前尚未完成真实 Win/mac 首屏串流与打包验证，因此 RFC 顶层 Validation 条目仍保持未勾选。
- Date: 2026-04-07 | Status: in progress
- Update: 已开始 M4 第一阶段，新增 [`crates/xbxengine/core/src/media/video/decode/backend_ffmpeg_windows_d3d11va.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/media/video/decode/backend_ffmpeg_windows_d3d11va.rs)，当前实现路径为 `D3D11VA hw decode -> av_hwframe_transfer_data -> BGRA`，继续复用现有 `wgpu` CPU upload 路径，不引入新的 Windows surface descriptor 或 `native_video` 直显改造。
- Decision: Windows 首轮仍按 RFC 约束只做 copyback/CPU upload，不尝试 D3D11 -> wgpu 零拷贝；同时保持 `video_decode.rs` 恢复/队列主控不变，只在 backend probe 链中把 Windows 候选调整为 `ffmpeg-d3d11va -> ffmpeg-software -> noop`。
- Validation: `cargo fmt --all`、`cargo check -p xbxengine` 已通过；尝试 `cargo check -p xbxengine --target x86_64-pc-windows-msvc` 时因本机未安装 Windows Rust target 失败，错误为缺少 `core` crate 与目标三元组，因此当前仅完成代码级落地，尚未完成 Windows target 编译验证。
- Date: 2026-04-07 | Status: in progress
- Update: 已补齐 Step 4 的最小 decoder probe/runtime 观测链：`video_decode` 初始化时会记录最近一次 decoder backend 选择结果，并通过 [`crates/xbxengine/core/src/api/backend.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/api/backend.rs)、[`crates/xbxengine/core/src/diagnostics/stats.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/diagnostics/stats.rs) 与 [`src-tauri/src/mods/xbxengine/trace_projection.rs`](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/mods/xbxengine/trace_projection.rs) 暴露 `selected_backend_name / selected_backend_kind / fallback_count / fallback_summary`，用于直接观察 Windows/macOS 在 probe 阶段是否因硬解初始化失败而回落 software。
- Decision: 本轮只补 probe/selection 的 runtime 证据链，不改变现有 recovery/reset 策略，也不提前把 Step 4 扩大到 Windows display change 或 backend reset 语义；后续若要真正做“连续失败后重选 backend”，需在此观测基础上再收口 reset 合同。
- Validation: `cargo fmt --all`、`cargo check -p xbxengine`、`cargo test -p xbxengine request_decoder_reset_calls_hardware_decoder_reset --lib -q` 已通过；目前仍未补 Windows target 编译与 Win/mac 实机串流验证，因此 Step 4 与顶层 Validation 继续保持未完成状态。
- Date: 2026-04-07 | Status: in progress
- Update: 已把 `backend failure -> decoder reset -> 重新 probe` 真正接入 [`crates/xbxengine/core/src/media/video/decode/video_decode.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/media/video/decode/video_decode.rs)：当 decode backend 因硬解失败进入 `BackendFailureEscalated` 时，主控不再只调用同一 decoder 的 `reset()`，而是直接重跑 backend factory，切到新的候选 backend，并同步刷新最近一次 probe observation。这里“decoder reset”是恢复语义，具体实现手段是 backend 重建。测试侧已补“硬解失败后回落 software”与相关旧回归用例的 reset 语义。
- Decision: 本轮把 reset 范围严格限制在“backend failure 触发的 decoder 内部重置”；外部 `request_decoder_reset()`、window/display change 触发的 local decoder reset 以及更细的 failure 分类观测仍留在 Step 4 后续。
- Validation: `cargo fmt --all`、`cargo check -p xbxengine`、`cargo test -p xbxengine backend_failure_resets_decoder_via_probe_factory_and_updates_probe_snapshot --lib -q`、`cargo test -p xbxengine hardware_decode_failure_escalates_recovery_state_to_waiting_keyframe --lib -q`、`cargo test -p xbxengine bad_data_failure_waits_for_next_keyframe_before_decoding_again --lib -q`、`cargo test -p xbxengine backend_failure_then_clean_bootstrap_frames_recover_pipeline_to_nominal --lib -q` 已通过。当前仍未补 Windows target 编译与 Win/mac 实机串流验证。
- Date: 2026-04-07 | Status: in progress
- Update: 已完成 Step 4 剩余收口：[`crates/xbxengine/core/src/media/video/decode/video_decode.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/media/video/decode/video_decode.rs) 的本地 decoder reset 语义已统一为“reset 语义、backend 重建实现”，不再保留与 `reset()` 并列的另一套 `recreate` 语义命名；同时在 [`crates/xbxengine/core/src/media/video/decode/actor.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/media/video/decode/actor.rs)、[`crates/xbxengine/core/src/transport/rtc/pipeline/supervisor.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/pipeline/supervisor.rs)、[`crates/xbxengine/core/src/transport/rtc/stack/runtime_port.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stack/runtime_port.rs) 与 [`crates/xbxengine/core/src/transport/rtc/stack.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stack.rs) 新增 local-only decoder reset 控制链，使 host display/frame-age timing 在显著变化时可触发一次本地 decoder reset，并通过冷却窗口避免 telemetry tick 抖动导致重复触发。
- Decision: 继续保持 remote `request_decoder_reset()` 只代表 transport/control channel 的远端命令；本轮新增的是 runtime/host telemetry 专用的 local decoder reset 入口，语义上不与远端 decoder reset 混用。窗口/显示变化检测暂时以 host timing 显著变化代理，不直接侵入 `native_video` 或 recovery 主链。
- Validation: `cargo fmt --all`、`cargo check -p xbxengine`、`cargo test -p xbxengine request_local_decoder_reset_rebuilds_backend_and_updates_probe_snapshot --lib -q`、`cargo test -p xbxengine recovery_fsm_moves_from_waiting_keyframe_to_recovering_then_nominal --lib -q`、`cargo test -p xbxengine small_host_timing_change_does_not_trigger_local_decoder_reset --lib -q`、`cargo test -p xbxengine obvious_host_timing_change_triggers_local_decoder_reset --lib -q`、`cargo test -p xbxengine reset_trigger_is_debounced_within_cooldown_window --lib -q` 已通过。当前仍未补 Windows target 编译与 Win/mac 实机串流验证，因此顶层 Validation 与 M5 仍未完成。
- Date: 2026-04-07 | Status: re-scoped
- Update: 需求基线调整为“硬解优先于软解、0 拷贝必须落实”。现状上，硬解优先链已具备：Windows 为 `ffmpeg-d3d11va -> ffmpeg-software -> noop`，macOS 为 `ffmpeg-videotoolbox -> ffmpeg-software -> noop`；但零拷贝只在 macOS `CVPixelBuffer descriptor -> Metal/wgpu/native layer` 路径部分成立，Windows 仍停留在 [`backend_ffmpeg_windows_d3d11va.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/media/video/decode/backend_ffmpeg_windows_d3d11va.rs) 的 `av_hwframe_transfer_data -> BGRA` copyback 路线。
- Decision: 自本次起，Windows `copyback/CPU upload` 不再视为可交付主线；后续实现必须把 `D3D11VA` 输出 surface 纳入 descriptor/native import 合同，形成与 macOS 对称的零拷贝 present 主线。
- Date: 2026-04-07 | Status: in progress
- Update: 已启动 M5 的 Windows descriptor 主线收口：[`crates/xbxengine/core/src/api/backend.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/api/backend.rs) 新增 `WindowsD3d11TextureDescriptor`；[`crates/xbxengine/core/src/media/video/decode/backend_ffmpeg_windows_d3d11va.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/media/video/decode/backend_ffmpeg_windows_d3d11va.rs) 现会优先尝试把 `AV_PIX_FMT_D3D11VA_VLD` 帧导出为 `IDXGIResource1::CreateSharedHandle` 驱动的 descriptor，并仅在 shared handle 导出失败时显式回落 `av_hwframe_transfer_data -> BGRA` copyback；[`src-tauri/src/mods/native_video/types.rs`](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/mods/native_video/types.rs)、[`src-tauri/src/mods/native_video/native_video_policy.rs`](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/mods/native_video/native_video_policy.rs) 与 [`src-tauri/src/mods/native_video/presenters.rs`](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/mods/native_video/presenters.rs) 也已补齐 Windows descriptor 的 surface 识别、policy 分类与 telemetry 命名，不再把它一律视为 `UnknownDescriptor`。
- Decision: 本轮只把“Windows 解码端默认产出 descriptor、copyback 改为显式 fallback、宿主 surface/policy/telemetry 能识别 Windows native handle”落到可编译状态；真正的 `shared handle -> wgpu/native_video` 导入消费仍是 M5 剩余主线，未在本轮假装完成。
- Validation: `cargo fmt --all`、`cargo check -p xbxengine`、`cargo check -p xbxrc` 已通过；由于当前环境缺少 Windows target 与实机 presenter 验证，本轮尚未确认 `CreateSharedHandle` 在目标设备/驱动组合上的命中率，也未完成 Windows shared handle 的真实渲染导入。
- Date: 2026-04-07 | Status: in progress
- Update: 已继续按 Moonlight 思路补齐 Windows renderer/presenter：[`src-tauri/src/mods/native_video/wgpu_renderer.rs`](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/mods/native_video/wgpu_renderer.rs) 新增 Windows `WgpuFrameRenderer`，默认用 `wgpu::Backends::VULKAN` 请求 `TEXTURE_FORMAT_NV12 + VULKAN_EXTERNAL_MEMORY_WIN32`，并通过 `wgpu-hal` 的 `texture_from_d3d11_shared_handle()` 直接把 `WindowsD3d11TextureDescriptor` 导入为 `NV12` 纹理，再以 plane 视图接入现有 `NV12` shader；[`src-tauri/src/mods/native_video/presenters.rs`](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/mods/native_video/presenters.rs) 新增 `WindowsWgpuPresenter`，`NativeVideoRegistry::create_presenter()` 现会在 Windows `GpuDirect` 路径返回真实 presenter，而不再直接落到 `Noop`。
- Decision: Windows 本轮选择“Vulkan external-memory import shared handle”作为首条可落地的 zero-copy presenter 主线，而不是继续停留在 descriptor 中间层；若后续遇到驱动兼容性问题，再按 Moonlight 的思路补充 quirk gate 与 copy/bind 双路径，而不是回退到默认 CPU upload。
- Validation: 在当前 macOS 开发机上，`cargo fmt --all` 与 `cargo check -p xbxrc` 继续通过，说明 `native_video`/`wgpu_renderer` 的跨平台条件编译未被破坏；但本轮仍未完成 `x86_64-pc-windows-msvc` 目标编译、Windows 实机 `VULKAN_EXTERNAL_MEMORY_WIN32` feature 命中验证，以及实际串流首屏验证，因此 M5 仍保持 in progress。
- Date: 2026-04-07 | Status: in progress
- Update: 已进一步把 Windows D3D11VA decode 资源分配前移到 `get_format()` 阶段：[`crates/xbxengine/core/src/media/video/decode/backend_ffmpeg_windows_d3d11va.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/media/video/decode/backend_ffmpeg_windows_d3d11va.rs) 现在会在选择 `AV_PIX_FMT_D3D11VA_VLD` 时调用 `avcodec_get_hw_frames_parameters()` 创建 `AVHWFramesContext`，并在 `av_hwframe_ctx_init()` 前把 D3D11VA hwctx 的 `BindFlags/MiscFlags` 强制并入 `D3D11_BIND_SHADER_RESOURCE | D3D11_RESOURCE_MISC_SHARED | D3D11_RESOURCE_MISC_SHARED_NTHANDLE`。这一步把 Windows 路线从“解码后再导出共享句柄”推进成更接近 Moonlight 的“解码 surface 自分配开始就满足 renderer/import 所需资源能力”。
- Decision: 本项目 Windows 零拷贝主线现在明确采用“两段约束”策略：1) FFmpeg `hw_frames_ctx` 初始化时先约束解码 surface 资源属性；2) 宿主 renderer 再以 `Vulkan external-memory import` 直接消费 shared handle。后续即使要补 quirk gate 或 bind/copy 双路径，也必须建立在这个前移后的资源分配语义上，而不是退回“默认 copyback，再看能否导入”。
- Validation: `cargo fmt --all`、`cargo check -p xbxengine`、`cargo check -p xbxrc` 已通过；本轮仅完成代码级与宿主侧条件编译验证，尚未完成 Windows 目标编译和不同 GPU/驱动上的 FFmpeg hwframes 分配行为验证。
- Date: 2026-04-07 | Status: in progress
- Update: 已从 [`crates/xbxengine/core/src/media/video/decode/backend.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/media/video/decode/backend.rs) 的 probe 候选链中移除 `videotoolbox-legacy`，并删除 [`crates/xbxengine/core/src/media/video/decode/backend_macos_videotoolbox.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/media/video/decode/backend_macos_videotoolbox.rs)。至此 macOS 旧手写 VideoToolbox backend 已不再参与编译，也不再作为 FFmpeg backend 失败后的隐藏 fallback。
- Decision: macOS H.264 解码主线现正式收敛为 `ffmpeg-videotoolbox -> ffmpeg-software -> noop`；后续若 `ffmpeg-videotoolbox` 命中问题，应在统一 FFmpeg backend 内补 quirk/probe/fallback，而不是恢复一条并行手写 VT 实现。
- Validation: 待本轮 `cargo fmt --all`、`cargo check -p xbxengine`、`cargo check -p xbxrc` 回归后确认。
- Date: 2026-04-07 | Status: in progress
- Update: 已移除 [`crates/xbxengine/core/src/media/video/decode/backend.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/media/video/decode/backend.rs) 中 `XbxVideoDecoderBackend::reset()` 旧接口，并同步删除 FFmpeg backend 与 [`crates/xbxengine/core/src/media/video/decode/video_decode.test.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/media/video/decode/video_decode.test.rs) 的残留空实现/断言。当前 decoder reset 统一表示恢复语义；具体落地是通过 probe factory 重建 backend，而不是暴露一套并列的全局 `recreate` 语义接口。
- Decision: 后续 decoder 恢复语义统一以 `reset` 为准；如果要补平台 quirk 或轻量清理动作，应内聚在具体 FFmpeg backend 内部，不再通过 trait 层暴露一个全局 `reset()` 合同。
- Validation: 待本轮 `cargo fmt --all`、`cargo check -p xbxengine`、`cargo check -p xbxrc` 回归后确认。
