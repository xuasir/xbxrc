# Browser 最新候选准入修复

默认60fps渲染路径已改为每个有效新候选即时提交，固定起点与时间槽状态已删除。低于60fps的显式预算使用滚动准入时间；普通锐化和SR、主机和云游戏继续共享FramePacer。19组设备入口时序全部准入，两组曾出现损失的相位抖动模拟供给也全部提交。

## 触发证据与范围

[14:42设备日志分析](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-05-latest-frame-scheduling-fresh-trace-analysis.md)记录首段SR累计352候选、268次提交、84次限帧跳过，占23.86%；同代际即时53.14→40.19fps。第二段SR即时167→167、55.43fps，留下video→candidate的独立供给缺口。用户随后授权“优化这部分”，本次按[更新后的RFC合同](../rfcs/2026-11-05-browser-latest-frame-scheduling.md)直接修复局部准入。

设备源文件在后续操作中已被清理；本次回放读取此前保留的[完整快照](/tmp/xbxrc-2026-11-05-latest-scheduling-trace.jsonl)，SHA-256与设备报告一致：`c83a7568a8b8e520f962943205ec030a2ee5922e39e5e6473813c5c5e4b0f038`。回放只保存必要计数与入口时间，原始日志留在本地临时目录。

## 实现

- 60fps及以上常规预算直接放行有效新候选。相位移动、毫秒取整、短间隔候选和迟到到达均沿同一即时绘制路径；源身份去重仍由VideoFrameSource负责。
- 低于60fps的显式预算仅保留目标帧率和下一次准入时间。1ms容差吸收量化边界，周期内保留剩余额度以均匀抽帧；迟到超过一个周期后重新锚定当前时刻，只给当前候选一次新额度。暂停与预算变化重置准入时间，重复预算投影保持已有额度。
- 默认候选突发会依次处理已到达的有效最新图像。显式低预算的超额候选直接跳过，后续候选重新判断；renderer定时重试为0。
- SR context恢复继续订阅后续新候选；首次装配仍绘制已有最新缓存。此前由同槽限帧掩盖的恢复缓存重复提交已一并修复，context恢复回归的duplicateDrawCount与rateLimitDropCount均0。

生产变更集中在[FramePacer](../../src/player/infra/render/frame-pacer.ts)与[SR恢复入口](../../src/player/infra/render/SuperResolutionWebGL2Renderer.ts)。网络、解码、输入和像素诊断沿现有实现继续运行。

## 验证

全量前端60文件610项测试通过，准入与两种renderer定向108项通过。`pnpm lint:fix`、`pnpm typecheck`、`pnpm build`（313模块）及staged/unstaged diff check通过。测试覆盖新候选即时提交、相位移动、53/60fps供给、真实入口时间、源身份去重、显式预算、短间隔突发、暂停/截图与context/关闭生命周期。

[生产回放](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-05-browser-frame-admission-audit.cjs)转译当前类与[固定槽源码快照](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-05-frame-pacer-fixed-slot.snapshot.ts)。基线源码SHA `92e48a020eb7fedc993d91e4c16485471e70b3463d4decf61a7e18f414d0343e`与原设备metrics绑定一致；原日期化报告与metrics继续保留采样时身份。[新指标](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-05-browser-frame-admission.metrics.json)绑定本轮测试报告SHA、当前生产源码SHA与基线SHA。

| 同一输入时序的回放 | 固定槽提交 | 当前提交 | 候选数 |
| --- | ---: | ---: | ---: |
| 模拟稳定30fps，常规预算 | 300 | 300 | 300 |
| 模拟稳定60fps，常规预算 | 600 | 600 | 600 |
| 模拟60fps，相位移动7.33ms、交替±1ms抖动 | 331 | 600 | 600 |
| 模拟53fps，相位移动7ms、交替±4ms抖动 | 453 | 530 | 530 |
| 模拟60fps源，显式30fps预算 | 301 | 300 | 600 |
| 模拟60fps源，显式45fps预算 | 450 | 450 | 600 |
| 模拟120fps抖动源，显式30fps预算 | 301 | 300 | 1200 |

12组模拟节拍回放通过，另外覆盖90/144/165Hz有效候选直通与显式24/59fps预算；显式30fps稳态最大提交间隔34ms。低预算长停顿后第一候选立即提交、紧邻下一候选被限流，旧帧rAF重试为0。

19组原始trace抽样入口时序逐项核对通过。其中2组在固定槽规则中发生冲突，当前规则全部准入。抽样入口时间与真实admit调用时间有观测边界，这项结果验证准入行为；设备实际新增帧率沿新构建日志验收。

```sh
./node_modules/.bin/vitest run --reporter=json --outputFile=/tmp/xbxrc-frame-admission-full-tests.json
node dev-docs/validation/2026-11-05-browser-frame-admission-audit.cjs /tmp/xbxrc-frame-admission-full-tests.json dev-docs/validation/2026-11-05-latest-frame-scheduling-fresh-trace-analysis.metrics.json dev-docs/validation/2026-11-05-browser-frame-admission.metrics.json /tmp/xbxrc-2026-11-05-latest-scheduling-trace.jsonl
```

## 设备验收

本轮自动证据确认默认准入损失消除、低FPS预算有效及生命周期正常。旧设备首段84次限帧跳过的实际帧率收益、video→candidate仍约53–55fps的供给缺口、GPU执行与物理submit-to-present由新设备样本继续核验。常规路径随有效候选直接提交，GPU/合成端积压需要观察提交间隔、GPU计时与物理呈现，而源身份/候选缺口继续由源层事实解释。

下一轮分别固定video、普通锐化、SR持续运动场景各60秒，核对同窗received→decode→video→candidate→submit、rendererRateLimitDropCount、提交间隔P95/P99与帧龄；网络freeze/恢复段单列，并补充物理present和端到端手柄响应。
