# iOS 与 PC HTTP 请求链路差异审计 RFC

## Status

- Completion: 完成；UTC2026-11-24 07:09:23快照只读审计，后续业务修复未授权
- Current State: done
- Owner: 主窗口直接执行
- Last Updated: 2026-11-24

## Background

用户观察到 PC 比 iOS 稳定很多，要求先整理 iOS 所有 HTTP 请求与 PC 的差异，覆盖发起、错误处理等完整链路。前一份 iOS 登录诊断含 7 次 beginLogin 失败（5 次快速 network、2 次 timeout），但缺少初始握手的逐请求分类，尚不能指认底层网络根因。

近期启动对齐已经接入共享认证缓存与续期，必须以当前源码为准，不能继续把旧版“iOS 每次全量认证”当作现状，也不能把双方共享的 transport 默认值误称为 iOS 独有问题。

## Goal

- 建立 iOS 所有应用自有 HTTP 发起边界与 PC 对应入口清单。
- 区分共享实现、装配差异、Swift/Tauri 包装差异和平台网络条件。
- 覆盖 client 配置/复用、TLS/代理、连接/请求预算、重试、并发/取消、缓存、凭据恢复、错误投影和 trace。
- 给出有源码锚点的差异矩阵、稳定性影响判断、未证实假设与后续修复优先级。

## Scope

- In scope:
  - iOS Swift 应用与 UniFFI Rust bridge。
  - 共同 AuthFlow、WebAPI、Cloud catalog、Streaming HTTP 控制与资源下载。
  - 当前 main PC 的 Tauri Rust 服务、Vue/TypeScript 请求发起与结果消费。
  - 图片、相册资源、触屏包、平台管理的网页登录与桌面更新。
  - 同目标平台的 Cargo 特征解析（可用时，仅离线解析，不构建）。
- Out of scope:
  - 业务代码修改、扩大超时、重试策略上线或第二套 transport。
  - WebRTC 媒体包、UDP/TURN 数据传输与原生媒体实现优化。
  - 没有同机/同网/同包运行证据时宣称已证明 DNS/TLS/VPN 或服务端根因。
  - 不同开发线或历史已废弃实现作为当前 PC 对照。

## Plan

1. 扫描直接 HTTP 和平台托管请求入口，映射共享 Rust API 与两端包装。
2. 对照配置、预算、重试、client 生命周期、并发和取消。
3. 对照凭据/缓存策略、类型化错误、恢复动作与日志保真。
4. 输出全量边界清单和差异矩阵，独立核对源码锚点、清理本轮临时产物。

## Validation

- [x] 直接发起入口扫描、共享API方法清单和两端调用覆盖（快照范围，不等于实际请求次数）。
- [x] 构造配置、5/12/18秒预算及其他外层deadline按调用路径核验。
- [x] 错误与恢复映射不混淆网络、认证、取消、限流和无效响应。
- [x] 区分源码可证事实、历史已修复行为和实机待证假设。
- [x] 本任务业务代码0修改；文件引用与文档差异格式核验，续接指纹失败保留。
- [x] 最终报告与验证/失败诊断归档；旧r1及当前r2临时目录清理，原用户日志保留。

## Risks

- iOS runtime 仅标识版本/build，没有源码 commit，不能确认现有设备包与当前工作树一致。
- PC 稳定性是用户现场观察，目前没有同环境配对请求 trace，不能用静态审计代替网络因果实验。
- 工作树存在大量用户修改，只做局部任务文档更新，不整理、恢复或提交无关改动。

## Progress

- [x] 入口清单与共享/独立边界。
- [x] transport、请求生命周期与预算矩阵。
- [x] 缓存、错误处理与诊断矩阵。
- [x] 报告、验证和临时清理。

## Execution Notes

- Date: 2026-11-24 | Status: done
- Update: 15provider/55公开方法、12Swift托管调用清单完成；原快照18源码断言。续接两目标4条locked离线依赖解析通过，13项PASS/3项工作树指纹FAIL及快照后9来源变化原样归档；不声称当前源码全量PASS。两个任务临时目录、25文件/约0.95MiB已清理。
- Delivered: [最终Report](/Users/guo.xu/Documents/code/games/xbxrc/docs/reports/2026-11-24-ios-pc-http-parity-audit.md) / [差异矩阵与证据](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-24-ios-pc-http-parity-audit/README.md)。
- Decision: 后续业务修复须另行确认具体差异与运行验证目标；本轮未改网络、状态或技术栈合同。
- Risk/Blocker: 固定快照审计已交付，无本轮执行阻塞。运行包配对、实际出口及移动端网络根因仍需后续运行证据；快照后源码验收归属相应任务。
