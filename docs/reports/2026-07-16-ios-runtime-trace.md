# iOS 纯 Swift Runtime Trace Report

> 说明：本 Report 记录 iosapp 独立 Runtime Trace 闭环及配套 JSONL 分析 skill 的交付结果。

## Summary

- Related RFC: [`docs/rfcs/2026-07-16-ios-runtime-trace.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-07-16-ios-runtime-trace.md)
- iosapp 已建立 Swift 自主管理的 schema v3 JSONL trace，Rust bridge 继续保持业务接口。
- App、认证、Cloud Access、缓存、目录、metadata、Xbox data、图片、UI、详情和游玩链路已接入结构化事件。
- 已建立纯 Python `analyze-ios-runtime-trace` skill，用于分析真实落盘 JSONL。

## Delivered

- Swift writer：profile、预算、批量 flush、队列压力、轮转、裁剪、导出、清理、OSLog 镜像和递归脱敏。
- 业务覆盖：13 个 Swift 文件、139 个 trace 调用，覆盖游戏库故障定位所需主链路。
- 文件预算：production 8 MiB×4，dev 32 MiB×6，off 关闭写盘；4096 pending rows，40ms/128 行 flush。
- 诊断 UI：账户页可切换 profile、导出当前/全部 trace、清理历史文件；登录失败状态下仍可使用。
- Python skill：schema、session seq、物理文件预算、保留数量、关键流程、operationId 配对、损坏行和敏感数据门禁。

## Changes

- 新增 `iosapp/XBXRC/Shared/Diagnostics/*` 四个 Swift 文件并登记 Xcode 工程。
- 在 App 生命周期、Swift UniFFI boundary、数据 Store、缓存 Repository、图片组件和游戏库页面接入 trace。
- 修复 writer 轮转时 `fileOpened` 与业务行的 seq 反转问题，并按时间戳/fileId 数值排序文件。
- 新增 `.agents/skills/analyze-ios-runtime-trace`，脚本保持纯 Python 和 JSONL 单一职责。

## Validation

- iOS Device App + XCTest target `build-for-testing`：通过。
- Rust 相关测试：36 项通过；`cargo check -p xbxrc`：通过。
- `cargo fmt --all`、Swift parse、`git diff --check`：通过。
- skill `quick_validate.py`：通过。
- Python analyzer 黑盒测试：3 项通过。
- Simulator 真实启动 trace：由用户运行 App 后导出，再执行：

  ```bash
  python3 -B .agents/skills/analyze-ios-runtime-trace/scripts/analyze_ios_runtime_trace.py <trace-or-directory> --strict --require-flow all --pretty
  ```

## Risks

- 首次真实运行的事件组合取决于登录态、缓存状态和网络分支；skill 将条件分支与关键必需锚点分开判断。
- “导出全部”文件是多个轮转文件的聚合体，其物理大小不使用单文件预算；预算验收优先分析原始 `runtime-trace-ios-*` 目录。

## Follow-up

- 用户运行 Simulator 或真机并导出首份真实 trace。
- 使用 Python skill 对登录、缓存、目录、首屏和图片回退时间线执行严格门禁。
