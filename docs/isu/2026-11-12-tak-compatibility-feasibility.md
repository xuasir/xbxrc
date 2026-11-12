# 官方 TAK 触控兼容可行性

官方 Layout/Manifest/Context、触控消息与控件行为具有可实现的兼容路径。推荐在现有 Vue + TypeScript 前端实现布局解释、触控状态机和绘制，将输出接入公共逻辑手柄输入；Rust 承接官方资源获取、认证、缓存和会话能力。当前结论是技术可行性，完整官方游戏覆盖以真实布局包与端到端验证为依据。

核对日期：2026-11-12。依据：[布局协议](../references/better-xcloud-touch-layout-protocol.md)、[引擎实现](../references/tak-babylon-touch-engine.md)、[源码与隔离回放](../../dev-docs/validation/2026-11-12-tak-layout-and-engine/README.md)。

## 问题范围

兼容目标分为官方文件和命令的可消费性、控件输入行为的一致性，以及最终视觉呈现。游戏动态切换布局、修改 Context、开启 layer 后，触控界面应产生匹配的虚拟输入；视觉实现可由 Vue 配合 SVG/Canvas 承担。精确的官方动画和像素呈现属于进一步的视觉覆盖目标。

## 当前基础与约束

| 现有能力 | 已核对的起点 | 需要补齐的部分 |
| --- | --- | --- |
| 标题输入配置 | Rust `input_configs()` 查询 `/v2/titles/inputconfigs`，RPC 保留 config JSON | 提取 supportedTabs、解析版本并下载实际布局包 |
| 消息接收 | MessageChannel 处理 titleinfo，并发出 channel.message | 路由五种 touchcontrols 命令、处理 scope 与 Context 更新 |
| 虚拟手柄 | InputController.setVirtualState、GamepadDriver 和 InputService 已连接 | TAK 控件贡献合并、完整状态适配和释放闭环 |
| Pointer/Mouse 编码 | InputPacketEncoder 包含相关报文编码 | 发送服务、宿主能力和真实服务端行为验证 |
| 触控运行时 | 桌面 runtime 声明 touch_surface=false；握手发送 touchInputEnabled=false | 实际触控界面、能力声明、版本协商和生命周期 |

现有活跃源码中的触控基础集中于能力模型、虚拟输入和编码；TAK 布局解释器与控件引擎需要新增。公共认证、Xbox API、会话和系统集成继续由 Rust 承担，UI 继续使用 Vue + TypeScript。

源码：[配置查询](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbox-webapi/src/session_api.rs:242)、[配置 RPC](/Users/guo.xu/Documents/code/games/xbxrc/src/shared/rpc/contract.ts:159)、[消息初始化](/Users/guo.xu/Documents/code/games/xbxrc/src/player/protocol/channels/MessageChannel.ts:86)、[虚拟输入](/Users/guo.xu/Documents/code/games/xbxrc/src/shared/gamepad/input-controller.ts:227)、[桌面能力](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/mods/streaming/context.rs:286)。

## 可选范围

| 范围 | 价值 | 工作量来源 |
| --- | --- | --- |
| 基础协议与常用控件 | 官方格式与社区模板共用，建立首批可玩布局 | schema、引用、槽位、基础控件、layer 与输入聚合 |
| 完整官方布局行为 | 游戏原有专用触控、动态状态和资源直接消费 | takx、历史版本转换、Context、更多控件、语言/DPI 资源和宿主能力 |
| 官方视觉细节 | 动画、图标与复杂图形进一步贴近官方 | 渲染覆盖、资源完整性、性能与视觉对照 |

原发布脚本采用内部依赖注入与网页宿主回调，原样复用需要核对全部依赖和宿主装配。自有兼容解释器可以将现有源码快照和回放作为语义参考，保持模块边界与升级节奏清晰。

## 推荐方向

首版选择官方数据与行为兼容：支持按钮、摇杆、方向键、触摸板、action 组合、layer、Context 引用和五种布局消息；使用一个控件贡献聚合器输出完整逻辑手柄状态，再接公共虚拟输入。schema 版本声明应与实际通过验证的覆盖一致。

官方布局资源沿 Rust 数据层补齐下载、解包、Manifest、locale/DPI 和缓存；游戏状态与 UI 布局解释按现有运行时边界装配。用社区 Forza 模板和实际官方 takx 包建立首批验证样本，随后扩大 throttle、传感器、相对鼠标与原生触摸覆盖。

41 项隔离回放提供已有语义证据，其中包含两项 responseCurve 异常复现。兼容实现应记录版本差异，保证轴输出为有限值，并为状态更新、同键多控件释放、布局切换和退出归零建立行为验证。

## 待补证据

- 官方 getTouchAdaptationBundle 的下载入口、返回形式与当前会话认证在 xbxrc 中的实际接通。
- 云游戏和主机串流中 supportedTabs 的实际返回、布局命令时序与代表性 takx 样本。
- 开启已实现能力后，服务端的命令交付与实际输入响应。
- WebView 的触摸、取消、窗口隐藏、实体手柄接管、重连与资源释放表现。

官方下载链在已核对网页中表现为 `fetchTouchAdaptationBundle(id) → client.getTouchAdaptationBundle(user,id)`；当前仓库具备输入配置查询，布局包下载接口仍需实现和验证。

## 后续候选任务

1. simple：核对官方 bundle 获取链并取得代表性原件，记录版本、资源和动态状态样本。
2. complex：制定兼容首版 RFC，固定支持矩阵、资源合同、消息/状态时序和输入归属，再实现基础引擎。
3. complex：按真实游戏样本扩展控件、传感器、鼠标及原生触摸覆盖，完成端到端验收。

建议以“官方文件与消息可消费、常用控件行为可验证、输入沿公共链交付”为首版完成条件。
