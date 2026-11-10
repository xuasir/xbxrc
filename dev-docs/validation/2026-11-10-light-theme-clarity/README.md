# 浅色透明度修复与表面提亮

日期：2026-11-10。任务：`2026-11-10-light-theme-clarity`。

## 原因与调整

`AppShellLayout.vue` 的 scoped 规则 `:global([data-theme='light']) .app-shell::after` 编译为 `[data-theme='light'] { filter: opacity(.65) }`，使 html 整页进入 65% 不透明度合成。浏览器原始计算样式确认 html 的 filter 为 `opacity(0.65)`，背景伪元素的 filter 为 `none`；文字、图标、封面与焦点一起被淡化。

将背景强度改为主题 token，并由 `.app-shell::after` 单独消费。浅色 html / body 的 filter 恢复 `none`，背景漂移层为 `opacity(0.35)`。浅色底色由 `#eef1f5` 提亮到 `#f8f9fb`，普通玻璃表面改为 96% / 92% 白色，浮层改为 98% / 96% 白色，普通 / 加强面板为 92% / 98% 白色，设置行采用 92% 白色。静态环境光由 60% 降到 8%，主机展示卡的绿青局部染色同步降低。

## 实际前端验证

通过 `pnpm dev --port 1430 --host 127.0.0.1` 加载已有[完整离线前端入口](../2026-11-09-gamepad-focus-action-completion/project.html)，实际运行生产 Vue App、路由、样式、焦点和主题切换，Rust RPC 使用原有内存夹具。

- 1280×720 / 390×844 深浅色设置页：根元素 filter 为 `none`，无横向溢出。
- 浅色主页与账户弹层：白色卡片和清晰文本，账户面板保留 `blur(24px) saturate(1.25)`。
- 深色六项材质 token、静态环境光渐变和漂移层 filter 与任务前逐项一致。
- 9 项浏览器检查全部通过，浏览器 error 日志 0 条。原始结果见 [browser-results.json](browser-results.json)。
- `pnpm lint:fix`、`pnpm typecheck`、`pnpm build` 和 `git diff --check` 通过。临时浏览器标签与 1430 服务已关闭。

调整前：

![浅色设置页调整前](settings-before.jpg)

调整后：

![浅色设置页调整后](settings-after.jpg)

其他证据：[浅色主页](home-light-wide.jpg)、[浅色账户弹层](menu-light-wide.jpg)、[浅色窄屏](settings-light-narrow.jpg)、[深色宽屏](settings-dark-wide.jpg)、[深色窄屏](settings-dark-narrow.jpg)。
