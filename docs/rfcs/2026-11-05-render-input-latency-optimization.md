# 渲染与手柄输入延迟优化 RFC

## Status

- Completion: 代码及自动验收完成；设备验收接续 Browser stable / S4
- Current State: completed
- Owner: 主窗口直接执行
- Last Updated: 2026-11-05
- Authorization: 用户在代码与历史证据审查后明确“就按建议去做”，授权本方案实施及自动验证。

## Background and Goal

main 的 Browser 媒体与 Gamepad 共享 WebView 主线程。canvas 补帧探测额外导入完整源纹理，4ms 手柄轮询持续构建设备与状态对象。已有帧率、队列及释放保护需要实际阶段长尾和浏览器缓冲年龄证据。

本轮降低足帧阶段探测成本，补齐渲染/输入分位数，复用稳定设备与状态，限制输入通道新增积压。历史依据：[浏览器稳定性 RFC](2026-10-28-browser-runtime-simplification-and-stability.md)、[输入交付 RFC](2026-11-02-gamepad-input-delivery-fixes.md)、[冷准备修复报告](../reports/2026-11-03-sr-cold-preparation-and-runtime-supply-fix.md)。旧设备51.39/46.97fps保留为冷准备修复前证据。

## Scope and Decisions

- 保持Vue/TypeScript/Tauri/Rust、单一共享帧源、latest-only、FramePacer、源像素下限与原有增强/恢复权限。
- 探测初始保持供帧资格；原始候选在1秒稳定窗口达到目标98%后降频。候选间隔超过目标间隔的1.5倍加1ms时立即恢复探测，下一候选建立补帧基线；1秒供给不足也恢复。计数推进与GPU补帧共同补供给，GPU补帧单独记账。单PBO、8ms结果新鲜度/稳态CPU预算、50msGPU期限继续生效。
- 探测错误检查绑定准备/冷导入与有界周期，fence及硬异常持续检查。当前探测和绘制使用独立context，本批按需探测减少重复导入，资源归属沿已有帧源执行。
- 最近256个数值样本提供P50/P95/P99/max，并保存统计期整体最大值，低频快照排序。FPS或暂停预算变化重置提交间隔。GPU timer每秒最多采一帧、一个query在途，5秒未完成时释放；扩展缺失/disjoint/失败显式表达，失败清除旧证据。
- 手柄始终读取真实状态，复用稳定设备信息和未变标准态。真实poll Hz、发布Hz及间隔分别统计。4ms/rAF、变化即时和100ms心跳继续执行。
- 输入只观测本会话浏览器bufferedAmount排空；有界字节账本记录接受年龄。普通发送采用512B基础余量及50ms排空预算，有效排空速率可扩大软水位，最多4KiB；16KiB硬水位及1KiB释放余量继续生效。缓冲持续老化时暂停新增普通状态，释放及前置边沿有序交付。128状态/250ms旧输入重建继续执行。
- 真实scanout/glass-to-glass与Windows/ROG实机运行沿用户手动验收，网络内部恢复事实保留观测边界。

## Plan and Validation

1. 有界延迟统计、renderer分段/间隔、异步GPU timer与生命周期回归。
2. 足帧探测降频；45/50Hz稀疏回调、静态画面、30fps、迟到/尺寸/代际/GPU异常验证。
3. Browser采样复用、UI消费投影、真实Hz与采样到发送长尾验证。
4. 缓冲排空账本及软预算，拥塞/边沿/扳机/释放/过期/会话替换验证。
5. pnpm test、pnpm lint:fix、pnpm typecheck、pnpm build、git diff --check及既有生产回放。

## Risks

健康降频需要快速恢复及持续唯一图像回放；GPU扩展和bufferedAmount具有平台边界。新增排队预算与SCTP已接受报文、远端游戏处理分别验收。

## Progress

- [x] 用户授权、历史约束与实施合同。
- [x] 渲染与输入实现。
- [x] 自动验证、报告与手动验收步骤。

59文件600项测试、lint、类型检查、生产构建、56组生产模块回放和diff check通过。健康60fps窗口保持600张唯一图像且探测少于110次/10秒；健康转45/50Hz后保持至少599张唯一图像/10秒。持续稀疏回调及171ms准备/25ms首导入场景保持600张唯一图像/10秒。交付、统计口径与设备待验见[报告](../reports/2026-11-05-render-input-latency-optimization.md)。
