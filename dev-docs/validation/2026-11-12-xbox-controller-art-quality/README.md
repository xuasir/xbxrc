# Xbox 手柄高清图片验证

日期：2026-11-12。共用手柄图替换为微软商店 Carbon Black 官方透明正面图，输出 1488×1032 RGBA；原图为 372×258，透明边界只有全透明和全不透明两个取值。

## 图片与定位

- [来源、裁切参数和哈希](asset-results.json)；来源说明位于 [素材目录](../../../src/assets/ctrl/README.md)。
- [深浅背景、两倍展示对照](asset-dark-light-comparison.png)验证外轮廓和握把边缘。
- [17 个按键校准图](asset-hotspot-calibration.png)确认标记均位于不透明主体上；左右扳机、Xbox、摇杆和方向键按新图校准。
- 画布继续使用 372×258 逻辑坐标。键盘摇杆方向从相同按键坐标派生，图像、热点和引线同步缩放。

## 验证结果

| 检查 | 结果 | 证据 |
| --- | --- | --- |
| 相关 Vitest | 5 文件、45 项通过 | [结果](vitest.txt) |
| pnpm lint:fix | 全仓通过 | [输出](lint.txt) |
| pnpm typecheck | 通过 | [输出](typecheck.txt) |
| pnpm build | 通过，包含高清资源 | [输出](build.txt) |
| 导航覆盖 | 78 个 Vue 文件、217 处交互、0 违规 | [输出](navigation.txt) |

[页面测量](browser-results.json)覆盖映射、摇杆、实时输入、震动及 390×844 / 320×844 浅色英文窗口。图片热点与交互按钮的最大中心偏差为 0.013px，横向溢出和控制台错误均为 0。连接页读取到 1488×1032 原图，显示宽度 180px。震动页两排居中布局继续通过。

- [映射页](mapping-final-dark.png)
- [摇杆页](sticks-final-dark.png)
- [实时输入页](live-final-dark.png)
- [连接页](connection-final-dark.png)
- [震动页](vibration-final-dark.png)
- [390px 浅色英文](vibration-390-light-en.png)
- [320px 浅色英文](vibration-320-light-en.png)

验证继续复用独立震动验证入口与既有离线夹具。原夹具保持原样。
