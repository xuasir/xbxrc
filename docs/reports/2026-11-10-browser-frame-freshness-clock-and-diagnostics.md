# 帧时钟、诊断调度和空窗修复

日期：2026-11-10。代码与自动验证完成，实机远端长测待验。

关联 [RFC](../rfcs/2026-11-10-browser-frame-freshness-clock-and-diagnostics.md) 与[验证记录](../../dev-docs/validation/2026-11-10-browser-frame-freshness-clock-and-diagnostics/README.md)。远端分辨率沿用户确认的特性收口。

## 交付

1. Worker/main 通过四时间戳消息校准偏移和误差，低频刷新；同域 1ms 量化倒序调整后记账，跨域帧年龄与 ACK 单向耗时使用校准值。校准过期保留 unknown，以年龄下界超过 50ms 执行陈旧释放，持续最新帧优先。
2. 直通供给且 RVFC 正常时关闭逐刷新 rAF 诊断，250ms watchdog 按需恢复轮询，RVFC 恢复后取消；纯 renderer 在启动成功后释放呈现诊断回调。启动期限、video 降级和显式像素诊断有回归覆盖。
3. 四项、1Hz 异常环记录 read/send/receive/consume/同序号 ACK 原始时序与时钟误差，longtask 有界关联；GPU 采用预热后独立任务插入、单 fence、1Hz、零等待与最多八次轮询，插入开销超过 4ms 后停止。常规日志保存完成上下界，完整 GPU 时间放入异常样本，保留硬预算。

## 结果

前台 WKWebView 本地 1440p H264 WebRTC 的 USM 与同档 SR 配对接收/解码/提交分别为 58.610fps、58.860fps。read/sent/received/delivered/submitted 分别全为 563、565，覆盖与陈旧丢弃为 0；ACK 单向和往返有效样本为 562/562、564/564，最后 ACK 的回传边界单列。两模式时钟 invalid/unknown 为 0，read 完成→提交 P95 为 1ms，GPU fence 完成上界 P95/最大为 5ms，应用帧源 rAF 和像素读回为 0，退出资源归零。

自动门：38 文件 472 项前端、4 项 Rust 共享合同、lint/typecheck/build/diff check 通过。30 分钟成本门写入 1801 条健康记录：常规 326196.93B/min、异常单分钟最大 127687B、编码 P95 0.1665ms，丢弃/超长/IPC 失败为 0。

## CPU 拷贝与实机边界

应用实时路径保持 VideoFrame 对象转移与 WebGL 直接导入，显式 CPU 像素拷贝为 0。CPU 守卫和对象身份测试覆盖供给、渲染与 GPU 重建；FSR 的 CPU 模块仅生成 uniform 常量。用户主动截图和显式像素诊断保留各自的读回路径。

当前夹具测得 NV12，现有 texImage2D 的完整源/UNSIGNED_BYTE/RGBA 参数符合公开 WebKit IOSurface/GPU 导入门。公开引擎源码也存在内部软件回退；已安装引擎的实际导入路径和端到端零拷贝需要引擎级证据。保留 texImage2D，公开快路径对 texSubImage2D 有额外函数门限制。

本地夹具的 lost/NACK/freeze/dropped 增量均为 0，读后提交与供给配平；主要短时空窗落在 readWait/receiveGap。远端基线网络、恢复、media supply、steady supply、提交/上屏和帧供给 delta 与下一轮验收方向一并记录在验证目录。SR 输入/输出同为 1440p，实际 EASU 放大、目标设备长测和物理 scanout 接续用户手动验收。

下一轮真实串流按默认、SR、锐化分别保存稳定段，优先核对校准和 ACK 覆盖、同序号异常元组、读取后帧年龄与 GPU 完成代理长尾，保证供给恢复后及时承接最新画面。
