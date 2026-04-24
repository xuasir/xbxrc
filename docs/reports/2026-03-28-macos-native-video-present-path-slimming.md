# MacOS Native Video Present Path Slimming Report

## Summary

- Related RFC: [`docs/rfcs/2026-03-28-macos-native-video-present-path-slimming.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-03-28-macos-native-video-present-path-slimming.md)
- 本次已完整完成 macOS native present 主线的三项收敛：`CMVideoFormatDescription` 缓存、`CMSampleBuffer` 预处理前移、`ensure_display_layer()` 变化更新收紧。

## Delivered

- macOS native direct 路线继续作为唯一主线，没有引入过渡兼容分支或 wgpu fallback。
- `CMVideoFormatDescription` 已按帧尺寸与色彩元数据缓存，避免每帧重复创建 format description。
- `CMSampleBuffer` 已在 display link 回调侧提前准备，主线程只负责最终 `enqueueSampleBuffer`。

## Changes

- [`src-tauri/src/mods/native_video/mod.rs`](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/mods/native_video/mod.rs) 新增/收敛了 `FormatDescriptionCacheKey`、`CachedFormatDescription`、`PreparedLayerSample` 和 `CoreFoundationOwnedRef`，把 sample 预处理与 layer 提交拆开。
- `macos_layer_display_link_callback` 现在先完成 sample 预处理，再把最小提交任务派发到主线程，`run_on_main_thread_delay` 的基准也移到了预处理之后。
- `ensure_display_layer()` 只在 bounds 或 backing scale 变化时才更新 layer，避免稳定状态下每 tick 重设 `setContentsScale / setFrame / setNeedsLayout`。

## Validation

- `cargo check -p xbxrc`
- `cargo fmt -p xbxrc`
- 再次执行 `cargo check -p xbxrc` 通过

## Risks

- CoreMedia / AVFoundation 对象缓存与跨线程准备依赖严格的生命周期管理，后续仍应通过真实 runtime trace 验证稳定性。
- 如果后续视频流在分辨率或色彩元数据上频繁切换，format description cache 会更频繁失效，但逻辑上应仍然正确。

## Follow-up

- 用下一份会出现渲染/播放压力的 runtime trace 验证 main-thread `tick_total` 是否下降。
- 如仍有偶发 stall，再考虑继续观察 prepare 阶段是否需要补更细的 timing 埋点。
