# Xbox 手柄图

`series-ctrl-transparent.png` 用于控制器连接、映射、实时输入、摇杆和震动页面。

- 来源：[微软商店 Xbox Wireless Controller — Carbon Black](https://www.microsoft.com/en-us/d/xbox-wireless-controller/8xn59crbsqgz)。图片归 Microsoft / Xbox 所有。
- [官方透明正面原图](https://cdn-dynmedia-1.microsoft.com/is/image/microsoftcorp/gldn-Xbox-Cntrl-Black-b00?fmt=png-alpha&wid=2000)，2000×2000 RGBA，获取日期 2026-11-12。
- 原图按 `(219, 455, 1781, 1539)` 裁去外围空白，使用 Pillow LANCZOS 和预乘透明通道缩放为 1488×1032，保存无损 PNG。
- `GamepadMappingDiagram.vue` 沿用 372×258 逻辑坐标；热点和引线按新图的实体按钮位置校准，键盘摇杆方向复用相同摇杆坐标。
- `series-ctrl.jpeg` 同步为1488×1488白底高清兼容图，使用同一透明图片居中合成并以质量95保存。

来源哈希、透明通道统计与深浅背景对照见[图片验证记录](../../../dev-docs/validation/2026-11-12-xbox-controller-art-quality/README.md)。
