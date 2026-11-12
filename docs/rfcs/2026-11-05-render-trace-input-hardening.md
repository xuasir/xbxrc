# 渲染、日志与输入七项问题修复

## Status

- Current State: completed
- Owner: 主窗口直接执行
- Authorization: 用户在逐项代码审查后明确“一口气解决这些问题”，授权七项修复与验证。
- Last Updated: 2026-11-05

## Scope and Decisions

保持main Browser稳定线、既有共享帧源/FramePacer/几何导航、Rust日志服务与单一writer线程。当前工作区的渲染/输入延迟优化作为本轮基线保留。

1. 普通/SR renderer最终销毁时，在取消回调、解绑context事件、释放GPU对象和timer后主动loseContext；context丢失/恢复路径保留自身生命周期。
2. 日志采用有界通道、8192行/8MiB硬预算、64KiB单行序列化预算；Debug/Raw使用6144行/6MiB，Key保留到7936行/7.75MiB，Essential使用剩余容量。入队采用try_send，所有丢弃分级计数。
3. writer独占文件创建、预算轮转、清理、flush和序号分配。配置指令与业务行按代次排队；启动/显式配置确认接收writer回执，日常记录保持有界序列化与非阻塞入队。诊断沿schema v3和traceBudgetNotice扩展关键/必要/超大/配置丢弃计数。
4. 输入保留字段统一little-endian，当前零值报文保持相同字节。
5. Pointer在预算计算前过滤有效事件并形成快照，编码全部帧，计数与实际记录一一对应；验证后续Mouse/Keyboard偏移和uint8容量边界。
6. renderer初始化期间只更新配置，初始化完成后启动单一回调链；重复init复用同一promise，暂停/销毁/迟到回调都检查请求归属。
7. setup初始化失败保留日志/弹窗，显式请求退出码1并向Tauri返回成功完成setup，使事件循环执行受控退出；try_state退出清理及macOS关闭窗口保护覆盖服务未装配状态。

## Validation

定向覆盖初始化中FPS更新、单链/迟到回调、销毁context、Pointer多帧/无效target/后续报告偏移；日志并发硬预算、优先级余量、慢writer、序号/轮转、模式切换/失败回滚、notice与退出排空。执行cargo fmt、Rust测试/检查、pnpm lint:fix/test/typecheck/build、既有56组生产供帧回放、diff check。实机启动/串流与Windows/ROG长测继续按仓库约定由用户验收。

## Progress

- [x] 七项审查、用户授权与实现合同。
- [x] 实现与定向回归；补齐编码容量上限、丢弃等级/原因成对计数、文件写失败容量记账及发布配置回滚。
- [x] 全量验证、证据与报告。前端59文件609项测试、Rust58项测试、lint/typecheck/build、cargo fmt/check、56组供帧回放、schema v3样本及diff check通过。

交付见[报告](../reports/2026-11-05-render-trace-input-hardening.md)与[本轮证据](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-05-render-trace-input-hardening.metrics.json)。实机启动、串流、退出码与Windows/ROG长测沿仓库约定由用户验收。
