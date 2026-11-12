# 全应用圆角统一增加 2px

> 2026-11-12 整理：本页保留验证摘要；带 Git 链接的截图、夹具和中间输出按采样时提交读取。

日期：2026-11-09。任务：`2026-11-09-ui-radius-increase`。

## 调整范围

| 圆角层级 | 调整前 | 调整后 |
| --- | --- | --- |
| 小号 | 2px | 4px |
| 控件 | 4px | 6px |
| 卡片 | 6px | 8px |
| 面板与弹层 | 8px | 10px |
| 窄屏底部导航上沿 | 16px | 18px |

基础 SCSS 与兼容 CSS 令牌同步增加 2px。底部导航接入共享面板圆角，键帽和分类标记分别消费控件、小号圆角；焦点环继续按控件轮廓外扩 4px。头像、圆点、开关与扳机进度条沿用圆形或胶囊形状，拼接边保留直角。

## 本地前端验证

复用 [完整应用离线数据入口](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-09-gamepad-focus-action-completion/project.html)，从本地 Vite 服务加载生产 `/src/main.ts`、App、路由及导航装配。

- 设置页面：390×844、1280×720，覆盖深色与浅色；基础圆角均为 4 / 6 / 8 / 10px。
- 导航、设置选项、按钮与键帽：6px；控件焦点环 10px，主机卡片焦点环 12px。
- 主机与账户页面：截图实际尺寸 646×875；主机卡片 8px、展示面板与账户弹层 10px。
- [9 项浏览器检查](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-09-ui-radius-increase/browser-results.json)全部通过；`pnpm lint:fix`、`pnpm typecheck`、`pnpm build`、`git diff --check` 通过。

[窄屏本地预览](http://127.0.0.1:1420/dev-docs/validation/2026-11-09-gamepad-focus-action-completion/project-narrow.html) · [源码及检查摘要](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-09-ui-radius-increase/validation-summary.json)

## 截图

[历史截图：主机卡片与账户弹层](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-09-ui-radius-increase/home-light-profile-panel.png)

[深色宽屏](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-09-ui-radius-increase/settings-dark-wide.png) · [浅色宽屏](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-09-ui-radius-increase/settings-light-wide.png) · [浅色账户弹层](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-09-ui-radius-increase/settings-light-profile-panel.png) · [浅色窄屏](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-09-ui-radius-increase/settings-light-narrow.png) · [深色窄屏](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-09-ui-radius-increase/settings-dark-narrow.png) · [主机页面](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-09-ui-radius-increase/home-light-medium.png)
