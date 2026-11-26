# iOS 规格迁移 Android 的评估证据

日期：2026-11-26；工作区：`release/test` / `a98e5457c8f9`。

本轮交付为[可行性评估](../../../docs/isu/2026-11-26-ios-to-android-feasibility.md)。核对共享 Rust/UniFFI、当前 iOS 规格、WebRTC/Metal、音频与后台续接、控制输入和最新 TAK 公共 IR 的源码与历史证据。

## 最小验证范围

- 静态核对：现有移动 bridge 的依赖/产物、Swift 专用生成入口、UniFFI 0.32 Kotlin 后端，以及 Android 工程的新增范围。
- 规格核对：五根页与数据行为、设置实际消费、输入/释放、后台生命周期、诊断观测来源与 iOS 实机待验项。
- 一次文档冒烟：评估的六个主要部分、仓库内链接、来源文件指纹、文档差异，以及既有变更列表的保持情况。

构建和实机运行列入候选任务。此前 Rust/XCTest/UI/设备编译结果按相关报告引用，当前轮次的验证结果见 [assessment-evidence.json](assessment-evidence.json)。

## 产物与清理

本轮产物为评估文档、本目录 README 与 JSON 证据。执行采用内存脚本，临时文件、DerivedData、xcresult、截图及日志的创建数量为零；保留项为上述三份最终文档证据。

现有 iOS/TAK/音频等工作区修改继续由原任务维护。本轮证据记录来源文件 SHA-256 和验证时的变化状态，以便后续执行建立清晰基线。
