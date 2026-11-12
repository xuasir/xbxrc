# 快捷键提示精简与震动统一设置验证

日期：2026-11-12。快捷键子页长提示已移除；震动设置的总开关和统一强度位于手柄图正下方，固定两排居中。

## 行为验证

- 统一强度提供轻、标准、强三个档位，一次保存四路电机强度及兼容字段；游戏震动开关独立保存。
- 单路仍可在图上微调。四路不同档位时，统一选项均取消选中；再次统一后同步当前通道浮层。
- 保存失败保留当前配置；重试和保存中的重复确认由真实组件测试覆盖。
- 快捷键子页保留左上串流菜单图标、左下诊断图标、绑定信息和独立清空操作。

## 检查

| 检查 | 结果 | 证据 |
| --- | --- | --- |
| Vitest | 5 文件、45 项通过 | [输出](checks/vitest.txt) |
| pnpm typecheck | 通过 | [输出](checks/typecheck.txt) |
| pnpm lint:fix | 全仓通过 | [输出](checks/lint.txt) |
| pnpm build | 通过，包含最终两排布局 | [输出](checks/build.txt) |
| 导航覆盖 | 78 个 Vue 文件、217 处交互、0 违规 | [输出](checks/navigation.txt) |
| git diff --check | 通过 | [输出](checks/diff.txt) |

## 页面回放

[观察结果](browser-results.json)保存统一四路、左扳机微调、开关独立和键盘确认等结果，以及 646×875、800×600、390×844、320×844 四组布局测量。四组均保持图下两排居中，横向溢出为 0；320px 英文页面的三个档位完整显示。页面控制台错误为 0。

- [中文深色震动页](vibration-final-dark-zh.png)
- [英文浅色 390px 震动页](vibration-narrow-light-en.png)
- [英文浅色 320px 震动页](vibration-320-light-en.png)
- [精简后的快捷键页](shortcuts-final-dark-zh.png)

独立验证入口复用既有项目与手柄夹具，仅为当前入口增加四路 actuator；[原夹具](../2026-11-11-gamepad-locked-input-and-extra-buttons/)保持原样。真实手柄震感和串流由用户按仓库约定手动验收。
