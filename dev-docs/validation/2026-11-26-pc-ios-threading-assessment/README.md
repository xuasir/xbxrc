# iOS 执行域 RFC 对 PC Browser 串流的启发

最值得借鉴的是渲染与诊断的执行域隔离，并将视频供给和最终渲染的健康事实分别监督。当前 PC 已有独立读帧 Worker、输入边沿队列和多段统计；继续推进应围绕现有运行时及明确的状态归属。

评估依据为[给定 iOS RFC](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-11-26-ios-streaming-latency-threading.md)和当前 `main` 的 Browser 源码。iOS RFC 当前状态为 planned，本轮输出为源码评估；工程迁移沿独立 PC RFC 明确能力门槛和分阶段验证。

| 方向 | PC 现状 | 可借用的合同与建议 |
| --- | --- | --- |
| 渲染执行域，优先验证 | Worker 持续读帧，保持一个在途帧和一个 latest；帧消息到主线程后同步上传/绘制，消费返回后才发 ACK | 在现有读帧 Worker 上验证 OffscreenCanvas / WebGL2，将逐帧导入、USM/SR、帧选择及 GPU 观测置于唯一渲染执行域；主线程维护 DOM canvas、尺寸与配置快照、菜单和生命周期。保持 latest-only 和同帧纹理复用 |
| 诊断聚合，低成本切入 | RTC 250ms 采样、1Hz UI 指标与 trace 准入已有约束；分位数排序、payload 投影和 JSON 编码在前端执行 | 借用有界数值 batch、独立聚合执行域、1Hz 分位数与不可变快照；主线程保留必须在 Window/DOM 读取的事实；公共监督读取快照，trace I/O 继续使用既有 Rust writer |
| 输入顺序与节拍 | 数字边沿/扳机变化/归零帧分别保留，纯摇杆更新合并；一次 flush 可连续排空多个输入包，兜底 retry 为50ms | 借用单调时钟 absolute deadline、有效节拍逐个交付数字状态、neutral barrier 与 epoch；输入队列与帧反馈共享发送预算，成功发送后出队。根据PC采样、排空和远端识别证据决定节拍与预算 |
| 显示观察与本地恢复 | `BrowserRenderHealth` 识别提交不足并输出性能告警；媒体活跃时间由浏览器 video 帧通知更新，公共监督尚无显式 render/display 作用域观察 | 复用 RFC 可选 displayObservation 思路，分开 decode、rendererSubmit、GPU completion proxy 和 video/RVFC；本地渲染重置保留原 peer，并设置 renderEpoch、单飞、动作预算与效果观察。物理呈现按宿主能力标记 |

目前的关键串行链在 [decoded-video-frame-reader.ts](/Users/guo.xu/Documents/code/games/xbxrc/src/player/infra/render/decoded-video-frame-reader.ts:317)：`onWorkerMessage → accept → consume → VideoFrameSource.notify → renderer.drawFrame/renderFrame → finally ACK`。Worker 的读帧可以继续覆盖 latest，在途帧的下一次发送资格等待主线程 ACK；主线程绘制、UI 或诊断长任务会影响消费、手柄采样和发送的及时性。渲染迁移的主要收益目标为 P95/P99 和主线程繁忙时的持续供给，平均上传开销使用已恢复的 texImage2D 基线另行核对。

平台边界需要按 Browser API 落地：

- [BrowserGamepadBackend](/Users/guo.xu/Documents/code/games/xbxrc/src/shared/gamepad/browser-backend.ts:133) 使用 Window 的 `navigator.getGamepads()`，当前采样周期为4ms。WorkerNavigator 类型包含的接口以系统/浏览器能力为准；当前 TypeScript Worker 库的 WorkerNavigator 省略 Gamepad API。设备采集、几何导航和 DOM 输入继续由当前主线程输入路由负责。
- TypeScript Worker 库已声明 RTCDataChannel，RTCPeerConnection 仍属于 Window 装配。独立发送 Worker 可以作为能力验证方向；WKWebView / WebView2 的 DataChannel 实际转移、bootstrap、writable 事件、关闭顺序和租约 fence 要先验证。输入 Worker 的独立性覆盖已采集状态，实体手柄的新状态按 Browser 采样交付。
- OffscreenCanvas 类型存在；实际 Worker WebGL2、VideoFrame 导入性能、上下文恢复、截图和尺寸更新仍需要两个桌面宿主的能力原型。主线程创建并挂接 DOM canvas，离屏转移发生在 context 创建前；GPU 对象由创建它们的唯一 context/Worker 持有。
- iOS 的 CAMetalDrawable 实际呈现回调提供对应平台的显示事实；PC 当前 canvas 的物理 scanout 为 unavailable。WebGL fence 表示 GPU 队列完成代理，RVFC 表示浏览器 video 的呈现通知；两者各自携带来源和新鲜度。
- iOS 的CVMetalTextureCache映射和PC的VideoFrame完整导入分别保留各自验证过的资源路径。PC已恢复 `texImage2D` 新帧导入，同帧纹理复用、源尺寸/context失效继续维护；Worker迁移时保持该合同并重新核验实际导入成本。

PC已有输入和诊断基础应继续复用。[InputService](/Users/guo.xu/Documents/code/games/xbxrc/src/player/app/input/InputService.ts:222) 已具备有序状态队列、模拟轴合并、generation fence、neutral重置、发送成功后出队和浏览器缓冲年龄保护。现行预算为128个待发状态、250ms输入年龄、50ms缓冲年龄和50ms兜底retry；iOS RFC 的8ms/64ms及1024/512字节是其规划初值，PC参数通过现有队列和实际发送证据确定。

[browser-runtime-trace.ts](/Users/guo.xu/Documents/code/games/xbxrc/src/streaming/runtime/browser-runtime-trace.ts:118) 已将profile、节流、队列准入放在payload构造前；Rust已承担trace写入。进一步迁移的对象是准入后的数值聚合、分位数、payload投影与JSON构造；DOM尺寸/样式、peer.getStats等Window事实采集保留在对应宿主。当前LatencyWindow在snapshot时计算并缓存排序结果，指标UI按1Hz更新，逐帧数字累计继续保持低成本。

[BrowserRenderHealth](/Users/guo.xu/Documents/code/games/xbxrc/src/streaming/runtime/browser-render-health.ts:62) 当前 `fallback=false`，吞吐不足输出告警建议；[公共监督样本](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbox-streaming/src/session/recovery/supervision.rs:37) 尚以peer/video和媒体窗口为主。后续显示恢复应沿可选公共合同增加 renderEpoch、当前renderer提交年龄和同窗delta，保留网络/解码恢复归属；同一renderer的有界重置采用清晰的配置/资源票据、失效fence和效果观察。

后续验证先固定一次最小基线：网络RTT/jitter/loss、Browser恢复计数、received/decode供给、3s/15s steady supply、read→submit与GPU完成代理长尾，以及received/decoded/submitted/overwrites/localDrop同窗delta；输入另核对physical sample→dispatch→send的P95/P99。一个受控主线程繁忙场景即可检查已采集输入发送、Worker最新帧和渲染推进，再以一个尺寸/context/停止序列核验资源寿命。指标的物理上屏能力继续明确标记。

本轮最小验收为一次源码证据、接口声明和文档引用核验，记录于 `checks.json`；运行时代码沿用现有实现，临时产物创建数为0。

建议优先在现有读帧 Worker 上做 OffscreenCanvas / WebGL2 / VideoFrame 能力与成本原型，同时明确 DOM/surface 配置和渲染状态的唯一归属；诊断聚合与可选显示观察可按独立阶段接续。

后续具体方案见[PC执行域RFC](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-11-26-pc-streaming-execution-domains.md)：细化为独立Reader和Render两个Worker，并以MessageChannel直连。该结构保留Reader在同步GL调用期间持续更新latest的能力，帧链保持一次transfer/一次ACK，额外控制/统计/反馈成本进入M0。当前PC局部恢复已落localCounters与renderer提交合同，线程迁移将接续现有所有者；数据channel发送迁移后置为按宿主能力接入的阶段。
