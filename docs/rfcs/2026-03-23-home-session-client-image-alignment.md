# Home Session Client Image Alignment RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 已完成
- Current State: completed
- Owner: TBD
- Last Updated: 2026-04-09

## Background

- 最新 xHome 失败 trace 已确认：本地 recreate 已触发，但服务端在旧 session cleanup 未收敛时复用了同一个 `Provisioning` session。
- 对比 `参比实现` 后发现，两侧在 xHome 下最大的可疑差异不在 wake/Provisioning 时序，而在会话创建请求画像：`clientAppId`、浏览器版本、显示尺寸以及是否显式携带 `User-Agent` 都不同。

## Goal

- 将我方 xHome `/v5/sessions/home/play` 请求画像向 `参比实现` 做最小对齐。
- 先验证服务端是否会因为更接近官方 web 客户端画像而减少 `Provisioning` 粘滞 session / reused session 问题。

## Scope

- In scope:
  - `crates/xbox-streaming/src/policy/session/compiler.rs`
  - xHome 会话 headers / `x-ms-device-info` 画像编译
  - 定向回归测试
- Out of scope:
  - RTC / ICE / 视频链路
  - 非 home 场景画像调整
  - 更大范围的启动重试策略调整

## Plan

1. 收敛 xHome 与参比实现的画像差异。
2. 实施最小 xHome-only 画像对齐。
3. 补测试并完成编译验证。

## Validation

- [x] `cargo fmt -p xbox-streaming`
- [x] `cargo test -p xbox-streaming home_session_headers_include_user_agent_and_follow_home_resolution --lib -- --nocapture`
- [x] `cargo test -p xbox-streaming cloud_session_headers_keep_custom_image -- --nocapture`
- [x] `cargo check -p xbox-streaming`

## Risks

- 服务端 sticky session 行为也可能与主机注册状态有关，仅靠画像对齐不一定彻底解决。
- 去掉 xHome 显式 `User-Agent` 后，若服务端还有其他分流规则，可能引入新的兼容性差异。

## Progress

- [x] Step 1: 已确认 xHome 下最可疑的差异点是 `clientAppId`、browserVersion、displayInfo 与 `User-Agent`。
- [x] Step 2: 已实施最小 xHome-only 画像对齐。
- [x] Step 3: 已补测试并完成验证。

## Execution Notes

- Date: 2026-03-23 | Status: done
- Update: 本轮先按最小实验原则，只改 xHome 会话创建画像，不碰 RTC/视频或更大握手策略。
- Decision: 优先对齐 `参比实现` 的 `www.xbox.com` 客户端画像，并保留 cloud 现有画像不动。
- Update: `compile_session()` 现按 target 分流画像编译：home 对齐 `www.xbox.com` / `130.0` / `1920x1080`，且不再显式发送 `User-Agent`；cloud 保持原有 `com.xuasir.xbxrc` / `140.0.3485.54` / `4096x2160` 与设备型 `User-Agent`。
- Validation: 两条定向单测与 `cargo check -p xbox-streaming` 已通过。
- Risk/Blocker: 若对齐后仍复用旧 session，再继续回到更严格的 home ready 判定或服务端 sticky-session 规避策略。
