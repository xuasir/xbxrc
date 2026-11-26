# iOS TAK 本轮能力范围

本轮延续 2026-11-25 已授权的手柄触屏能力补齐。用户最后明确暂缓鼠标、键盘和传感器；这些布局继续保留逐字段诊断及保存门禁。

| 能力 | 本轮交付 |
| --- | --- |
| Context / definitions | Rust 有界展开静态引用，保留原始 Context.state 来源；Swift 从同一 document 重新归一化 |
| 状态补丁 | Base64/UTF8 解码，JSON Patch 逐项执行；成功项保留，失败项生成 warning；整体无效或展开后的能力检查失败保留当前有效布局 |
| 游戏消息 | showlayout、showlayoutv2、showtitledefault、hide、patchstate 沿现有 message DataChannel 与会话布局控制器串行执行；同包切换保留当前状态 |
| 输入释放 | 动态隐藏/替换释放受影响控件；其余多指/sticky 状态保留；hide、前后台抑制、布局替换及结束复用当前输出归零 |
| 手柄连发 | turbo 按毫秒间隔开/关，复用 CADisplayLink 时钟与可靠 input 通道；实际采样粒度随屏幕刷新率与既有输入轮询变化 |
| 色板 | 官方 11 个系统颜色及布局命名颜色；default/highContrast 变体、hex 与透明度 |
| 样式 | DPad fill/方向渐变/描边、joystick outline/方向指示与 knob、throttle 两段轴描边/指示/cap、图标 action/none 标签及状态继承 |
| 图标 | 官方 v4.1 全量173个名称映射 iOS 原生语义符号及3类本地向量轮廓；同类编号变体共享图形，微软原始轮廓保留为视觉对齐边界 |
| 官方/社区 | 现有 TAK 包与 better-xcloud 包装复用统一引用解析、能力扫描与触屏执行 |
| 门禁 | 未知版本/动作/样式、缺失图片、非法布尔状态/颜色继续提供具体字段原因 |

合同依据为 fixtures/layout-schema.json、fixtures/context-schema.json 与前轮冻结 [TAK 源码证据](../2026-11-12-tak-layout-and-engine/README.md)。整套任意游戏布局的兼容性由使用的具体字段决定。真实地下城布局包、游戏服务端动态消息与实机多指串流按仓库规范由用户设备侧验收。
