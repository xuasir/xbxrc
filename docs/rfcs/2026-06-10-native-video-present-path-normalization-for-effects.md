# Native Video Present Path Normalization For Effects RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 未完成
- Current State: in-progress
- Owner: agent
- Last Updated: 2026-06-10

## Background

- 当前 `rust-owned` 的上屏路径同时存在 macOS `AVSampleBufferDisplayLayer` 直出、macOS WGPU、Windows WGPU。
- macOS 直出路径已经沉淀出较完整的 display cadence、retained redraw、host mailbox、hostTiming 诊断能力。
- WGPU 路径已经复用 `ScheduledFrameSlot + HostCadenceTelemetry`，但 tick 语义、retained frame 来源、idle/rejected 诊断仍有平台实现差异。
- Rust-owned 的后续画质 effect 适合挂在 WGPU/effect pipeline 上，前置条件是 win/mac 的 WGPU 与直出路径先具备统一上屏调度合同。

## Goal

- 将 win/mac native video 上屏路径收敛到统一的调度语义：
  - host display tick
  - latest-only mailbox
  - retained displayed redraw
  - stale/idle/rejected 诊断
  - host present telemetry
- 让 WGPU 路径具备 macOS 直出路径已有的节拍和诊断能力。
- 为后续 `rust-owned` 画质 effect 新增统一插入点：effect pipeline 只接入规范化后的 WGPU path。

## Scope

- In scope:
  - `src-tauri/src/mods/native_video/scheduling.rs`
  - `src-tauri/src/mods/native_video/mod.rs`
  - `src-tauri/src/mods/native_video/presenters.rs`
  - `src-tauri/src/mods/native_video/effects.rs`
  - native video diagnostics / hostTiming 事件口径
- Out of scope:
  - 具体画质 effect shader 本体
  - 默认启用画质实验
  - 替换 WebRTC / decode / recovery 主线
  - 改动 webrtc-direct 浏览器渲染实现

## Plan

1. 统一 retained displayed frame 合同：`ScheduledFrameTakeOutcome::RetainedDisplayedFrame` 直接携带 displayed frame，直出与 WGPU 都消费同一事实源。
2. 抽取 host mailbox tick 诊断：retained / idle / rejected 事件从 layer 专属逻辑上移为共享函数，WGPU 路径同口径输出。
3. 收敛 WGPU tick 执行语义：macOS/Windows WGPU 共享相同 take/render/refresh 语义，并保持 display-link/fallback loop 的 pending/rerun 纪律。
4. 在 effect pipeline 前固定能力门：只有规范化 WGPU path 承接后续非 `Noop` effect，直出路径继续作为低成本基线。
5. 标准化 effect pipeline 生命周期：处理结果、fallback reason、输入/输出尺寸、render cost 进入同一诊断口径。
6. 规范化 WGPU renderer：共享 shader / bind group layout / sampler / render pipeline / surface config / surface present / render pass / CPU texture upload / NV12 bind group 构建逻辑，平台实现只保留 surface 创建与 native import 差异。
7. 规范化 WGPU take outcome 执行：macOS WGPU 与 Windows WGPU 共用 ready / retained / idle / stale 分支处理、hostTiming 输出、present/refresh telemetry 更新，平台实现只保留 renderer lifecycle 与 host view/window 差异。
8. 规范化 WGPU display tick 取帧入口：macOS WGPU 与 Windows WGPU 共用 display tick 记录、view epoch 切换、`ScheduledFrameSlot::take_ready_frame()` 与 diagnostics snapshot 采集。

## Validation

- [x] `cargo fmt -p xbxrc`
- [x] `cargo test -p xbxrc --lib native_video`
- [x] `cargo check -p xbxrc`
- [x] `git diff --check`
- [x] 代码层确认 macOS WGPU、Windows WGPU 与 layer 共用 retained/idle/rejected hostTiming 输出函数
- [x] 代码层确认 macOS WGPU、Windows WGPU 与 layer 共用 `hostFramePresented` 输出函数
- [x] 代码层确认 macOS WGPU 与 Windows WGPU 共用 display tick 取帧入口
- [x] 测试覆盖 WGPU display tick 取帧入口的 ready diagnostics 与 view epoch replay 合同
- [x] 代码层确认 macOS WGPU 与 Windows WGPU 共用 take outcome 执行函数
- [x] 代码层确认非 `Noop` effect 必须匹配 `GpuDirect` presenter
- [x] 代码层确认 effect pipeline 输出 `effectKind/effectActive/fallbackReason/renderCost/input/output` 诊断
- [ ] Windows target 交叉 `cargo check -p xbxrc --target x86_64-pc-windows-msvc`：当前本机缺 Windows SDK / C 标准头，`ring` 与 `aws-lc-sys` 构建阶段报 `assert.h` / `windows.h` 缺失，尚未到达 crate 代码检查阶段。
- [ ] Windows target 交叉 `cargo check -p xbxrc --target x86_64-pc-windows-gnu`：当前本机缺 `x86_64-w64-mingw32-gcc`，`ring` 构建阶段报 linker/toolchain 缺失，尚未到达 crate 代码检查阶段。
- [ ] 新 trace 验证 macOS WGPU 与 layer 均输出 retained/idle/rejected 同口径 hostTiming
- [ ] 新 trace 验证 Windows WGPU retained redraw 不依赖平台缓存副本

## Risks

- WGPU retained redraw 若直接使用 slot frame，会暴露此前由 renderer cache 掩盖的生命周期问题，需要 trace 验证。
- hostTiming 增加 WGPU 事件后，分析脚本需按 pipeline 区分 `layer / wgpu / wgpu-windows`。
- 画质 effect 插入前必须保留直出路径的低成本 fallback，避免实验能力影响播放稳定性判断。

## Progress

- [x] Step 1: `RetainedDisplayedFrame` 携带 displayed frame，macOS layer、macOS WGPU、Windows WGPU 均从 scheduling outcome 取帧。
- [x] Step 2: host mailbox retained/idle/rejected 诊断共享化并接入 WGPU。
- [x] Step 3: WGPU tick 语义继续收敛。
- [x] Step 4: effect capability gate 设计。
- [x] Step 5: effect pipeline 生命周期与诊断合同标准化。
- [x] Step 6: WGPU renderer 资源、surface present、render pass 与 CPU 上传路径共享化。
- [x] Step 7: WGPU take outcome ready/retained/idle/stale 执行共享化。
- [x] Step 8: WGPU display tick 取帧入口共享化。

## Execution Notes

- Date: 2026-06-10 | Status: in-progress
- Update: 已完成第一阶段代码收敛：retained displayed frame 从 `ScheduledFrameSlot` 直接输出，WGPU 路径不再依赖 renderer state 缓存作为持帧重绘帧源。
- Validation: `cargo fmt -p xbxrc`、`cargo test -p xbxrc --lib native_video`、`cargo check -p xbxrc` 已通过。
- Date: 2026-06-10 | Status: in-progress
- Update: 已完成第二阶段代码收敛：抽出 `record_host_mailbox_retained_displayed / idle / rejected_stale`，macOS layer、macOS WGPU、Windows WGPU 对 retained displayed、no pending、stale dropped 使用同一 hostTiming payload 口径。
- Validation: `cargo fmt -p xbxrc`、`cargo test -p xbxrc --lib native_video`、`cargo check -p xbxrc` 已通过。
- Date: 2026-06-10 | Status: in-progress
- Update: 已完成第三阶段代码收敛：抽出 `record_host_frame_presented`，macOS layer、macOS WGPU、Windows WGPU 在真实 present / retained refresh 成功后输出同口径 `hostFramePresented`，用于后续区分 effect render 成本与 host 上屏事实。
- Validation: `cargo fmt -p xbxrc`、`cargo check -p xbxrc`、`cargo test -p xbxrc --lib native_video` 已通过。
- Date: 2026-06-10 | Status: in-progress
- Update: 已完成 effect gate 设计落地：`VideoEffectPipelineKind::required_presenter_mode()` 固定非 `Noop` effect 的 presenter 需求，policy 在不匹配时降回 `Noop`。后续画质 effect 只能接到规范化 WGPU path，直出路径继续保持低成本基线。
- Validation: `cargo fmt -p xbxrc`、`cargo test -p xbxrc --lib native_video`、`cargo check -p xbxrc` 已通过。
- Date: 2026-06-10 | Status: in-progress
- Update: 已完成 effect lifecycle 代码落地：`VideoEffectPipeline::process_frame()` 返回结构化 outcome，registry 统一记录 effect kind、active、fallback reason、render cost、input/output size，并通过 `videoEffectProcessed` hostTiming 输出。当前 WGPU effect 仍为 passthrough 占位。
- Validation: `cargo fmt -p xbxrc`、`cargo test -p xbxrc --lib native_video`、`cargo check -p xbxrc` 已通过。
- Date: 2026-06-10 | Status: in-progress
- Update: 已完成 WGPU renderer 初始化结构规范化：新增 `WgpuFrameRenderResources` 承载 copy/NV12 shader、bind group layout、sampler、render pipeline；新增 `build_surface_config()` 统一 surface config 构建。macOS/Windows renderer 保留平台 surface、纹理上传与 native import 差异。
- Validation: `cargo fmt -p xbxrc`、`cargo test -p xbxrc --lib native_video`、`cargo check -p xbxrc` 已通过。
- Date: 2026-06-10 | Status: in-progress
- Update: 已完成 WGPU renderer 主体规范化：新增 `render_wgpu_frame_texture_surface()`、`draw_frame_texture_render_source()`、`update_wgpu_surface_size()`，macOS/Windows WGPU 共用 surface acquire/reconfigure、render pass、aspect-fit viewport、submit/present 流程；新增 `create_rgba_texture_bundle()`、`write_rgba_texture()`、`create_nv12_cpu_texture_parts()`、`create_nv12_bind_group()`、`write_nv12_texture_planes()`，RGBA/BGRA 与 CPU NV12 上传、NV12 shader bind group 构建走同一口径，平台 native import 继续保留在各自实现内。
- Validation: `cargo fmt -p xbxrc`、`cargo test -p xbxrc --lib native_video`、`cargo check -p xbxrc`、`git diff --check` 已通过。`cargo check -p xbxrc --target x86_64-pc-windows-msvc` 被本机 Windows SDK / C 标准头缺失阻塞，报错停在 `ring` / `aws-lc-sys` C 依赖构建阶段；`cargo check -p xbxrc --target x86_64-pc-windows-gnu` 被本机缺 `x86_64-w64-mingw32-gcc` 阻塞，报错停在 `ring` 构建阶段。
- Date: 2026-06-10 | Status: in-progress
- Update: 已完成 WGPU take outcome 执行规范化：新增 `process_wgpu_render_take_outcome()`，macOS WGPU 与 Windows WGPU 的 ready / retained displayed / no pending / stale dropped 分支共用同一处理函数，统一 `hostMailboxTakeDecision`、retained/idle/stale 诊断、renderer update/render、present/refresh telemetry 与 `hostFramePresented` 输出。平台段只保留 renderer 创建、surface/host view 生命周期、rerun guard 与状态字段回写差异。
- Validation: `cargo fmt -p xbxrc`、`cargo test -p xbxrc --lib native_video`、`cargo check -p xbxrc`、`git diff --check` 已通过。
- Date: 2026-06-10 | Status: in-progress
- Update: 已完成 WGPU display tick 取帧入口规范化：新增 `take_wgpu_scheduled_frame()` 与 `WgpuScheduledFrameTake`，macOS WGPU 与 Windows WGPU 共用 display tick 记录、view epoch 切换、`take_ready_frame()` 调用以及 slot/telemetry diagnostics 采集。平台段保留锁失败后的退出或 rerun guard 策略。
- Validation: `cargo fmt -p xbxrc`、`cargo test -p xbxrc --lib native_video`、`cargo check -p xbxrc`、`git diff --check` 已通过。
- Date: 2026-06-10 | Status: in-progress
- Update: 已补齐 WGPU display tick 取帧入口回归测试：`wgpu_scheduled_frame_take_records_display_tick_and_ready_diagnostics` 锁定 display tick epoch、ready frame 与 diagnostics；`wgpu_scheduled_frame_take_replays_displayed_frame_for_view_epoch_change` 锁定 host view epoch 切换时 replay displayed frame 与 displayed view epoch。
- Validation: `cargo fmt -p xbxrc`、`cargo test -p xbxrc --lib native_video`（48 passed）、`cargo check -p xbxrc`、`git diff --check` 已通过。
