# iOS TAK 通用协议方案证据

日期：2026-11-26。本轮完成协议提取、共性缺口分析和[实施 RFC](../../../docs/rfcs/2026-11-26-ios-tak-protocol-engine.md)。

## 范围

- 官方 v4.1 Layout schema 及 Context schema：沿用前轮冻结原件，提取结构、约束、引用、枚举和默认值。
- 社区修订 `4416b1ab07588a3244a5c02ce1fa5dfa64d8df74`：92 个游戏定义、75 个模板、85 份布局，全部模板均被游戏定义引用。
- TAK core 与 Babylon renderer：重新读取公开原件，SHA-256 与 2026-11-12 冻结清单一致。
- 官方图集：经宿主模块932383和发布runtime恢复五档DPI数据，名称一致，均包含234个图标条目。
- 当前 iOS 归一化、图标校验及绘制代码：按源码字面合同定位 GTA6 图标的具体拦截点。

## 文件

| 文件 | 内容 |
| --- | --- |
| `community-source.zip` / `community-source.json` | 社区完整 touch-layouts JSON 原件、固定修订及逐文件哈希 |
| `official-runtime-source.zip` / `official-runtime-source.json` | 两份官方公开发布原件及哈希核对 |
| `atlas-host-source.zip` / `atlas-host-source.json` | 图集依赖和发布runtime原件 |
| `official-icon-atlas-source.zip` / `official-icon-atlas-source.json` | 官方五档DPI图集原件及逐文件哈希 |
| `official-icon-atlas-inventory.json` | 234个官方图标名称、五档一致性和图片哈希；图片原文保留在来源ZIP |
| `protocol-inventory.json` | 官方 schema 结构约束及运行时合同来源 |
| `community-inventory.json` | 模板、游戏引用、控件/样式/图标形态的批量提取 |
| `capability-gaps.json` | GTA6 及整库图标命名缺口和其他待对照形式 |
| `validation-summary.json` | 一轮 JSON/哈希/引用/文档冒烟结果和本轮边界 |
| `cleanup.json` | 本轮临时产物及最终保留项 |

## 已定位的共性问题

GTA6 定义 `touch-layouts/2035456122.json` 引用 `gta-vi/custom`。三个图标值 `leftTrigger`、`leftBumper`、`rightTrigger` 在当前 `TouchLayoutNormalizer.scanImageTree` 中命中图标待支持分支。已确认这三个名称和 `rightBumper` 全部存在于官方五档DPI图集，属于 iOS 校验遗漏。

官方 schema 列出173个语义图标，当前 iOS 目录为177个名称，官方宿主图集为234个名称。社区直接注入路径按官方运行时和真实图集执行，完整能力目录同时采集文件 schema 与运行时资源。

社区模板还使用 `rightBumper` 图标；四类名称分别出现 30、23、13、8 次，共 74 次。相同协议形式共享一次能力实现。

`touch-layouts/ids.json` 原件存在 JSON 语法错误，已保留来源诊断；75 个被引用模板和92 个按 TitleId 的游戏定义均可解析。`wolfenstein-3d` 的字符串 `toggle:"true"` 作为社区运行时行为对照项记录。

## 证据边界

结果来自协议提取和静态路径核对。RFC 中的统一编译、IR 消费及渲染改造进入实施阶段验收；当前应用能力继续以既有 Report 和实际实现为准。整库解析统计表达冻结数据的形态覆盖，行为覆盖由官方方法回放与原生输入/绘制验证证明。

用户补充确定宽容运行原则：可解释的输入正常执行，可选图标/样式采用默认回退，未知能力分级隔离，文档/资源预算与输入输出语义保持协议边界。图集中的嵌入PNG从形态清单中去重，原件保留在来源ZIP，减少重复内容约4.06MiB。

网络读取使用系统 curl 的证书校验；Python 自带证书链诊断后已切换到系统读取路径。源归档在内存处理，最终原件与清单直接归档。
