# 会话分辨率与 Worker 新鲜度验证

代码验收已完成。实机验收关注远端实际尺寸、本地供给空窗与提交后长尾；本目录的回放使用确定性模拟。

- [实施报告](../../../docs/reports/2026-11-10-session-resolution-and-worker-freshness.md)
- [供给回放脚本](replay.mjs)：加载当前 Worker/reader，与显式旧 ACK 串行策略模型比较；固定 60fps 与 80/175ms 主线程停顿。
- [供给回放指标](replay.metrics.json)：包含源文件 SHA256、完整时序分布、主动覆盖与陈旧丢弃计数。
- [生产预算指标](production-trace.metrics.json)：30 分钟健康/异常/输入完整路径的字节、编码成本和入队结果。

复算：

```sh
node dev-docs/validation/2026-11-10-session-resolution-worker-freshness/replay.mjs
PRODUCTION_TRACE_METRICS_PATH=dev-docs/validation/2026-11-10-session-resolution-worker-freshness/production-trace.metrics.json pnpm test:trace-production
./node_modules/.bin/vitest run src/player/app/session/SessionService.test.ts src/player/app/stats/StatsService.test.ts src/player/infra/render src/player/infra/webrtc/SdpManipulator.test.ts src/player/app/media/PlaybackService.test.ts src/streaming/runtime/browser-runtime.test.ts src/streaming/runtime/playback-health.test.ts src/shared/diagnostics/production-trace-contract.test.ts
cargo test -p xbox-streaming --lib
cargo test -p xbxrc --lib mods::runtime_trace::trace_contract
```

回放改进的是阻塞恢复后的提交新鲜度。主线程停顿、网络和 GPU 的持续空窗需要各层证据分析。readToSubmit 从处理器读取完成开始计时，源帧完整 glass-to-glass 年龄继续需要远端时间与上屏证据。
