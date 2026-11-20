# iOS 三段式布局与 PC 透明风格

## 结果

完成以 PC 为蓝本的 iOS 三段式布局，按用户提供的 Xbox 移动端参考图校准文字、内容明度与留白。顶部身份/返回、当前页标题和页面操作固定，中部内容纵向滚动，底部五个 Xbox 体系图标固定，以轻透模糊背景体现边界。设置入口保留完整描述，窄屏采用统一紧凑字号，开关独立缩小并与标签垂直居中。

执行沿 [迁移 RFC](../rfcs/2026-11-12-ios-ipad-pc-capability-migration.md) 的既有授权与 [视觉合同](../../iosapp/DESIGN.md)，由主窗口直接完成。

## 实现

- 公共字号在常规宽度采用一级标题 24pt/600、子页/分区 18pt/600、正文 14pt、辅助文字 12–13pt。小于 360pt 的实际页面宽度采用一级标题 22pt、子页 17pt、分区 16pt、正文 13pt、说明 11–12pt。`AppResponsiveTypographyModifier` 统一投影窗口密度，工具栏与 `appFont` 共用档位，Dynamic Type 继续按平台缩放。
- 五导航以 26pt 直接复用 `src/assets/nav/` 的 `xhome.svg`、`xcloud.svg`、`my.svg`、`ctrl.svg`、`setting.svg`：主机为 Xbox 远程游玩、云游戏为 Game Pass、控制器为 Xbox 手柄轮廓。SVG 路径保留，Xcode template rendering 提供主题着色；选中项使用主题正文色。二级入口和常用动作采用微软 Fluent 24px 资源，MIT 许可随应用保留。
- 底栏采用 `ultraThinMaterial`，深色覆盖率 24%、浅色 36%，延伸至底部安全区；五项等宽，行高 56pt、上下留白各 8pt，宽窗整体限宽 560pt。顶部返回和操作采用纯图标，隐藏系统 shared background。
- 设置根入口采用至少 60pt 的紧凑双行、8pt 纵向内边距与 12pt 行距，完整描述可见；右侧值采用 12pt 常规字重，窄屏为 11pt。`AppSettingsValueStyle` 使用垂直中心对齐，`AppSettingsToggleStyle` 将原生开关视觉缩至 82%，标签沿公共字号，控件框与行保留至少 44pt 操作区域，Picker 和 Toggle 自行提供内边距。
- 页面边距 16pt，普通行/网格间距 16pt、分区间距 32pt，宽窗设置内容限宽 760pt。公共 `appSurface` 采用 PC 透明渐变装配，深色画布为 `#17191B`，内容合成明度接近参考图 `#26282C`；普通深色表面覆盖率 82%/78%，浮层 96%/94%，浅色普通表面 96%/92%、浮层 98%/96%，附细边框和轻阴影。卡片、登录、成就、菜单与串流操作层统一装配。
- 云游戏栏目采用 152pt 方形封面，游戏名与辅助信息置于封面下方，栏目提供“查看全部”。环境光使用 PC 的绿色、青色与蓝色来源，在页面上部柔和过渡。
- 纯图标返回通过 `dismiss` 更新原生栈；边缘返回沿 UINavigationController 转场，进入时接入手势门禁，退出恢复原始代理。已访问页面保留返回栈和滚动位置，输入与会话生命周期沿现有实现。

## 验证

- 最终 iPhone Simulator 构建及 131 项 XCTest 全部通过；日志 `/tmp/xbxrc-ios-xbox-compact-controls-phone.log`，结果 `/tmp/xbxrc-ios-xbox-compact-controls-phone.xcresult`。
- iPad 复用同一最终产物，12 项迁移回归全部通过；日志 `/tmp/xbxrc-ios-xbox-compact-controls-ipad.log`，结果 `/tmp/xbxrc-ios-xbox-compact-controls-ipad.xcresult`。包含原生边缘返回、代理恢复与重复进入子页。
- 布局绘制覆盖 10 组尺寸/方向/主题/辅助字号场景，320–1366pt；iPhone 与 iPad 各生成 110 张原生页面截图。代表性截图见 [效果目录](../../dev-docs/validation/2026-11-16-ios-three-part-style)。窄屏和常规宽度截图确认公共字号档位已生效，开关与标签居中，完整设置描述保留。
- 五个导航 SVG 与 PC 原始资源逐项一致，template / vector 元数据有效；Swift 格式校验、`pnpm lint:fix` 与 `git diff --check` 通过。应用内显式 Glass 调用扫描为零，底栏保留用户指定的单一模糊材质。
- 最终截图确认：Xbox 导航图形来源一致，顶部操作直接透出背景，深色内容明度和文字层级清晰；窄屏、辅助字号和宽窗分别沿公共策略适配。

## 验收范围

当前 Xcode 与 Simulator Runtime 为 27.0，应用最低部署目标保持 iOS 26。截图主要覆盖未登录入口、设置、控制器和页面壳层；真实账户头像、完整游戏封面与 iOS 26 实机观感沿迁移任务统一验收。
