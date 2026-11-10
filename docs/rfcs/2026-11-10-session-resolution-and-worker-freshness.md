# 会话分辨率约束与 Worker 帧新鲜度

## Status

- Current State: code-validated；远端采纳与实机时延收益待验收
- Owner: 主窗口，单窗口执行
- Authorization: 用户 2026-11-10 明确要求排查会话创建/SDP 约束，并分解 Worker 接收/ACK 与供给空窗、压低画面陈旧时间。当前实施承接已授权的逐解码帧直通合同。
- Report: [实施与验证结果](../reports/2026-11-10-session-resolution-and-worker-freshness.md)

## 证据与目标

11:17 日志第二会话的 config、compiledPlan、Windows 设备画像、HTTP 设备头和 Browser 目标均为 1920×1080；本地 SDP 只有 max-fs=8160/max-fr=60，远端实际为 2560×1440。创建计划已经跟随 xHome 独立设置，接收宽高限制需要在 SDP 中明确声明。

同日志两会话 Worker 交付后与 WebGL 提交配平，首段新增 4 帧缺口，局部提交空窗达 100ms 以上。现有 Worker 在主线程同步消费并 ACK 后才开始下次 read；ACK 往返与处理器读取串行。用户要求继续优先保证最新画面。

## 范围与合同

- 保持公共 Rust 会话编译与 HTTP 网关；补齐安全的会话分辨率摘要，验证 cloud=1440/home=1080 的隔离与实际请求设备头。
- 在既有 Browser offer 操作中添加 H264 接收宽高 imageattr，与 max-fs/max-fr 同源；正确限定 video section、支持 CRLF/LF、保持重启幂等。远端 answer 与实际解码尺寸分别作为诊断证据。
- 保持原 RTCPeerConnection、视频轨 clone、MediaStreamTrackProcessor 与 WebGL；Worker 连续读取并保留最新一个待发送帧，ACK 控制在途帧数为 1。待发送帧替换时及时 close。
- 主线程以共同 performance 时间基准测量读完→发送→收到→同步消费→ACK；供给消息带独立序号与有界数值统计。只在快照时计算分位数。
- 对已知 Worker 交付年龄超过 50ms 的帧及时 close/ACK，释放下一张最新帧；时间证据缺省时承接现有交付。50ms 是本地交付预算，GPU 完成与物理上屏继续独立验证。
- 默认独立解码帧直通 WebGL，故障沿既有通道降级 video；取消、停止、错误与迟到帧均保持资源归属闭环。

## 验证

- Rust 编译矩阵：Home 720/1080/1080HQ/1440 设备画像与 HTTP 头，尤其 cloud=1440/home=1080；网关 payload/headers 与编译计划一致。
- SDP 回归：1080/1440 宽高与宏块约束；媒体边界与 payload 复用；LF/CRLF；重复 offer/restart 幂等；既有 fmtp/码率/反馈保持。
- Worker 回放：延迟 ACK 下持续读取、待发送帧替换、最新帧即时释放、在途≤1、重复 ACK 代际/序号隔离、停止/异常/迟到帧释放。
- 主线程：传输年龄/消费/ACK 分解与 50ms 陈旧帧释放；timestamp=0、首帧等待与 fallback 保持；供给快照进入 trace。
- 执行相关 Vitest、cargo fmt/Rust 检查、pnpm lint:fix/typecheck/build 及 diff check。
- 实机下一轮：确认远端实际尺寸，比较 media/steady supply、帧供给 delta、Worker 传输/ACK P95 和空窗；GPU/物理上屏与网络恢复健康按能力限定。

## 风险与验收边界

- imageattr 表达接收尺寸协商约束，远端采纳程度由新日志验证；保留源尺寸与用户配置事实。
- 连续读取只保留最新待发送帧；低供给段的陈旧帧丢弃会牺牲部分完整性，以本地新鲜度为优先目标。
- 跨 Worker 时间使用 performance.timeOrigin + performance.now；负差和缺省时间保持 unknown，避免时钟缺证触发丢弃。
- 诊断通过已有帧消息与低频资源快照发布，逐帧写日志和 GPU 同步读取保持为 0。

## 验证落地

会话创建 HTTP 回放和 Browser 实际 offer/ICE 重启验证通过。Worker 最新槽、序号 ACK、陈旧交付与资源退出回归通过；20 文件 318 项前端回归、146 项 xbox-streaming、4 项 Rust trace 合同、lint/typecheck/build 均通过。

production 的 Worker 数值证据拆入既有发布节拍下的独立 1Hz/2048B 快照，常规预算为 307869.7B/min，健康发布完整且丢弃为 0。保留原 4096B 健康行预算。确定性回放中两类空窗恢复后陈旧提交均从 1 次降至 0，最新帧提交从 5ms 提前到 4ms；该结果绑定模拟环境，实机收益与远端尺寸采纳接续 fresh trace。
