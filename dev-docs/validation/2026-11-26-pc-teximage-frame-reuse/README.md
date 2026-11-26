# 恢复 texImage2D 新帧导入与同帧纹理复用

已按用户确认策略完成：每个新 VideoFrame 使用 `texImage2D(VideoFrame)` 导入，同一帧的截图重绘复用已有纹理。实现与最小验证完成，恢复后的真实上传耗时接续实机日志确认。

修改范围为 [Renderers.ts](/Users/guo.xu/Documents/code/games/xbxrc/src/player/infra/render/Renderers.ts:810) 与 [renderer-scheduling.test.ts](/Users/guo.xu/Documents/code/games/xbxrc/src/player/infra/render/renderer-scheduling.test.ts:504)。采用[历史实机对照](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-26-pc-upload-historical-comparison/README.md)确定的优先策略；沿用当前 Browser / WebGL2 / SR 渲染器及 Worker 供帧。

- 新帧直接通过完整导入更新既有源纹理，上传成功后记录 `videoGeneration + localSequence`；持续 0 时间戳的新解码帧分别处理。
- 同帧截图保持纹理复用；源尺寸变化、纹理创建、context 释放/重建清空上传缓存，输出尺寸切换保持源纹理身份。
- 首次完整导入及源尺寸/context 失效后沿用 GL 强制确认；稳态沿既有每秒查询预算和统一失败归属处理。
- 子上传探测与回退运行态已移除；资源快照固定 `videoUploadMethod=texImage2D`、`videoTextureSubUploads=0`，成功完整上传与纹理复用继续分别累加，既有可选 trace 类型兼容历史日志。

最小验证各执行一次：

| 检查 | 结果 |
| --- | --- |
| 两个既有渲染测试文件 | 117/117 通过，Renderers 11 项、scheduling 106 项 |
| pnpm typecheck | PASS |
| pnpm lint:fix | PASS，任务范围外前端文件内容变化为 0 |
| 本轮 git diff --check | PASS |

既有回归确认同尺寸帧身份、同帧截图/输出尺寸变化复用、较新的限帧截图候选、源尺寸增长/缩小、同帧在 context 恢复后重新上传、主线程兼容供帧及错误归属。原子上传兼容用例调整为策略回归：子上传 API 缺失、抛异常或模拟 8 ms 成本时，新帧继续使用完整导入；首次完整导入的 context 错误由统一所有者接住。模拟耗时用于验证 API 选择与计数合同，实机性能由运行时 trace 量化。

```sh
pnpm exec vitest run src/player/infra/render/Renderers.test.ts src/player/infra/render/renderer-scheduling.test.ts
pnpm typecheck
pnpm lint:fix
```

测试摘要见 `renderer-smoke-summary.json`，源码基线/最终 SHA256、命令结果、lint 边界与清理结果见 `checks.json`。本轮既有暂存内容保留，变更按任务范围直接应用；首次验证全部通过，失败诊断为空。

本轮临时目录为 `/private/tmp/xbxrc-pc-teximage-restore-20261126-dxvpg9mu`；清单包括两份源码基线、lint 前前端快照、完整测试报告与运行日志。相关验证进程已结束，必要摘要归档后执行清理；保留本目录 README 和两份 JSON。

下次实机验收关注 `videoUploadMethod=texImage2D`、新帧完整上传计数推进、截图复用计数及上传 P95，按历史约 1 ms 基线核对恢复效果。

清理核验：目标目录已删除，本轮移除 2,977,878 字节；清理后可用磁盘空间 92,160,000,000 字节。
