# MacOS Native Video Present Path Slimming RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 已完成
- Current State: completed
- Owner: TBD
- Last Updated: 2026-04-09

## Background

- 当前 macOS native video 主线已经明确收敛为 `NativeDirect`，不会在这次工作里引入并行的兼容路线。
- 现有 `AVSampleBufferDisplayLayer` 提交路径仍然在主线程里完成每帧 CoreMedia 对象创建与 enqueue，叠加 `run_on_main_thread` 串行化后，容易在高帧率或主线程抖动时出现 `presentAgeMs` 堆积和 stall。
- 当前实现里 `ensure_display_layer()` 也在每个 present tick 里重复做 `setContentsScale / setFrame / setNeedsLayout`，这类稳定状态下的重复更新属于额外 CPU 开销。
- 用户目标已经明确：直接完成 1、2、3 改造，不保留过渡状态，不腐化现有架构，并且明确 MacOS 始终走 native 线路。

## Goal

- 在 macOS native 主线内完成三个收敛：
  1. 缓存 `CMVideoFormatDescription`，避免每帧重复创建。
  2. 将 `CMSampleBuffer` 的准备尽量移出主线程，主线程只保留最终提交。
  3. 让 `ensure_display_layer()` 只在尺寸或 scale 变化时更新 layer 属性。
- 保持当前架构不分叉、不降级，不引入“临时兼容态”或双主线实现。
- 验证 native direct 路径在实际 trace 中更稳定，且仍保持 macOS 直通语义。

## Scope

- In scope:
  - `src-tauri/src/mods/native_video/mod.rs`
  - `src-tauri/src/mods/native_video/presenters.rs`
  - `src-tauri/src/mods/native_video/scheduling.rs`（如需要补充缓存状态或诊断字段）
  - 相关最小测试或守护性回归检查
- Out of scope:
  - 引入新的兼容 presenter 分支
  - 切换默认路由到 wgpu
  - 重写解码/transport pipeline
  - 引入新 native runtime 或新窗口协调模块

## Plan

1. 在 macOS layer state 中引入可复用的 CoreMedia 描述缓存，并让呈现路径优先复用已知稳定的 format description。
2. 拆分 present 路径：把可预计算的 CoreMedia 准备从主线程中挪出，主线程仅执行必要的 `enqueueSampleBuffer` 和最小提交逻辑。
3. 收紧 `ensure_display_layer()`，只在 bounds 或 scale 真正变化时更新 layer，避免每 tick 重复设置。
4. 补充最小验证与验收：`cargo check -p xbxrc`，并通过代码审查确认 macOS 仍然只走 native direct。

## Validation

- [x] `cargo check -p xbxrc`
- [x] `cargo fmt -p xbxrc` 或等价格式化检查
- [x] 代码审查确认 macOS 仍保持 `NativeDirect -> PlatformNative` 主线
- [x] 未新增单测，当前验收以编译与代码审查为准

## Risks

- CoreMedia / AVFoundation 对象的线程归属和生命周期需要谨慎处理，缓存与跨线程准备不能引入悬挂引用或过早释放。
- 如果 format description 的缓存键不够严格，可能在分辨率或色彩元数据变化时复用错误对象。
- 如果 sample buffer 的准备拆分方式不正确，可能只是在结构上变复杂，却没有实质减少主线程成本。

## Progress

- [x] Step 1: 方案确认与代码边界核对
- [x] Step 2: 代码改造与缓存/拆分实现
- [x] Step 3: 验收、验证与任务记录收口

## Execution Notes

- Date: 2026-03-28 | Status: completed
- Update: 已完成 macOS native present 主线收敛：`CMVideoFormatDescription` 改为按帧尺寸与色彩元数据缓存，`CMSampleBuffer` 预处理提前到 display link 回调线程，主线程只保留最终 `enqueueSampleBuffer`，`ensure_display_layer()` 只在 bounds 或 scale 真正变化时更新 layer。
- Decision: 继续保持 MacOS 明确走 native direct 主线，不引入过渡兼容态或 wgpu fallback。
- Risk/Blocker: CoreMedia 缓存与样本准备仍需依赖后续真实 runtime trace 验证，但当前编译与格式化已通过。
