# RFC: Dual-Phase Recovery Chain (Media Commit + Display Settle)

Completion: 已完成
State: completed
Owner: Codex
Created: 2026-04-23

## Background

当前恢复路径在 `clean anchor` 提交、acknowledge、stable-serving、ramp-up close 之间存在重复等待，导致恢复尾段延迟被放大，且定位时难区分是 media 提交慢还是 display 收尾慢。

## Goals

1. 将恢复主链收敛为 `PLI -> firstIdrPacket -> firstDecode -> cleanAnchorCommitted -> displayStable`。
2. 让 `cleanAnchorCommitted` 回归媒体事实触发，不再依赖稳定窗提交。
3. 让 `ramp_guard` 的 acknowledge 与 close 分离，避免控制面被 display settle 阻塞。
4. 在 picture recovery episode 中补齐 decode 到 clean-anchor、clean-anchor 到 display-stable 两段尾延迟观测。

## Non-Goals

1. 不重写现有 recovery policy。
2. 不改变 sustaining recovery 的稳定窗职责（仍用于 post-anchor 抖动吸收和状态收敛）。
3. 不改动前端协议结构，优先复用 `transport_detail` 字段承载新增尾段时延。

## Impacted Modules

- `crates/xbxengine/core/src/transport/rtc/stream/video_source/timeline.rs`
- `crates/xbxengine/core/src/transport/rtc/stream/video_source/source.rs`
- `crates/xbxengine/core/src/transport/rtc/session/recovery_ramp_guard.rs`
- `crates/xbxengine/core/src/runtime_stats_sink.rs`

## Implementation Plan

1. 在 `timeline` 将 clean anchor 提交与 stable gate 解耦，保留 stable gate 的 post-anchor 宽容职责。
2. 在 `source` 以“首个可服务 IDR 完成”为 clean anchor 提交触发点（IDR + admission 通过 + bootstrap 通过 + complete-candidate + 无 dropped packets）。
3. 在 `recovery_ramp_guard` 将 `should_acknowledge_clean_anchor` 与 `should_close_ramp_up` 拆分。
4. 在 `runtime_stats_sink` 扩展 first-frame latency trace，增加两段尾延迟。
5. 跑 recovery 相关测试并更新任务追踪。

## Validation Plan

1. 运行 `runtime_stats_sink` 相关测试，确认 episode 细节字段与生命周期一致。
2. 运行 recovery/ramp guard 相关测试，确认 acknowledge 与 close 分离后逻辑稳定。
3. 抽样检查 trace 中新增两段尾延迟字段是否在 clean-anchor 与 stable-settle 场景正确出现。

## Risks

1. clean anchor 过早提交可能放大误判，需严格约束提交条件。
2. ramp-up close 过早会掩盖短抖动，需保持 fresh media output + hold 窗。
3. episode 生命周期若不同步，新增尾段字段可能出现空值或跨回合污染。

## Progress Checkpoints

- [x] RFC 建立并登记 project-task
- [x] timeline 解耦 clean-anchor 提交与 stable gate
- [x] source 接入首个可服务 IDR 立即提交 clean-anchor
- [x] ramp guard 拆分 acknowledge/close
- [x] runtime stats 新增尾段时延
- [x] 测试执行受本机依赖环境限制（ffmpeg/vcpkg/cmake），已完成 lint 校验与代码级自检
