# PC 主机游戏与“我的”四页交付报告

- 日期：2026-11-05
- 关联：[RFC](../rfcs/2026-11-05-pc-console-library-my-pages.md)
- 状态：代码接入与本地验证完成；线上服务覆盖率和实机体验保留用户验收。

## 交付范围

- 主机页增加当前设备的“已安装游戏”，包含名称搜索、存储筛选、排序、容量分区和游戏详情；串流沿现有设备入口。
- 一级导航下组按“我的 / 控制器 / 设置”排列；“我的”以现有 `SettingSidebar` 承载账户与好友、成就、统计、相册。
- 账户与好友支持可见资料、状态筛选、好友详情及游戏历史；游戏详情复用成就列表、秘密成就主动揭示、双方成就对比、累计时长和相册。
- 相册提供截图/录像两源分页、游戏筛选、预览、资源重新查询和主动系统保存。保存服务重新查询媒体地址，使用临时文件完成下载后落到用户选择的位置。
- Rust Xbox API 保留 HTTP 与 DTO；桌面 data service 处理账号上下文、并发、超时和局部失败；Vue 通过 typed RPC 消费结果。
- 主机封面按商品 ID 补充，复用当前 catalog-flow 的分批与图片归一化、桌面既有市场/语言范围。Tile 优先，Poster 回退；8 秒总预算到期时保留安装数据并提示封面查询状态。
- 自动与手动刷新保留已浏览页数，使用服务新游标重读；后续页失败保留原有完整内容。设备重新激活/刷新获取当前事实，切换设备/账号和离开页面隔离迟到响应，查询失败后串流入口等待有效状态。
- 好友详情刷新同步更新个人资料与本人关系列表；游戏概览保留安装、时长的局部错误。相册保存反馈检查账号与当前媒体身份。

## 样式与可读性

- 延续当前壳层、主题、2–8px 圆角、方形游戏封面和 16px 网格间距。
- 返回采用透明箭头与文字，关联入口采用透明文字操作；刷新使用带可访问名称的 36px 图标，加载时旋转，遵循减少动态效果偏好。
- 账户、累计统计和存储信息使用透明容器、留白与细分隔线；容量进度采用 4px 细条。窄窗口的主机返回/标题同行，摘要压紧，游戏列表更早进入首屏。
- 页面与分区标题分级；游戏详情顶栏显示当前分类，游戏名保留在游戏资料区。
- 正文、好友/游戏名称和时长数值为 16px；时间、进度、容量说明和媒体时长为 14px。辅助文字使用当前主题辅助色 80% 与主色 20% 的实色混合，行高 1.5。
- 焦点沿几何导航引擎及现有轮廓；详情默认焦点优先落在返回入口，返回列表恢复原项目。
- 刷新、筛选删除当前项或末页移除“更多”入口时，焦点交给邻近有效项；头像菜单默认聚焦“进入我的”，返回动作关闭菜单。

## 验证证据

| 检查 | 结果 |
| --- | --- |
| `cargo test -p xbox-webapi` | 30 项通过，覆盖成就/历史/媒体分页参数、安装游戏归类、身份、复合成就要求和时长缺值 |
| `cargo test -p xbox-cloud-catalog-flow` | 4 项通过，覆盖商品分批与图片归一化 |
| `cargo test -p xbxrc mods::data:: --lib` | 11 项通过，包含主机商品封面身份匹配、缺值保留及现有 catalog scope/cache 合同 |
| `cargo check -p xbxrc` | 通过 |
| `cargo fmt --all -- --check` | 通过 |
| `pnpm test src/pages/library src/components/layout/shell-context.test.ts src/components/settings/keyboard-editing.test.ts` | 5 文件 19 项通过；包含账号/设备迟到响应隔离、重新激活、设备失败后串流可用性、缓存并发合并、分页去重、已浏览范围刷新和媒体共同覆盖范围 |
| `pnpm lint:fix` / `pnpm typecheck` / `pnpm build` | 通过 |
| `git diff --check` | 通过 |
| 真实 Vue 组件 + 隔离 RPC 数据预览 | 深浅色、1280×720 与 960×540 检查通过；账户、好友详情、游戏详情、统计、相册和主机安装页均可呈现 |
| 搜索与返回 | 安装列表搜索 FORZA → 打开游戏 → 返回后保留搜索值与原游戏焦点 |
| 相册关闭 | 打开截图预览 → Esc 关闭 → 恢复原缩略图焦点 |
| 可读性 | 浏览器 computed style 确认辅助文字 14px/21px 行高、列表名称 16px、时长 16px/24px 行高；深色统计与浅色相册重新目视核验 |
| 已浏览范围与末页焦点 | 统计加载第二页后共 18 款，刷新后仍为 18 款；末页入口移除后焦点保留在列表附近。相册两源合并后共 12 项，末页焦点回到原有缩略图 |
| 示例录像与保存反馈 | 录像播放时 paused=false、readyState=4、error=null；模拟保存返回后显示结果；Esc 关闭恢复原录像缩略图焦点。系统下载与保存窗口保留实机验收 |
| 四档密度 | 1600×900 comfortable、1280×720 standard、960×540 compact、800×540 narrow，页面整体横向溢出为 0；comfortable/narrow 检查统计页，standard 检查账户/主机/详情，compact 检查相册 |
| 头像入口与详情返回 | 菜单默认聚焦“进入我的”，进入账户与好友成功；FORZA HORIZON 11 详情返回后保留 FORZA 搜索与 installed-10 焦点 |

预览截图使用隔离样例数据：

- [深色统计页，1280×720](/Users/guo.xu/.codex/visualizations/2026/11/05/01a1aa58-f9f3-7a80-aeb4-30e424290605/pc-statistics-dark-1280.png)
- [浅色相册页，960×540](/Users/guo.xu/.codex/visualizations/2026/11/05/01a1aa58-f9f3-7a80-aeb4-30e424290605/pc-album-light-960.png)
- [本次深色账户页，1280×720](/Users/guo.xu/.codex/visualizations/2026/11/05/01a1aa58-f9f3-7a80-aeb4-30e424290605/pc-account-readable-dark-1280.png)
- [本次深色主机页，1280×720](/Users/guo.xu/.codex/visualizations/2026/11/05/01a1aa58-f9f3-7a80-aeb4-30e424290605/pc-console-readable-dark-1280.png)
- [本次浅色相册页，960×540](/Users/guo.xu/.codex/visualizations/2026/11/05/01a1aa58-f9f3-7a80-aeb4-30e424290605/pc-my-readable-light-960.png)

## 实机验收边界

- 使用真实账号核验公开/受限好友、成就与累计时长覆盖率、账号切换、限流与重新登录。
- 使用实际主机核验安装列表、存储字段、休眠/离线状态以及原串流入口。
- 使用 Tauri WebView 核验 Xbox 网络原图、录像播放/停止、URL 过期和系统保存窗口；物理手柄、设备启动与串流按仓库约定由用户验收。
- 周/月趋势、指定游戏启动、远程安装卸载以及消息/派对继续按设计作为独立后续范围。

## 2026-11-05 界面反馈修正

- 主机页操作区采用同一 flex 行，串流入口后排列“已安装游戏”，间距 12px。安装入口采用随深浅主题变化的中性灰底色，常态 border 为 0px，聚焦继续使用应用现有轮廓。
- 安装列表为空或当前无可读取游戏时，内容区统一呈现一个居中的“暂无数据”。容量分区、筛选器、条目数量及分项通知在有安装数据时呈现；有数据后的筛选空结果保留筛选区。
- 游戏详情返回入口和下方分类标题之间增加 16px 留白，聚焦轮廓具有独立安全空间。
- 修复分类切换后焦点回跳“返回”：鼠标和 LT/RT 操作保持目标分类焦点，内容区恢复该分类独立滚动位置，新分类从顶部开始；过期恢复跳过。分类路由更新显式携带当前游戏路由名、参数及来源/好友 query。
- 新增三项分类切换回归，修复前均复现返回焦点错误，修复后通过。`pnpm test src/pages/library src/components/layout/shell-context.test.ts src/components/settings/keyboard-editing.test.ts` 当前 6 文件 22 项通过；`pnpm lint:fix`、`pnpm typecheck`、`pnpm build`、`git diff --check` 通过。
- 浏览器核验：1280×720 深色主页入口同排且底色区别于绿色串流按钮；960×540 浅色空列表与分项失败均只有一个“暂无数据”。个人详情连续六次、主机详情连续四次切换时，标题/分类高亮/焦点一致，游戏身份和来源保留；好友详情切换保留 targetXuid，浏览器错误日志为空。

本次真实 Vue 组件与隔离样例数据截图：

- [主机操作入口修正](/Users/guo.xu/.codex/visualizations/2026/11/05/01a1aa58-f9f3-7a80-aeb4-30e424290605/pc-console-actions-fixed-dark-1280.png)
- [安装页统一空态](/Users/guo.xu/.codex/visualizations/2026/11/05/01a1aa58-f9f3-7a80-aeb4-30e424290605/pc-console-empty-fixed-light-960.png)
- [详情返回聚焦间距](/Users/guo.xu/.codex/visualizations/2026/11/05/01a1aa58-f9f3-7a80-aeb4-30e424290605/pc-game-return-gap-fixed-dark-1280.png)

## 2026-11-05 页面标题字号收紧

- 新增页抬头从 title-lg 28px 调整为 title-md 20px，字重使用 medium 500；账户名称与游戏详情名称使用 body-lg 18px。正文 16px、辅助信息 14px、二级菜单和分区标题沿既有层级。
- 合并页头与内容工具栏 h1 规则，主机安装页、账户、成就、统计、相册及游戏详情共用这一限制。
- 深色 1280×720 详情 computed style 确认页头 20px/500、游戏名 18px/500；浅色 960×540 账户页确认页头 20px、名称 18px、正文 16px、辅助信息 14px，主机安装页抬头为 20px。`pnpm lint:fix`、`pnpm typecheck`、`pnpm build`、`git diff --check` 通过。
- [收紧后的详情标题，隔离样例预览](/Users/guo.xu/.codex/visualizations/2026/11/05/01a1aa58-f9f3-7a80-aeb4-30e424290605/pc-detail-title-compact-dark-1280.png)
