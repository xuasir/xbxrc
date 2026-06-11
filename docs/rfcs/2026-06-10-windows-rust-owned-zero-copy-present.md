# Windows Rust-Owned Zero-Copy Present RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 未完成
- Current State: implementation landed, pending Windows validation
- Owner: agent
- Last Updated: 2026-06-10

## Background

- Windows `rust-owned` 当前主路径为 FFmpeg D3D11VA 解码后导出 D3D11 shared handle，再由 WGPU/Vulkan external memory 导入并 present。
- 该链路同时跨 D3D11、DXGI shared handle、Vulkan external memory、WGPU surface、Tauri 主线程 present，现场表现为无画面与窗口卡死。
- 用户明确要求修复目标为直通 0 CPU 拷贝。

## Goal

- Windows Rust 串流主路径保持 GPU zero-copy：远端 H264 decode 输出停留在 GPU 纹理，不落回 BGRA/NV12 CPU 内存。
- 上屏链路从 D3D11VA 解码纹理直接进入 Windows 原生 GPU presenter。
- 避免把 GPU 初始化、纹理导入和 present 长耗时操作压到 Tauri 主窗口事件循环。
- 保留现有 `ScheduledFrameSlot + HostCadenceTelemetry`、host mailbox、hostTiming 诊断口径。

## Scope

- In scope:
  - `crates/xbxengine/core/src/media/video/decode/backend_ffmpeg_windows_d3d11va.rs`
  - `src-tauri/src/mods/native_video/d3d11_presenter.rs`
  - `src-tauri/src/mods/native_video/presenters.rs`
  - `src-tauri/src/mods/native_video/wgpu_renderer.rs`
  - `src-tauri/src/mods/native_video/native_video_policy.rs`
  - `src-tauri/src/mods/native_video/types.rs`
  - Windows native video diagnostics / hostTiming
- Out of scope:
  - Windows 软件解码 fallback 作为主路径
  - CPU copyback 作为正常播放路径
  - 浏览器 `webrtc-direct` 路径
  - macOS `AVSampleBufferDisplayLayer` 直出路径
  - 新 transport / signaling / media recovery 路线

## Decision

- Windows 0-copy 主线改为 D3D11 native presenter。
- D3D11VA decoder 输出 `WindowsD3d11TextureDescriptor`，presenter 直接消费 `ID3D11Texture2D + array_slice + DXGI_FORMAT_NV12`。
- WGPU/Vulkan external memory import 保留为非默认实验路径，Windows 默认播放路径走 D3D11 compositor。
- CPU copyback 仅作为显式诊断开关或错误报告证据，默认播放路径不自动吞掉错误并回落 CPU。

## Plan

1. 定义 Windows D3D11 zero-copy 合同：descriptor 必须携带 `ID3D11Texture2D` 生命周期、array slice、DXGI format、颜色元数据、decoder/presenter interop mode。
2. 新增 `WindowsD3d11Presenter`：在独立视频窗口或当前 native video window 上创建 D3D11 device/swapchain 或 DirectComposition target，按 host cadence 从 `ScheduledFrameSlot` 取最新帧。
3. 实现 NV12 GPU shader path：D3D11 pixel shader 分别采样 Y/UV plane，按 descriptor 色彩矩阵/range/chroma location 输出到 swapchain。
4. 移除 Windows 默认 WGPU/Vulkan import 路由：policy 将 `WindowsD3d11TextureDescriptor` 路由到 `PlatformNative` D3D11 presenter。
5. 收紧失败语义：D3D11 descriptor export、presenter 初始化、shader bind、present 失败全部写结构化诊断；默认不做 CPU copyback 隐式降级。
6. 调整线程模型：GPU 资源创建和 render loop 放在 Windows presenter 专用线程；Tauri 主线程只承担窗口句柄获取与必要生命周期事件。
7. 补齐回归测试：策略路由、zero-copy descriptor 保留、CPU fallback 禁用、host mailbox 诊断、presenter failure 诊断。

## Validation

- [x] `cargo fmt`
- [x] `cargo test -p xbxengine windows_d3d11va --lib`
- [x] `cargo test -p xbxrc --lib native_video`
- [x] `cargo check -p xbxengine`
- [x] `cargo check -p xbxrc`
- [x] `git diff --check`
- [ ] `cargo xwin check -p xbxrc --target x86_64-pc-windows-msvc`
- [ ] Windows 实机验证：播放期 `descriptorUploadMode=d3d11-native`，CPU copyback count 为 0
- [ ] Windows 实机验证：首帧可见，应用窗口响应，present cadence 推进
- [ ] Windows 实机验证：1080p/60fps 下 decode/present 长尾、host mailbox、DisplayStable 正常

## Risks

- D3D11VA decoder 与 presenter 使用不同 D3D11 device 时，需要共享 handle 或前移 decoder device ownership，优先方案是统一 device ownership。
- Tauri WebView 与 D3D11 swapchain/DirectComposition 的窗口层级需要实机验证，独立 native video window 是更稳的承载点。
- WGPU effect pipeline 后续需要 Windows D3D11 native effect 分支，当前任务先保障 0-copy 播放主路径。
- 当前开发环境缺 Windows target 工具链与实机 GPU 验证，代码级检查只能覆盖非 Windows 编译面和共享逻辑。

## Progress

- [x] Step 1: 固定 Windows zero-copy descriptor 合同。
- [x] Step 2: 新增 D3D11 presenter 与 shader path。
- [x] Step 3: 调整 policy 默认路由。
- [x] Step 4: 收紧失败诊断与 CPU fallback 策略。
- [ ] Step 5: 补测试与验证。

## Execution Notes

- Date: 2026-06-10 | Status: planned
- Update: 根据现场症状和用户 0 CPU 拷贝要求，确定 Windows 修复方向为 D3D11VA 解码纹理直连 D3D11 native presenter。
- Decision: Windows 默认路径从 WGPU/Vulkan external memory import 切到 D3D11 native presenter；CPU copyback 退出默认播放路径。
- Risk/Blocker: 需要 Windows 实机验证 DXGI/DirectComposition/swapchain 行为，本机环境只能先完成静态与共享测试验证。

- Date: 2026-06-10 | Status: implementation landed
- Update: 新增 `WindowsD3d11Presenter`，使用独立 host render loop、D3D11 device + DXGI swapchain、`OpenSharedResource1` 打开 decoder NT shared handle，并用 `CreateShaderResourceView1` 创建 NV12 plane SRV；像素着色器在 GPU 上完成 NV12->RGB 后 present 到 swapchain。
- Update: Windows `WindowsD3d11Texture` policy 现在路由到 `NativeDirect + Noop`，factory 在 Windows `PlatformNative` 时创建 D3D11 presenter；CPU surface 继续走 WGPU/effect 路径，D3D11 native presenter 对 CPU surface 和非 Windows descriptor 显式拒收。
- Update: host mailbox / retained redraw / submit gap / presented frame 诊断继续沿用现有 `ScheduledFrameSlot + HostCadenceTelemetry`，D3D11 presenter 诊断暴露 `descriptorUploadMode=d3d11-native`，CPU upload count 保持 0。
- Validation: `cargo fmt`、`cargo test -p xbxrc --lib native_video`（48 passed）、`cargo check -p xbxrc`、`cargo check -p xbxengine`、`cargo test -p xbxengine windows_d3d11va --lib`（0 filtered tests, command passed）、`git diff --check`。
- Blocker: `cargo check -p xbxrc --target x86_64-pc-windows-msvc` 仍停在本机 Windows C 头文件缺失；`cargo xwin check -p xbxrc --target x86_64-pc-windows-msvc` 仍停在 SDL3/Opus 的 CMake/Ninja 交叉构建环境，尚未编译到新增 Windows-only D3D11 文件。
