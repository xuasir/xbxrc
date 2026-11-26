# iOS TAK 动态状态、连发与样式补齐 Report

## 交付

按[本轮 RFC](../rfcs/2026-11-25-ios-tak-completeness.md)完成 Context/state 动态更新、游戏布局消息与手柄连发。Rust 保留静态 definitions、原始状态来源、图片映射和 JSON Patch，Swift 复用现有多指输入、布局控制器及 UIKit/CA 绘制。

- 接通 showlayout、showlayoutv2、showtitledefault、hide、patchstate；命令沿现有 message DataChannel 串行消费，受32条队列预算和会话/目录/选择版本约束。同包布局切换保留当前状态，目录升级和会话结束释放旧状态。
- 动态隐藏或替换释放对应控件输入；其余触点/sticky保持当前状态。非法补丁、引用展开失败或状态类型不兼容时继续使用当前有效布局，并记录具体原因。
- turbo 按官方毫秒间隔交替输出，复用 CADisplayLink；多指/重复动作贡献独立释放，抑制和结束归零。实际脉冲采样粒度随显示刷新率与既有输入轮询变化。
- 支持11个系统颜色引用、布局命名颜色、default/highContrast变体、DPad填充/方向渐变/描边、摇杆outline方向指示、throttle轴描边与cap、图标action/none标签和状态样式继承。173个官方图标名映射到原生语义符号，剑/镐/拳头使用本地向量；编号变体共用图形。

用户在关联聊天最后明确暂缓鼠标、键盘和传感器；这些输入继续保持能力诊断和保存门禁。完整范围见[能力矩阵](../../dev-docs/validation/2026-11-25-ios-tak-completeness/support-matrix.md)。

## 最小验收

Rust **36/36**通过。iPhone 18 Pro / iOS 27.2 定向验收最终 **9/9**通过：8项单元与1条同时覆盖官方/社区的UI，包含Context隐藏释放、同包状态保留、5种游戏命令、连发时序、色板/渐变和正式游戏入口预览保存。15个Swift文件格式、cargo fmt、pnpm lint:fix、串流边界和差异检查通过。

首次启动诊断定位到模块重命名后的UniFFI API checksum，重新生成桥接并重建后解决。主轮8项通过，唯一图标用例发现3类SF Symbols缺少图形资源，补上向量后仅补验这一项并通过。对应摘要、失败诊断、3张最终截图、源码哈希与清理记录均在[证据目录](../../dev-docs/validation/2026-11-25-ios-tak-completeness/README.md)。

## 设备侧验收

UI 使用官方格式和社区来源夹具，真实地下城游戏包、游戏服务端动态消息与实机串流多指操作由用户设备侧验收。原生语义图形与微软原始轮廓的精确视觉对齐属于当前明确的兼容边界。建议安装更新后的 iOS 客户端，重新保存地下城第二布局，再检验游戏内操作与布局切换。

本轮临时根已清理，覆盖DerivedData、3份xcresult、诊断导出、重复截图与工具日志；临时分配量约857.8MiB，可用磁盘空间增加约781.2MiB，清理后剩余88.50GiB。最终证据与已有共享构建缓存保留。
