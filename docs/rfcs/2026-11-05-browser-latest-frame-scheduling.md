# Browser 最新帧渲染调度 RFC

## Status

- Completion: 已完成代码与自动验证
- Current State: completed
- Owner: 主窗口，单窗口执行
- Last Updated: 2026-11-05
- Authorization: 用户明确要求“简化策略，无论主机还是云游戏渲染调度都走覆盖旧帧、立即追逐最新帧”。

## Background

[本次设备日志](../../dev-docs/validation/2026-11-05-render-input-fresh-trace-analysis.md)的健康 SR 段呈现计数推进182帧、有效回调144次、提交143帧；181次rAF轮询新增候选0，179次GPU像素读取新增供帧0。当前增强绘制CPU均值约0.274ms，GPU实际执行与物理present缺证。已有逐帧像素补偿与累计deadline重试使热路径拥有多套节拍和资格。

## Goal 与合同

1. 主机/云游戏复用同一个 Browser VideoFrameSource 与两种增强 renderer，使用源的最新可用图像。有效新候选到达立即尝试提交，后续候选覆盖被跳过的图像。
2. 常规60fps及以上预算直接放行每个有效新候选；源身份去重继续由VideoFrameSource负责。低于60fps的显式预算使用滚动准入时间与1ms量化容差，周期内保留剩余额度，迟到超过一个周期重新锚定当前时间。低预算超额候选直接跳过，后续新候选立即重新判定；零预算继续暂停。准入层仅保留目标帧率与下一次准入时间。
3. 帧身份继续沿有效RVFC/播放计数和有界双向抵扣，原始metadata保留给协议与统计。GPU缩略像素仅作为dev低频诊断，退出供帧资格与renderer准备链。
4. 截图独立进行一次有界绘制；初始化、暂停、context恢复、关闭与迟到回调代际保护继续有效。
5. 统计把限帧跳过与提交失败分列，提交率、源推进和物理呈现保持各自证据口径。

## Scope

- 当前main Browser渲染模块、统计投影与面板；共享路径同时覆盖主机和云游戏。
- 网络、解码与恢复由现有Browser WebRTC运行时负责；native WIP保持rust开发线的既有装配。
- 默认video继续使用浏览器自身呈现；锐化、颜色、SR算法和真实源尺寸下限沿原实现。

## Plan

1. 将FramePacer收成即时准入，移除两个renderer的旧候选rAF重试及重复fresh状态。
2. 移除像素供给模式与预准备消费者，保留有界dev异步取证。
3. 更新跳帧统计和面板，验证抖动、稀疏供给、预算、截图与生命周期。

## Validation

- [x] 两个生产renderer在60fps及相位抖动下直接提交，限帧跳过后rAF保持提交数不变，新帧到达直接推进最新序号。
- [x] 30fps上限、90/144/165Hz显示、长期暂停/恢复、稀疏RVFC与播放计数去重。
- [x] dev/off候选与提交一致；诊断最多约2组/秒，退出资源归零；静态像素和异步过期保持诊断事实。
- [x] 前端60文件591项测试、lint:fix、typecheck、build（313模块）与diff check。

## Risks 与设备验收

- rVFC通知稀疏且播放计数合批时，真实新增图像的可获取时机仍由WebView决定。此次删除高频像素补偿；此前受控像素补满60fps样本保留为历史合同，当前验收以即时处理实际候选为准。
- 常规预算随实际有效候选直接提交，WebGL内部GPU/合成队列需要设备测量。显式低FPS滚动限流约束长期提交总量，帧率提升仍取决于WebView实际候选供给。
- 用户分别固定主机/云游戏的video、锐化、SR持续运动场景，比较同窗received→decode→video→candidate→submit、提交间隔与帧龄，再测物理present/输入响应；网络freeze与恢复段单独归因。

## Progress

- [x] 合同、授权和现有日志证据明确。
- [x] 实现与自动验证，9组生产FramePacer回放通过。
- [x] [Report](../reports/2026-11-05-browser-latest-frame-scheduling.md)与任务闭环；真实设备供给/GPU/present接续Browser stable/S4。

## 新设备证据后的准入修正

[新设备日志](../../dev-docs/validation/2026-11-05-latest-frame-scheduling-fresh-trace-analysis.md)确认首段352候选中84个被固定时间槽跳过，生产回放复现运行中相位移动与交替抖动的损失。用户随后授权“优化这部分”，本次按上述更新合同直接修复局部准入规则；普通锐化/SR与主机/云游戏共用FramePacer，源资格、像素低频诊断及生命周期沿现有实现。

修复与[独立报告](../reports/2026-11-05-browser-frame-admission-fix.md)已完成，60文件610项测试及12组节拍/19组设备入口回放通过；SR恢复缓存重复提交同步收口。新设备的真实供给/GPU/present继续进入既有验收范围。
