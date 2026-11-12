# 手柄映射、自动输入测试与四路震动验证

日期：2026-11-11。入口装配真实 Vue App、路由、公共 InputController 与 Browser GamepadBackend；RPC 与 21 按钮/5 轴设备采用现有离线夹具。夹具文件保持现状。

## 当前自动检查

```sh
pnpm lint:fix
pnpm typecheck
pnpm build
pnpm test:navigation-coverage
./node_modules/.bin/vitest run src/shared/gamepad src/pages/settings/useGamepadPanelSession.test.ts src/navigation src/components/settings/navigation-controls.test.ts src/app/page-motion.test.ts
cargo test -p xbxrc config_policy --lib
cargo fmt --all -- --check
git diff --check
```

相关 Vitest 20 文件 / 197 项、Rust 配置 14 项通过。导航覆盖 75 个 Vue 文件 / 209 处交互 / 0 违规；lint、typecheck、build、fmt 和 diff 检查通过。

## 最终浏览器回放

启动 `pnpm dev --host 127.0.0.1 --port 8948`，打开：

`http://127.0.0.1:8948/dev-docs/validation/2026-11-11-gamepad-locked-input-and-extra-buttons/index.html#/controller`

本轮通过 CUA 操作当前页面，并使用夹具现有的“Xbox / B / 背键1 / 组合 / 释放”按钮模拟输入；键盘使用实际键盘事件。结果见 [final-interaction-results.json](final-interaction-results.json)：21 项全部通过。

| 范围 | 结果 |
| --- | --- |
| 图上录入 | 17 个标准目标；B、组合和背键录入，全部释放完成与焦点恢复，保存重开通过 |
| 来源识别 | 手柄来源显示 17 个按钮，键盘来源增加 8 个摇杆方向；标题小标自动识别通过 |
| 输入测试 | 关闭键盘模拟仍可测试，B 保持锁定并显示保存输出，扩展键首次输入后动态展示与释放，触屏/鼠标返回通过 |
| 摇杆 | 百分比滑杆与左右独立反转通过；桌面、800×600、390×844 浮层边界及 8px 锚点间距通过 |
| 震动 | 四路图示、标题测试入口、390×844 浮层边界通过；硬件参数与双路降级由单元测试覆盖 |
| 通用 | 快捷键分组位于应用更新上方；键盘绑定录入、释放保存、焦点恢复通过 |
| 稳定性 | 录入前、中、后画布矩形完全一致，映射在 800×600 与 390×844 无横向溢出 |

几何测量：图示水平中心误差约 0.008px，纵向中心偏差 4px；左右标注到引线空白均为 14.603px。

## 当前截图

- [通用界面震动与串流菜单快捷键](general-shortcuts-final.png)
- [标准目标、组合及背键输入映射](mapping-final-desktop.png)
- [键盘映射窄窗口](mapping-final-narrow.png)
- [百分比滑杆与独立 Y 反转](sticks-final-desktop.png)
- [窄窗口摇杆浮层](sticks-final-narrow.png)
- [四路震动与高对比浮层](vibration-final-desktop.png)
- [窄窗口震动浮层](vibration-final-narrow.png)
- [实时 Xbox 输出与动态扩展输入](test-final-desktop.png)

截图中的 Xbox validation controller 来自离线夹具的模拟设备，实际应用展示 Browser Gamepad API 返回的接入设备。

## 历史证据

[browser-results.json](browser-results.json)、[browser-results-narrow.json](browser-results-narrow.json)、[browser-results-800.json](browser-results-800.json)、[layout-results.json](layout-results.json)、[separator-results.json](separator-results.json) 和早期 JPG 截图保留对应阶段证据。旧“运行回归”脚本包含持续录入、来源选择、页内开关、方向入口与旧调节控件断言；当前交付使用上方最终人工回放和最新自动检查结果。

## 实机验收

实际键盘/手柄的单键和组合映射、Menu+View 安全出口、独立背键识别与重连、左右 Y 反转、四路马达效果、界面触觉，以及串流输出与两种菜单触发由用户手动验收。独立输入和触觉能力以固件、驱动与 Browser Gamepad API 输出为准。
