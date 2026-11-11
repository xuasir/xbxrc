# 手柄图示子页与锁定输入验证

日期：2026-11-11。入口装配真实 Vue App、路由、公共输入控制器和 Browser GamepadBackend；RPC 与 21 按钮/5 轴手柄使用离线夹具。

## 自动检查

```sh
pnpm lint:fix
pnpm typecheck
pnpm build
pnpm test:navigation-coverage
./node_modules/.bin/vitest run src/shared/gamepad/browser-backend.test.ts src/shared/gamepad/input-controller.test.ts src/shared/gamepad/settings-model.test.ts src/shared/gamepad/keyboard-fallback-bridge.test.ts src/shared/gamepad/runtime.test.ts src/pages/settings/useGamepadPanelSession.test.ts src/navigation/core/engine-focus.test.ts src/navigation/core/engine-settings.test.ts src/navigation/core/scope-lifecycle.test.ts src/components/settings/navigation-controls.test.ts src/app/page-motion.test.ts
cargo test -p xbxrc config_policy --lib
cargo fmt --all -- --check
git diff --check
```

Vitest 11 文件/144 项、Rust 配置 11 项通过。导航覆盖 190 处交互，0 违规；lint、typecheck、build、fmt 和 diff 检查通过。

## 浏览器回放

启动 `pnpm dev --host 127.0.0.1 --port 8948`，打开：

`http://127.0.0.1:8948/dev-docs/validation/2026-11-11-gamepad-locked-input-and-extra-buttons/index.html#/controller`

点击“运行回归”。完整回放包含 21 秒持续观察，最终保留布局页。运行前重载入口可恢复离线配置与中立设备态。“重连回归”单独验证无设备提示、重连激活和再次退出；实际手柄激活规则保留，重连先采样中立态，再按一次按钮。

| 尺寸 | 结果 | 文件 |
| --- | --- | --- |
| 1280×720 | 65 项通过 | [browser-results.json](browser-results.json) |
| 390×844 | 65 项通过 | [browser-results-narrow.json](browser-results-narrow.json) |
| 800×600 | 65 项通过 | [browser-results-800.json](browser-results-800.json) |

检查包含 Xbox 与 A 双向交换/保存重载、Xbox 录入/焦点、背键到 Xbox、Xbox 键盘绑定/保存，以及 17 个原按键/映射标注（含 Xbox）、页内目标选择、B 录入与持续锁定、普通 Esc、全部普通按键、短/长组合、菜单抑制、背键保存/恢复、21 秒观察、断开、非标准原始输入、键盘出口、草稿放弃确认、摇杆死区、键盘保存、握把能力、输入测试与来源焦点。

[layout-results.json](layout-results.json) 记录桌面/窄窗口、中英文/深浅色，以及摇杆、键盘、震动辅助图的几何检查：标注无重叠、文字无省略、导航节点全部可达。桌面热点与连接线误差 <0.01px；窄布局使用两列标注、Xbox 独占一行并隐藏连接线。本次更新三种窗口的映射与键盘测量；摇杆/震动测量沿用前阶段证据。

## 最新子页面截图

- [Xbox 与 A 双向映射](mapping-xbox-to-A.jpg)
- [桌面深色映射](mapping-desktop-dark.jpg)
- [800×600 浅色映射](mapping-800-light.jpg)
- [窄窗口英文映射](mapping-narrow-english-dark.jpg)
- [B 录入后持续锁定](locked-capture-B.jpg)
- [实时输入 B 与背键](locked-live-B.jpg)
- [摇杆辅助图](sticks-preview.jpg)
- [键盘辅助图](keyboard-preview.jpg)
- [左右握把辅助图](vibration-preview.jpg)

截图取自子页面实现；普通编辑页显示页内草稿操作，录入/观察显示专用出口提示。

## 底部分隔线修正

映射目标和背键分组使用留白，固定操作栏保留一条分隔线。[separator-results.json](separator-results.json) 记录 1280×720、390×844 的样式测量：两处内容分组上边框为 0px，操作栏上边框为 1px，内容滚动正常；Xbox 映射截图已更新。`pnpm lint:fix` 和 `git diff --check` 通过。

## 实机验收

真实 Xbox/B 输入、Menu+View 长按出口、独立背键识别/重连/串流映射与设备震动由用户手动验收。独立来源以固件、驱动和 Browser API 输出为准。
