# Home Session Client Image Alignment Report

> 说明：本 Report 仅用于复杂任务完全完成后的最终总结，不用于记录执行中的阶段性进度；中间过程应持续回写到对应 RFC。

## Summary

- Related RFC: [`docs/rfcs/2026-03-23-home-session-client-image-alignment.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-03-23-home-session-client-image-alignment.md)
- 已完成 xHome 会话创建请求画像的最小对齐实验，用于验证更接近官方 web 客户端的画像是否能减少 sticky `Provisioning` / reused session。

## Delivered

- 将 xHome 的 `x-ms-device-info` 编译改成单独画像，不再复用 cloud 基线。
- 去掉 xHome 显式 `User-Agent`，保留 cloud 原有 `User-Agent` 行为。
- 补齐 home/cloud 两条定向回归测试，锁定本轮实验边界。

## Changes

- [`crates/xbox-streaming/src/policy/session/compiler.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbox-streaming/src/policy/session/compiler.rs) 现在按 `Target` 分流编译 `ms_device_info`、`x-ms-device-info` header 和 `User-Agent`。
- `Target::Home` 现对齐为 `clientAppId=www.xbox.com`、`browserVersion=130.0`、`displayInfo=1920x1080`，且不再显式发送 `User-Agent`。
- `Target::Cloud` 继续保留 `com.xuasir.xbxrc`、`140.0.3485.54`、`4096x2160` 和既有设备型 `User-Agent`。

## Validation

- `cargo fmt -p xbox-streaming`
- `cargo test -p xbox-streaming home_session_headers_include_user_agent_and_follow_home_resolution --lib -- --nocapture`
- `cargo test -p xbox-streaming cloud_session_headers_keep_custom_image -- --nocapture`
- `cargo check -p xbox-streaming`

## Risks

- 这轮只改 client image；若 sticky `Provisioning` 的主因仍是主机注册信号缺失，问题可能只会部分改善。
- 去掉 xHome 显式 `User-Agent` 后，若服务端还存在其它 UA 分流规则，可能会暴露新的兼容性差异。

## Follow-up

- 用下一份 xHome trace 对比 `sessionCreated`、`sessionRecreateCleanup` 和最终 `sessionId`，确认是否还会复用旧会话。
- 若 sticky `Provisioning` 仍未明显改善，继续回到 wake 后 ready 判定和服务端 session cleanup 时序。
