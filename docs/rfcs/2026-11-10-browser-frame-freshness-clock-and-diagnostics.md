# Browser 帧时钟、诊断调度与空窗取证

- State: Implemented / Validation Pending；用户已明确授权处理建议 1、2、3，代码与自动门完成，远端长测及引擎内部导入路径接续实机取证。
- Owner: 主窗口，单窗口执行。
- 背景：12:30 日志提交 59.85fps，15 帧时间证据无效，ACK 单向有效覆盖约 40%，最大提交间隔 89ms。远端 1440p 按用户确认的远端特性收口。

## 实施合同

1. Worker/main 四时间戳探测建立有界时钟偏移与误差，低频刷新。帧年龄和 ACK 单向耗时使用校准结果；以年龄下界执行 50ms 陈旧释放，时钟缺证保持 unknown。内部同域 read/pending/ACK 往返继续独立记账。
2. 直通且 RVFC 正常时关闭逐刷新 rAF 诊断。250ms watchdog 检测回调停滞后启用 rAF，RVFC 恢复时释放；video/旧接口的现有回退保持。
3. 超过两帧间隔的供给事件进入固定容量、限频数值缓冲；保留 read/send/receive/consume/ACK 原始时序、时钟状态和主线程 longtask 支持状态。GPU 使用低频、单 fence、零等待异步完成观测，标明调度误差和 GPU 完成代理语义。
4. 应用实时链路保持 track clone→processor VideoFrame→transfer list→WebGL TexImageSource→GPU shader；禁止引入 copyTo、CPU RGBA/YUV 转换、ImageData、2D canvas 中转或像素读回。浏览器内部导入行为以引擎证据限定结论。

## 验证与边界

- 测试偏移、非对称消息时延、负差容差、误差下界、校准过期/乱序与帧资源退出。
- 测试 RVFC 正常/停滞/恢复、render-only 无诊断调度、video fallback、取消后的定时器归零。
- 测试异常缓冲容量与速率、longtask 缺省、fence 未完成零等待/超时/故障/释放；保持正常供给不被诊断阻塞。
- 测试 VideoFrame 对象及 transfer list 身份、实时绘制无 CPU 像素 API；生产 trace 白名单与健康/异常预算，lint/typecheck/build。
- 实机帧供给收益由下一轮日志验证；GPU fence 包含异步轮询派发误差，物理 scanout 继续独立验收。

## 实施结果

- 四时间戳校准、按需 rAF、同序号异常元组与 GPU 完成上下界已装配。Worker/main 分别保留 1ms 量化容差，同域小倒序做单调调整；陈旧释放基于年龄下界。
- fence 在首帧后预热 1 秒、独立任务插入，插入 CPU 超过 4ms 后释放并停止采样；上界包含查询 CPU 时间。完整时间戳只进入异常样本，常规硬预算保持。
- 38 文件 472 项前端、4 项 Rust 共享合同、lint/typecheck/build 及生产 30 分钟成本门通过。本地 WKWebView 两模式供给/提交 563、565 帧配平，invalid/unknown=0，ACK 有效覆盖全部已回传样本，readToSubmit P95=1ms，GPU 完成上界 P95=5ms，帧源 rAF/像素读回=0。
- 应用显式 CPU 像素拷贝为 0，NV12 和 texImage2D 参数符合公开 WebKit GPU 导入门；安装引擎的软件回退、物理上屏与真实 SR 放大分别保留证据边界。
- [Report](../reports/2026-11-10-browser-frame-freshness-clock-and-diagnostics.md)；[验证与源码身份](../../dev-docs/validation/2026-11-10-browser-frame-freshness-clock-and-diagnostics/README.md)。
