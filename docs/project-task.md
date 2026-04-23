# Project Tasks

## In Progress

- 2026-04-22: 推进双窗口串流视频宿主改造，拆分主窗口透明 UI 与独立 native video 承载窗口。RFC: [docs/rfcs/2026-04-22-dual-window-stream-video-host.md](D:\Code\xbxrc\docs\rfcs\2026-04-22-dual-window-stream-video-host.md)
- 2026-04-23: 规划视频中后段显示调度收敛改造，目标是完成 P1 latest-only render/runtime 交付与 P2 host 单点最终显示时钟收敛，消除 core/host 双重调度与显示进展语义错位。RFC: [docs/rfcs/2026-04-23-video-display-scheduling-convergence.md](D:\Code\xbxrc\docs\rfcs\2026-04-23-video-display-scheduling-convergence.md)

## Recent Completed

- 2026-04-23: 修复前端 `gamepad-listener` 的重复触发与策略切换状态机，按键 repeat 改为独立定时器驱动，并在 `stream-only` 策略切换及 listener 停止时统一清理 pressed/combo/timer 状态，避免手柄输入卡键与重复触发残留。
- 2026-04-23: 完成双阶段恢复链路改造（single media commit + display settle），clean anchor 提交改为首个可服务 IDR 完成即提交，ramp guard 拆分 acknowledge/close，并在 picture recovery episode 增加 `firstDecodeToCleanAnchorCommittedMs`、`cleanAnchorCommittedToDisplayStableMs` 两段尾延迟 trace。RFC: [docs/rfcs/2026-04-23-dual-phase-recovery-chain.md](D:\Code\xbxrc\docs\rfcs\2026-04-23-dual-phase-recovery-chain.md)
- 2026-04-23: 去除 gamepad 配置类 RPC 的重复事件转发，改为由 shell 后台订阅桥统一广播 `runtimeSnapshot/slotSnapshot/devicesChanged`，避免一次配置变更触发双份手柄事件。
- 2026-04-23: 修复手柄采样节流语义错位，收敛 `sampleSeq` 为真正的流输入变化令牌，并在 `ohmygamepad` runtime 落地 `uiPushRateHz` 与 `streamPushMode/streamPushRateHz`，避免空闲状态持续推送与 UI 事件风暴。
- 2026-04-23: 调整 Windows Tauri 窗口手柄暂停策略，不再因 `Focused(false)` 直接 suspend gamepad，改为在窗口最小化时再暂停，以避免 Xbox Full Screen Experience 下手柄输入被系统焦点切换误杀。
- 2026-04-22: 修正 Windows 播放器策略对 `D3D11 texture` 的错误路由，避免 `GpuDirect presenter` 误绑定仅支持 CPU surface 的 `Wgpu effect pipeline`，导致 `present_frame` 在 `can_process(native_handle)=false` 处提前返回、宿主 render loop 空转且始终黑屏。
- 2026-04-22: 修正 Windows `ffmpeg-d3d11va` 硬解输入路径，停止将 Annex-B access unit 重打为 AVCC，改为直接喂完整 Annex-B，并移除该后端上的 `AV_CODEC_FLAG2_CHUNKS`，避免首个 IDR 被硬解静默吞掉后长期无输出。
- 2026-04-22: 新增 RFC [docs/rfcs/2026-04-22-rtc-recovery-boundary-convergence.md]，规划 RTC 恢复系统的收边界、减解释与高置信快速决策路径收敛方案。
