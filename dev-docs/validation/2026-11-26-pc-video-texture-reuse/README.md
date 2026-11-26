# PC 串流同尺寸纹理更新与同帧上传复用

- 日期：2026-11-26（Asia/Shanghai）。
- 范围：当前 Browser PC 的既有 WebGL2/SR 引擎；用户确认的两步上传优化。
- 状态：代码实现与针对性冒烟完成，实际 WebView 耗时由实机短测量化。
- 源码：`src/player/infra/render/Renderers.ts`、`render-observation.ts`；回归：`renderer-scheduling.test.ts`。

## 实现

1. 首帧或解码源尺寸变化时，用 `texImage2D(VideoFrame)` 初始化/重定义已有源纹理；稳定尺寸的新帧通过 `texSubImage2D(VideoFrame)` 更新。
2. 上传缓存由 `videoGeneration + localSequence` 标识；时间戳持续为 0 的不同解码帧各自更新。截图重绘同一帧复用现有纹理，较新的限帧候选仍正常上传。
3. 源尺寸变化清空上传缓存；纹理创建、context 释放/重建同时重置缓存与能力确认。输出尺寸变化保持源纹理缓存，重绘沿现有显示计划执行。
4. 每个 context 首次子上传使用既有 GL guard 确认；API 缺失、同步异常或 GL 拒绝时，当前帧通过完整上传补做并在该 context 内保持回退。context 丢失继续交由现有失败所有者处理。
5. 资源快照记录实际上传方式、回退原因、成功完整/子上传次数及纹理复用次数；既有上传/提交耗时与截图独立计数继续使用。

## 最小验证

- 最终运行两个既有渲染测试文件，**117/117 通过**：`Renderers.test.ts` 11 项，`renderer-scheduling.test.ts` 106 项。
- `pnpm typecheck` 通过，覆盖最终生产实现；随后调整了既有测试对上传入口的计数断言。
- `pnpm lint:fix` 通过；任务范围外前端文件内容变化为 0。
- 本轮变更范围的 `git diff --check` 通过。
- 回归覆盖稳定纹理及 0 时间戳、截图/输出尺寸切换复用、限帧后较新截图候选、源尺寸增长/缩小、context 重建后同帧重新上传、能力缺失/异常/GL 拒绝回退及 context 丢失。
- 既有稀疏 RVFC/像素诊断开关回放继续确认 600 帧的顺序和唯一性；30/60fps 供给、暂停恢复与 GL 低频查询预算通过。

复现命令：

```sh
pnpm exec vitest run src/player/infra/render/Renderers.test.ts src/player/infra/render/renderer-scheduling.test.ts
pnpm typecheck
pnpm lint:fix
```

## 本轮失败诊断

首次针对性运行 112/117 通过；5 项原有断言把 `texImage2D` 作为唯一帧导入入口。稀疏供帧的观测改为同时统计完整/子上传；主线程零时间戳用例分别确认首帧初始化与后续更新。保持原有帧供给、顺序、唯一性与及时提交断言后，最终 117/117 通过。详见 `initial-failure-summary.json`。

## 实机收益口径

当前结果证明上传调用和缓存生命周期正确。浏览器可在 `texImage2D(VideoFrame)` 内复用存储并使用专用快速路径，两种导入方式的真实成本按设备量化。

建议在相同源分辨率、显示尺寸与增强设置下做短测，同窗比较上传和 read/receive→submit P95/P99、GPU 完成上下界、稳态帧间隔及 received→decoded→delivered→submitted 增量；同时记录选中路径 RTT/jitter/丢包、jitter buffer 与恢复后持续供帧。实际 submit→屏幕呈现通过平台观测或外部取证确认。

## 临时产物收尾

- 本轮临时目录：`/private/tmp/xbxrc-pc-video-texture-reuse-20261126-zbi5t73n`。
- 临时清单：源文件基线、lint 前前端快照、初次/最终完整测试报告、运行日志与一次性核验输出。
- 全部测试、类型检查和 lint 进程已结束；必要测试摘要、失败诊断与检查结果归档到本目录，临时目录随本轮收口删除。
- 保留项：本目录 README 与 JSON 摘要。

清理核验：目标目录已删除；本轮移除临时文件 3331505 字节，清理后可用磁盘空间 92979200000 字节。详见 `checks.json`。
