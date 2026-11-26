# PC 帧率统计口径核对

- 日期：2026-11-26（Asia/Shanghai）。
- 范围：当前 `release/test` 工作区的 PC Browser 串流统计；本轮执行源码核对和两项现有回归，保留应用实现。
- 结论：顶部 FPS 表示串流视频的呈现/实时绘制提交频率。30fps 游戏采用 60fps 串流编码时，同一游戏画面可以作为多个独立视频帧传来，统计可显示 60fps。游戏内帧率需要游戏自身的渲染遥测。

## 来源

1. `src/components/stream/StreamExperiencePanel.vue:30`：顶部 `presentFps` 使用 `FPS` 标签，容易让用户按游戏帧率理解。
2. `src/streaming/stream-panel-view-models.ts:69`：显示值直接取有效快照的 `presentFps`。
3. `src/player/app/stats/StatsService.ts:349`：原生 video 按视频呈现帧增量与实际采样时长统计。
4. `src/player/app/stats/StatsService.ts:365`：canvas 按 `realtimeDrawCount` 的推进统计实时绘制提交；目标帧率、接收帧率与此事实各自记录。
5. `src/player/app/media/PlaybackService.ts:444`：源节拍估计来自视频媒体时间与原始呈现帧计数，观测范围仍为媒体链路。

独立视频帧的身份推进与游戏内容更新频率分别成立。客户端现有计数保留串流媒体事实；游戏内容重复率和物理屏幕扫描频率需要对应的额外观测。用户本次会话的实际编码帧率、画面重复率及游戏内帧率仍由实机数据确认。

推荐将顶部标签明确为“串流 FPS”，游戏实际帧率以游戏内统计为准。

## 最小验收

执行一次：

```sh
pnpm exec vitest run src/streaming/stream-panel-view-models.test.ts src/player/app/stats/StatsService.test.ts -t 'keeps actual source resolution|aligns independent received' --reporter=dot
```

- 2 个文件、2 项定向测试通过；其余 20 项按名称过滤跳过，Vitest 时长 177ms。
- 面板回归：接收 60fps、实际呈现 24.6fps 时，顶部显示 24.6。
- 统计回归：接收 60fps、解码 56fps、video 推进 40fps、canvas 实时提交 4fps 时，顶部事实为 4fps。
- 实现、统计合同与媒体边界保持现状；本轮仅归档核对结果与任务状态。
- 本轮构建/测试进程已结束；一次性脚本、截图、DerivedData、xcresult 与临时日志创建量为零。
