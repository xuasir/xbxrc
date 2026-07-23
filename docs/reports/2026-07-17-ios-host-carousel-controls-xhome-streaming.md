# iOS 主机轮播控制与 xHome 串流 Report

## Summary

- Related RFC: [`docs/rfcs/2026-07-17-ios-host-carousel-controls-xhome-streaming.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-07-17-ios-host-carousel-controls-xhome-streaming.md)
- iOS 主机页已成为首位默认 Tab，使用电脑图标、共享圆弧轮播和桌面 Series X/S 图片。开机、关机、串流三个操作实时作用于当前选中主机，页面保持单层结构。

## Delivered

- 主机 Tab 调整为第一位和默认页，图标使用 `desktopcomputer`。
- 成就页圆弧轮播提取为共享 `OrbitCardCarousel`，主机页使用 `260×390` 主机卡片与 Series X/S 资产。
- 主机卡片负责轮播选择，旧主机详情页及导航入口已移除。
- 开机、关机按钮接入 Xbox Console API，包含 single-flight、执行态、失败反馈和成功刷新。
- 串流按钮接入 xHome access、home target 会话与现有 Swift libwebrtc 数据面。

## Changes

- Rust `xbox-ios-bridge` 新增 `power_on_console`、`power_off_console` 与 `prepare_home_access`；cloud/home 串流统一使用 `create_stream_session`，由 access handle 中的权威 target 编译会话 plan。
- home session 复用桌面 `SessionFlowService` 的会话创建、server registration 有界重试、SDP/ICE、keepalive 和 close 流程。
- Swift `XboxDataStore`、`AuthStore` 与 `StreamingFeatureStore` 增加当前主机电源操作和 home target 启动路径。
- 桌面 `series-x.png` 与 `series-s.jpeg` 逐字节复制到 iOS Asset Catalog。

## Validation

- `cargo fmt -p xbox-ios-bridge -- --check` 通过。
- `cargo test -p xbox-ios-bridge`：26 项通过。
- `xcodebuild ... -destination 'generic/platform=iOS' ... CODE_SIGNING_ALLOWED=NO build`：Device arm64 完整构建成功。
- `plutil` PBX lint、Asset Catalog JSON 解析、桌面/iOS 图片 `cmp`、UniFFI bindings API 与主机页源码门禁通过。
- `git diff --check` 与 `git diff --cached --check` 通过。

## Risks

- 真实 Xbox 账号、主机唤醒、xHome 服务注册、首帧与音频体验需要模拟器或真机运行验证。
- 主机状态来自 SmartGlass 刷新快照，电源命令成功后通过重新拉取主机列表收敛最终状态。

## Follow-up

- 用户在模拟器或真机验证轮播手势、当前选择切换、开关机反馈与 xHome 串流完整链路。
- 运行验证出现异常时导出 iOS Runtime Trace，使用现有 trace 分析链定位 auth、session、SDP/ICE 和首帧阶段。
