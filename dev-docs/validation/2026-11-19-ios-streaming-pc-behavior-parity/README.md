# iOS 串流行为与设置消费对齐 PC 验收证据

M0～M6实施与自动验收已交付；任务保持`Validation Pending`，实机验收责任人为用户。[RFC](../../../docs/rfcs/2026-11-19-ios-streaming-pc-behavior-parity.md)与[Report](../../../docs/reports/2026-11-19-ios-streaming-pc-behavior-parity.md)记录合同、交付和剩余验收范围。

## 自动验收

| 范围 | 结果 | 原始证据 |
| --- | --- | --- |
| Rust三个crate | 253 passed | [日志](diagnostics/rust-tests.log) |
| PC受影响5个文件 | 94 passed | [日志](diagnostics/pc-tests.log) |
| iPhone 17e | 178单元＋2 UI passed | [Xcode摘要](phone-summary.json)、[日志](diagnostics/phone-tests.log) |
| iPad Pro 13-inch (M5) | 178单元＋2 UI passed | [Xcode摘要](ipad-summary.json)、[日志](diagnostics/ipad-tests.log) |
| iPhoneOS设备目标 | BUILD SUCCEEDED，关闭签名 | [日志](diagnostics/device-build.log) |
| PC构建与检查 | lint/typecheck/build、Tauri check通过 | [lint](diagnostics/lint.log)、[typecheck](diagnostics/typecheck.log)、[构建](diagnostics/pc-build.log)、[Tauri](diagnostics/tauri-check.log) |
| UniFFI与会话边界 | 生成与边界检查通过 | [bindings](diagnostics/bindings.log)、[边界脚本](../../../iosapp/scripts/check-streaming-session-boundary.sh) |

[测试摘要](test-summary.json)明确测试选择和范围。两项资料模块旧断言以`-skip-testing`从最终指定回归排除，分别涉及四源资料刷新计数和分区失败提示；[归属说明](baseline-failures.json)与[实际错误行](diagnostics/profile-test-failures.txt)已保留。早期本地HTTP测试受到沙箱listen权限限制，最终环境重跑253项通过；[诊断](diagnostics/sandbox-http-test-failure.txt)关联最终Rust日志。

## 相同事实与设置消费

[PC/Rust基准](shared-supervision-baseline.json)包含decode-pressure、decode-gap、quality-improved、quality-unsupported、quality-supply-loss、rtt-growth六组186条事实。[PC回放测试](../../../src/streaming/runtime/shared-supervision-baseline.test.ts)与[Rust回放测试](../../../crates/xbox-streaming/src/session/recovery/supervision.test.rs)共同消费冻结基准；最终Rust日志包含该回放通过结果。

[19组设置消费者](settings-consumer-matrix.json)逐项列出生效时机、生产入口、自动证据和实机观察点。分辨率/codec/码率边界、旧值/新会话快照、常规Metadata、输入映射/归属、麦克风串行与Peer代次、初始静音和idle timer恢复已核对到实际调用。

[关键实现哈希](implementation-hashes.json)与续接时源码一致。[81份源码快照](validation-source-manifest.json)记录最终测试对应的源文件哈希和修改时间。[归档清单](archive-manifest.json)记录日志和截图来源及哈希。

## 最终界面证据

- 手机：[PC分辨率与比例选项](phone/settings-PC-options.png)、[手柄图示内联录入](phone/controller-inline-recording.png)。
- iPad：[PC分辨率与比例选项](ipad/settings-PC-options.png)、[手柄图示内联录入](ipad/controller-inline-recording.png)。
- [截图清单](screenshot-manifest.json)关联设备、XCUITest标识和采集时间；[手机采样回放](phone/sampling-pressure-replay.json)、[iPad采样回放](ipad/sampling-pressure-replay.json)保留短按边沿及高频轴采样结果。

## 模拟器 trace 门

手机6个dev原始文件共2802行，iPad6个dev原始文件共4573行。两组schema、序列、配对、脱敏、物理文件大小与留存门均PASS，pressure notice为0；配对检查分别覆盖28/42个起始事件。[手机门报告](phone-trace-gate.json)、[iPad门报告](ipad-trace-gate.json)与原始文件目录[手机](traces/phone)/[iPad](traces/ipad)已归档。

这组trace覆盖模拟器夹具和设置入口，使用`--require-flow none`。真实串流的网络路径、音频上行和六维媒体表现按照[实机验收清单](device-acceptance.md)继续采集。

```sh
python3 -B .agents/skills/analyze-ios-runtime-trace/scripts/analyze_ios_runtime_trace.py dev-docs/validation/2026-11-19-ios-streaming-pc-behavior-parity/traces/phone --strict --require-flow none
python3 -B .agents/skills/analyze-ios-runtime-trace/scripts/analyze_ios_runtime_trace.py dev-docs/validation/2026-11-19-ios-streaming-pc-behavior-parity/traces/ipad --strict --require-flow none
```

## 临时产物收尾

本任务临时根目录`/private/tmp/xbxrc-ios-streaming-pc-behavior-parity`的M1/M4两轮已完成清理。相关构建、测试、导出和测试宿主均已结束；23份xcresult、独立DerivedData、克隆包、重复截图、日志、一次性脚本及本任务系统临时锁已回收，占用1.76 GiB，清理后可用空间60.27 GiB。必要证据保留在本目录，共享Rust和WebRTC缓存继续保留；[清理记录](cleanup.json)记录目标路径、产物分类与空间核验。
