# 设置页展示 Schema 重组 RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 未完成
- Current State: planned
- Owner: Codex
- Last Updated: 2026-05-11

## Background

- 当前设置页已经具备稳定的配置主线：Tauri 侧通过 `rpc.config.getGroups/set` 提供配置读写，renderer 侧通过 [`src/shared/config/domain-definition.ts`](/Users/guo.xu/Documents/code/games/xbxrc/src/shared/config/domain-definition.ts) 维护字段定义、控件类型和分组顺序。
- 当前页面结构直接复用 config group 作为展示层导航，[`Setting.vue`](/Users/guo.xu/Documents/code/games/xbxrc/src/pages/Setting.vue) 再叠加 `expert reset`、`input tools`、`single select/value/display` sheets、重启提示等额外状态，导致页面承担了多种对象语义：
  - 配置项
  - 工具入口
  - 危险操作
  - 风险提示
  - 空分组占位
- 这套结构在代码层是可运行的，在用户体验层存在持续摩擦：
  - 顶层导航偏内部模块边界，弱于用户任务心智
  - `input` 页混合“配置”和“工具”
  - `streaming.expert` 以 section 形式出现，高风险项进入成本偏低
  - `xcloud` 当前是空顶层页，[`src-tauri/src/mods/config/grouping.rs`](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/mods/config/grouping.rs:36) 已明确不返回独立策略字段
  - `force_region_ip` 这类跨语义字段目前只能依赖 group 顺序硬塞到页面中

## Goal

- 在不改配置存储协议、不改字段 key、不改 Tauri RPC 合同的前提下，为设置页新增一层独立的展示 schema。
- 让设置页导航、section、工具入口、危险区、特殊标签都由展示层定义驱动，而不是直接由 config group 反推。
- 将当前设置体验重组为更符合用户任务的结构，降低查找成本和误操作概率。
- 为后续设置搜索、显示规则、受众分层、风险 gating、效果标签建立稳定扩展点。

## Scope

- In scope:
  - [`src/pages/Setting.vue`](/Users/guo.xu/Documents/code/games/xbxrc/src/pages/Setting.vue)
  - [`src/pages/settings/SettingSectionList.vue`](/Users/guo.xu/Documents/code/games/xbxrc/src/pages/settings/SettingSectionList.vue)
  - [`src/pages/settings/SettingSidebar.vue`](/Users/guo.xu/Documents/code/games/xbxrc/src/pages/settings/SettingSidebar.vue)
  - [`src/pages/settings/SettingInputToolsSection.vue`](/Users/guo.xu/Documents/code/games/xbxrc/src/pages/settings/SettingInputToolsSection.vue)
  - [`src/pages/settings/setting-types.ts`](/Users/guo.xu/Documents/code/games/xbxrc/src/pages/settings/setting-types.ts)
  - [`src/shared/config/domain-definition.ts`](/Users/guo.xu/Documents/code/games/xbxrc/src/shared/config/domain-definition.ts) 中与字段定义复用边界有关的部分
  - 设置页 i18n、导航结构、展示层数据模型、受众分层和风险分区
- Out of scope:
  - 修改 `settings_store`、`ConfigService`、`ConfigStorageRepository` 的存储协议
  - 修改配置 key、默认值、规范化规则或现有 RPC 方法签名
  - 引入新的客户端技术路线或第二套设置页运行时
  - 首轮内直接实现全文搜索、云同步、多端设置迁移

## Current Problem Breakdown

### 1. 配置域与展示域绑定过紧

- 当前 [`CONFIG_GROUP_DEFINITIONS`](/Users/guo.xu/Documents/code/games/xbxrc/src/shared/config/domain-definition.ts:152) 同时承担“配置归类”和“页面展示结构”两层职责。
- 一旦页面希望按用户任务重组，前端只能继续往 `Setting.vue` 和 section 组件加特判。

### 2. 同一页面混入多类对象

- `SettingSectionList` 当前主要渲染字段行。
- `SettingInputToolsSection` 额外挂入调试视图和手柄映射。
- `streaming.expert` 还嵌了 reset 动作与风险提示。
- 这些对象具备不同交互语义，继续放在统一 `row` 模型下会拉高后续复杂度。

### 3. 顶层导航不贴近用户任务

- 现有顶层页是 `app / streaming / host / xcloud / input`。
- 用户心智更接近：
  - 通用
  - 串流体验
  - 连接与主机
  - 输入设备
  - 高级与诊断
- 当前 `host` 和 `xcloud` 的展示价值都偏低，`xcloud` 甚至没有独立内容。

### 4. 特殊项散落在页面控制器里

- 专家重置、危险提示、重启确认、工具入口都在 [`Setting.vue`](/Users/guo.xu/Documents/code/games/xbxrc/src/pages/Setting.vue) 或 [`SettingSectionList.vue`](/Users/guo.xu/Documents/code/games/xbxrc/src/pages/settings/SettingSectionList.vue) 中硬编码。
- 这会让后续任一新需求都继续推高顶层组件复杂度。

## Proposed Direction

### A. 保留配置域，新增展示域

本 RFC 采用方案 B：

- 配置域继续负责：
  - 配置 key
  - 字段定义
  - 控件类型
  - 默认值与规范化
  - `rpc.config.getGroups/set`
- 展示域新增一层 `SETTING_PAGE_DEFINITIONS` 或同等职责模型，负责：
  - 顶层导航
  - section 组织
  - item 排序
  - 工具入口
  - 危险区
  - 受众层级
  - 生效标签
  - 可见性规则

### B. 新的顶层信息架构

建议首版固定为 5 个页面：

1. 通用
   - `locale`
   - `theme`
   - `fullscreen`
   - `ui_haptics`
   - `ui_audio`
   - `background_keepalive`

2. 串流体验
   - `resolution`
   - `xhome_resolution`
   - `video_format`
   - `display_options`
   - `enable_audio_control`
   - `xhome_bitrate_mode`
   - `xhome_bitrate`
   - `xcloud_bitrate_mode`
   - `xcloud_bitrate`
   - `audio_bitrate_mode`
   - `audio_bitrate`
   - `performance_style`

3. 连接与主机
   - `preferred_game_language`
   - `force_region_ip`
   - `ipv6`
   - `xhome_turn_fallback`

4. 输入设备
   - `polling_rate`
   - `vibration`
   - `vibration_strength`
   - 手柄概览
   - 输入测试工具
   - 手柄映射工具

5. 高级与诊断
   - `stream_runtime_mode`
   - `debug`
   - `runtime_trace_mode`
   - `use_vulkan`
   - `use_msal`
   - `server_url`
   - `server_username`
   - `server_credential`
   - 专家重置

### C. 展示项模型从单一 `field row` 扩成 schema item

建议新增统一 item 类型：

- `field`
  - 绑定现有 config key
  - 继续走 toggle / singleSelect / textInput / numberInput / displayOptions
- `tool`
  - 打开输入调试、映射编辑等操作
- `action`
  - 重置专家配置、清理缓存、重启应用等具副作用动作
- `notice`
  - 风险提示、说明块、段落型提醒
- `groupSummary`
  - 设备概览、连接状态概览等展示块

这样可以把当前散落在组件分支里的特殊逻辑收成声明式结构。

### D. 展示 schema 应带的元数据

每个展示项建议具备以下元信息：

- `audience`
  - `basic`
  - `advanced`
  - `expert`
- `effect`
  - `instant`
  - `restartRequired`
  - `nextSession`
- `riskLevel`
  - `normal`
  - `warning`
  - `danger`
- `visibleWhen`
  - 基于运行时条件或其他设置值控制显示
- `keywords`
  - 为后续搜索预埋

### E. 专家区与危险区改成显式分区

- `server_url / server_username / server_credential` 不再作为普通 section 常驻展开。
- 这些项放入“高级与诊断”的 `danger zone`，默认收起，通过显式确认进入。
- `expert reset` 从 section 标题侧按钮改成危险区动作项，由 schema 驱动显示。

### F. 输入页将“设置”和“工具”拆层

- `polling_rate / vibration / vibration_strength` 继续保留为配置项 section。
- `输入测试`、`手柄映射`、后续可能加入的 `设备诊断`、`校准` 都归到工具 section。
- 如果保留 runtime snapshot 卡片，视为 `groupSummary`，处于页面顶部。

### G. 空顶层页收敛

- `xcloud` 顶层页移除。
- `host` 顶层页合并进“连接与主机”。
- `force_region_ip` 仅在一处展示，避免多处重复入口。

## Data Model Sketch

建议新增以下前端结构，命名可在实现时微调：

```ts
type SettingPageKey
  = | 'general'
    | 'streamingExperience'
    | 'connectionHost'
    | 'inputDevices'
    | 'advancedDiagnostics'

type SettingSchemaItem
  = SettingFieldItem
  | SettingToolItem
  | SettingActionItem
  | SettingNoticeItem
  | SettingSummaryItem

interface SettingFieldItem {
  kind: 'field'
  fieldKey: string
  audience?: 'basic' | 'advanced' | 'expert'
  effect?: 'instant' | 'restartRequired' | 'nextSession'
  riskLevel?: 'normal' | 'warning' | 'danger'
  visibleWhen?: string
}
```

首轮实现重点在职责拆分，字段数量保持克制，避免一开始把 schema 做成新一轮难维护 DSL。

## Module Impact

### 1. `domain-definition.ts`

- 保留字段定义职责。
- 逐步剥离“页面展示顺序”职责。
- `CONFIG_GROUP_DEFINITIONS` 后续可退化为兼容层，服务 `getGroups` 返回值和字段域组织。

### 2. `Setting.vue`

- 从“集中处理所有特判的页面控制器”收敛为：
  - 拉取配置状态
  - 依据展示 schema 构造页面 view model
  - 打开已有 sheets
  - 分发工具和动作事件

### 3. `SettingSectionList.vue`

- 从“字段列表组件”升级为“schema item 列表渲染器”。
- 通过 item `kind` 渲染 `field/tool/action/notice/summary`。

### 4. `SettingInputToolsSection.vue`

- 当前职责会被吸收到统一 schema 渲染链中。
- 组件本身可保留为输入页专用 summary/tool renderer，也可按实现复杂度逐步拆薄。

### 5. 导航与 i18n

- 新增 page/section/item 级文案键。
- 焦点路径继续复用现有 spatial nav 体系，避免重新发明导航模型。

## Plan

1. 定义展示 schema 与 item 类型，明确配置域和展示域边界。
2. 先在不改存储与 RPC 的前提下完成顶层导航与 section 重组。
3. 将 `input tools`、`expert reset`、风险提示吸收到 schema item。
4. 收掉空 `xcloud` 页与 `host`/`xcloud` 的冗余顶层结构。
5. 为设置项增加统一的生效标签和风险分区。

## Validation

- [ ] 设置页展示结构已从 config group 直驱改为独立 schema 驱动
- [ ] `rpc.config.getGroups/set`、配置 key、默认值与规范化逻辑保持不变
- [ ] `xcloud` 空顶层页移除，`host/xcloud` 相关项已并入“连接与主机”
- [ ] `input` 页已拆成“配置项 + 工具 + 概览”三类对象
- [ ] 专家项和危险操作已进入显式危险区，默认不与普通设置并列展开
- [ ] 现有手柄导航、sheets 编辑流和重启确认流继续可用

## Risks

- 如果展示 schema 设计过度抽象，设置页会从“特判分散”变成“配置化过重”，维护成本同样会上升。
- 如果 `CONFIG_GROUP_DEFINITIONS` 与新展示 schema 在迁移期双向演化，容易出现字段归属分叉。
- 输入页工具、概览和配置项混排时需要额外注意手柄导航顺序，否则易引入焦点跳转回归。

## Progress

- [x] Step 1: 已完成问题收敛与方案选择，确认采用“保留配置域，新增展示 schema”的路径
- [x] Step 2: 已完成 RFC 初稿，明确新的顶层信息架构、item 模型和模块边界
- [ ] Step 3: 待把 schema 类型和页面 view model 落到前端实现
- [ ] Step 4: 待补导航、交互和文案层的回归验证

## Execution Notes

- Date: 2026-05-11 | Status: planned
- Update: 基于现有设置页实现、配置域定义和 Tauri 分组返回结构，整理出设置页按展示 schema 重组的 RFC 初稿。
- Decision: 采用方案 B，保留配置存储和字段定义主线，在前端单独新增展示 schema，按用户任务重组设置页。
- Decision: 首轮实现保持 `rpc.config.getGroups/set`、字段 key、默认值和现有 sheets 组件稳定，页面重构聚焦信息架构与渲染模型。
- Decision: `xcloud` 空顶层页收掉，`host` 顶层语义并入“连接与主机”，输入页拆成配置、概览、工具三类对象。
- Risk/Blocker: 展示 schema 与现有 `CONFIG_GROUP_DEFINITIONS` 的迁移边界需要在实现前再确认一次，避免短期内出现双源维护。
