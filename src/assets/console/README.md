# 主机示意图片

首页使用 `series-x-transparent.png` 与 `series-s-transparent.png`，均为 512×512 透明 PNG，主体最大尺寸 440px，居中放置。接触投影和环境光由界面绘制，图片适配深浅背景。

来源为仓库已有的 `series-x.png` 与 `series-s.jpeg`。保留原文件便于追溯；Series X 裁去透明留白，Series S 从四边移除连通的深灰棋盘背景，再裁切和缩放。Series S 处理阈值为 RGB 最大值 ≤65；内部黑色散热孔保留，由白色机身隔开。

选择规则继续消费 `consoleType` 的 `Series X` / `Series S`。其他型号沿用现有回退逻辑。
