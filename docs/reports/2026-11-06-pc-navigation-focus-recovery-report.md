# PC 导航焦点恢复与窄屏底部导航 Report

## Summary

- 日期：2026-11-06；主窗口执行。
- Related RFC：[PC 全页面导航焦点恢复](../rfcs/2026-11-06-pc-navigation-focus-recovery.md)。
- 审计确认的五类焦点问题已修复，正式回归及实际 Vue 页面预览通过。
- 700px及以下按最新反馈采用贴底通栏毛玻璃一级导航，配合圆角上沿、细描边与柔和阴影，操作提示悬浮在导航上方12px；当前页以背景选中态表示，按用户要求移除绿条，正文随导航及提示实际高度预留焦点空间。

## Delivered

- 普通节点统一校验连接、布局、可用性、祖先可见性和最上层 Scope；失效、停用和卸载清理旧高亮。空域继续分发返回和页面 / 分类切换，节点到达后建立焦点。首次 A 在失效状态下恢复位置，后续 A 执行确认。
- 页面保存和恢复使用引擎的当前焦点及稳定 ID 记忆，Home / XCloud / Setting / Controller 与 library 页面接入同一恢复事务；点击一级导航保留按钮来源，肩键切换恢复目标页面，详情返回恢复来源卡片。游戏详情记忆补齐好友及安装项目身份，注销清理页面记忆。
- Scope 使用实例 token 和激活代际，KeepAlive 停用释放，迟到 nextTick / rAF 校验当前身份。下层重复注册保留上层归属，同 ID 新旧实例可安全交接，嵌套弹层关闭保留来源链。
- 临时禁用控件优先转移至当前页邻近候选，重新可用后恢复来源；用户继续操作取消等待。全 busy 弹层使用域内承接节点，数据恢复后交还控件。
- Tab / Shift+Tab 循环收在共享引擎，支持原生输入后代；音量面板消费最上层 B / Escape。串流持键释放 sink 显式标记，延迟恢复检查页面及当前弹层。录入会话恢复同样走引擎校验。

## Changes

- `src/navigation/core/`：有效性、恢复事务、Scope 注册、唯一高亮、原生 Tab 和生命周期回归。
- `src/pages/`、`src/pages/library/`：页面来源和分类恢复接入；数据身份、延迟请求与读焦点入口统一。
- `src/components/`：弹层保留自己的关闭 / busy 规则，共用引擎 Tab；轨道显露读取当前引擎焦点，并取消停用后的延迟观察请求。
- TopNavBar / AppShell / ShellActionHints：窄屏70px通栏导航贴底，16px上沿圆角、90%半透明背景、24px模糊与描边阴影，五入口单排，底部安全区随系统inset扩展。正文延伸至导航后方；提示在上方12px独立悬浮，ResizeObserver将提示实际高度同步到正文padding及scroll-padding，避让包含导航和提示高度并保留额外24px焦点留白；宽屏沿用左侧导航。
- 提示高度变化后，AppShell在DOM更新完成时调用共享引擎的`revealCurrentFocus`即时滚动显露当前控件，保持临时禁用来源记忆；弹层持有焦点时按上层Scope边界处理。
- Tauri + Vue 3 + TypeScript + Rust、InputController 和方向几何寻路主线继续沿用；本次交付为前端改动。

## Validation

| 检查 | 结果 |
| --- | --- |
| 焦点修复阶段：导航、Scope、页面、library、录入会话定向回归 | 7 文件 / 62 项通过 |
| 焦点修复阶段：全量前端回归 | 67 文件 / 666 项通过 |
| 毛玻璃及提示显露调整：全量前端回归 | 67 文件 / 668 项通过 |
| `pnpm lint:fix` | 通过 |
| `pnpm typecheck` | 通过 |
| `pnpm build` | 通过，346 modules transformed |
| `git diff --check` | 通过 |

正式测试位于 `engine-focus.test.ts`、`scope-lifecycle.test.ts`、`usePageFocus.test.ts` 及现有设置 / library 回归入口。审计目录中的原始测试和 `results.json` 保留审计基线证据，修复后的验证以默认 `src/**/*.test.ts` 入口为准。

实际预览加载生产 Vue 组件、几何导航及公共键盘输入桥，RPC / 认证 / 保存使用隔离的内存夹具。核对结果：

| 操作 | 实际恢复位置 |
| --- | --- |
| Home → 安装库 → Home | `xhome.installed-games`，单一标记 |
| 设置分类 → 控制器 → 肩键返回设置 | `setting.tabs.streamingExperience`，单一标记 |
| 设置主题保存完成 | 原选项 `…theme.option.1`，单一标记 |
| 好友详情返回 | `my.friend.2533274799`，单一标记 |
| 个人游戏详情返回 | `my.achievements.game.1000`，单一标记 |
| 相册截图关闭 | `album.screenshot.screenshot-0`，单一标记 |
| 云游戏两次 LT/RT | recent 第一项 → new 第一项，均为单一标记 |
| 音量 B/K 与 Esc | 面板关闭；Tab 可进入原生数值输入 |

贴底导航与毛玻璃调整重新执行lint / typecheck / build / diff check及全量668项回归，并以实际iframe视口核对360×640、700×600的深浅色及960×540的侧栏布局。360px时导航为360×70px，五入口约64×52px；700px时导航为700×70px，按钮最大宽度120px。导航底边与视口底边一致，提示与导航固定相隔12px，背景不透明度90%，材质为blur(24px) saturate(140%)，圆角为16px 16px 0 0，一级当前页的`::after`为`none`。

360px设置页的提示换行为96px高，正文padding与scroll-padding同步为190px（70px导航 + 96px提示 + 24px留白），内部内容底边450px、提示顶边462px。初始主机提示40px高，末项切换后提示变高时曾遮住焦点12px，补齐布局显露后焦点底边478px、提示顶边492px，相距14px且保持单一焦点标记；新增回归同时验证避让变化的即时滚动及临时禁用来源恢复。960px时提示恢复原有透明页尾，一级导航位于左侧；前一阶段另已核对1280×720布局。

截图证据：

- [360px 深色](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-06-pc-navigation-focus-audit/assets/fixed-bottom-glass-nav-360-dark.png) / [360px 浅色](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-06-pc-navigation-focus-audit/assets/fixed-bottom-glass-nav-360-light.png)
- [700px 深色](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-06-pc-navigation-focus-audit/assets/fixed-bottom-glass-nav-700-dark.png) / [700px 浅色](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-06-pc-navigation-focus-audit/assets/fixed-bottom-glass-nav-700-light.png)
- [提示增高后的末项焦点](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-06-pc-navigation-focus-audit/assets/fixed-bottom-glass-nav-360-last-focus.png)
- [960px 浅色侧栏](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-06-pc-navigation-focus-audit/assets/fixed-bottom-nav-wide-960-light.png)
- [960px](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-06-pc-navigation-focus-audit/assets/fixed-nav-960-dark.png) / [1280px](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-06-pc-navigation-focus-audit/assets/fixed-nav-1280-dark.png)
- [保存后保持单一焦点](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-06-pc-navigation-focus-audit/assets/fixed-settings-save-single-focus.png)

## Risks and Follow-up

按仓库约定，物理手柄、系统窗口失焦 / 恢复、原生 select 的手柄录入及真实串流 chrome / 菜单 / 音量 / 文本 / 诊断切换由用户实机验收。优先验收长按 A 关闭弹层后的承接，以及连续肩键切页和输入录入取消后的恢复。
