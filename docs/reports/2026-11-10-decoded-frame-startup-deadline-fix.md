# 逐解码帧默认路径、启动期限与 video 降级修复

日期：2026-11-10；主窗口直接执行。任务：`2026-11-10-decoded-frame-startup-deadline-fix`。

## 结果

默认原画、SR 与锐化使用 WebRTC 视频轨 clone 的独立 VideoFrame 供给现有 WebGL。初始化期间保留解码帧等待；接口缺省、初始化/首帧超时、读取失败及轨结束，按具体 `decodedFrame:<reason>` 接入 PlaybackService 的故障路径，直接降级为 video 标签呈现。用户配置保留，主动重试重新建立独立供给并等待首帧交接。

逐解码帧 reader 已区分 Worker 接口初始化与媒体首帧等待。视频轨在连接建立前创建时，主线程 processor 或已 ready 的 Worker 保留读帧等待；媒体轨 unmute 或共享帧源观察到真实呈现进展后，再执行 2 秒首帧期限。独立解码帧到达后解除期限，并继续沿现有 WebGL 引擎即时提交。

修复前的 09:54 样本中，reader 启动后约 6.76 秒才连接、7.49 秒才观察到首帧；固定构造后 2 秒超时使本次运行全程回退 presentation。原画及增强提交约 52fps，上游接收/解码约 60fps。源码与日志构建指纹匹配，详见 [排查报告](../../dev-docs/validation/2026-11-10-frame-supply-gap-investigation.md)。

## 实现

- `decoded-video-frame-reader.ts`：主线程 processor 构造成功视为接口就绪；Worker 保留 2 秒初始化截止，ready 解除该阶段计时。媒体开始供给且接口就绪后，执行独立首帧截止，回退原因记为 `firstFrameTimeout`。
- `video-frame-source.ts`：原始帧身份确认有新帧时，将媒体进展传给现有 reader，覆盖 unmute 事件缺省的运行时。重复媒体进展与重复 ready 保持首帧截止稳定。render 订阅只消费独立解码帧；接口失败延后到消费者登记后通知，取消与新 reader 隔离，多个 renderer 故障各自交给资源所有者。
- `Renderers.ts`：WebGL 上传源类型限定为 VideoFrame；供给失败停止请求链并上报原故障原因。暂停截图复用已上传纹理，维持截图与实时提交计数隔离。
- `PlaybackService.ts`：复用既有 video 降级、配置保留及显式重试；呈现观测按实际 renderer 选择独立解码身份或 video 原始身份。
- 轨 ended、主动退出和失败统一解绑原轨监听器、取消 reader、停止 clone 与 Worker。失败终态幂等，迟到帧正确关闭。

`maxBufferSize=1`、Worker 在途一帧与 ack、最新 VideoFrame 替换及关闭、独立 localSequence、显式低帧率预算均沿用现有合同。首帧到来后继续以独立解码帧驱动绘制，显示侧候选及时消费；原始 RVFC 保持呈现与协议诊断职责。

## 自动验证

| 验证 | 结果 |
| --- | --- |
| reader / 共享帧源 / WebGL / SR 调度、真实播放降级与统计健康 | 9 文件、199 项通过 |
| lint:fix | 通过 |
| typecheck | 通过 |
| build | 通过；产出独立 decoded-video-frame-worker 资源 |
| 修复前基线复现 | 日志构建 commit `76a1ea69` 的 reader SHA 与原排查相符，保留 3 个历史缺陷对照场景 |
| diff check | 通过 |

新增回归覆盖同步接口缺失、多个消费者故障通知与取消代际，真实 PlaybackService / WebGL / SR 的接口缺失、读取失败、首帧超时后实际切 video、后处理暂停、配置保留、迟到帧关闭、显式重试及退出资源归零。调度夹具改用独立 VideoFrame，证明低频 RVFC 下逐解码帧供给与提交配平；暂停截图沿已上传纹理验证。启动回归覆盖连接等待约 6.76 秒后首帧正常交付、Worker ready 后延迟媒体、缺省 unmute 时由呈现进展启动期限、已供帧轨的静默 reader、重复进展/ready 保持截止、Worker 初始化失败截止、原轨结束及迟到消息释放。已有回归继续证明零时间戳独立帧、最新帧替换、截图/调度与退出资源合同。

复算命令：

```sh
./node_modules/.bin/vitest run src/player/infra/render/decoded-video-frame-reader.test.ts src/player/infra/render/video-frame-source.test.ts src/player/infra/render/renderer-scheduling.test.ts src/player/infra/render/Renderers.test.ts src/player/app/media/PlaybackService.test.ts src/player/app/stats/StatsService.test.ts src/player/app/stats/browser-stats.test.ts src/streaming/runtime/browser-render-policy.test.ts src/streaming/runtime/playback-health.test.ts --reporter=dot
pnpm lint:fix
pnpm typecheck
pnpm build
node dev-docs/validation/2026-11-10-frame-supply-gap-investigation.repro.mjs
```

## 实机验收

当前完成代码与自动验证；原日志属于修复前基线。重新启动串流后，正常直通应观察 `actualPipeline=webgl2/webgl2_sr`、`renderFrameSource=worker/main-thread`、decoded 接收计数推进、候选与提交配平，并对原画、SR、锐化的稳定提交帧率和完整 3 秒最低值进行配对比较。供给或渲染异常时应记录具体故障原因，实际管线切为 video；显式重试后恢复独立解码供给。

验收同时覆盖网络健康、恢复链健康、media supply、steady supply、submit/present 长尾与帧供给 delta。画面新鲜度优先使用更新空窗、逐帧身份及 GPU 完成观测；最近提交年龄保持其“距上次提交”的语义。实际 GPU 执行与物理上屏证据继续由目标设备样本提供。
