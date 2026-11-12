# 高清界面素材验证

> 2026-11-12 整理：本页保留验证摘要；带 Git 链接的截图、夹具和中间输出按采样时提交读取。

任务 `2026-11-12-hd-ui-assets`，2026-11-12完成手柄、主机及界面图标的高清素材更新。同日按用户要求恢复提交 `e33f554a` 的整套应用图标，撤销应用图标重绘、生成脚本和新增favicon。

应用图标当前状态以[回退核验](checks/app-icon-restore.json)及[资源检查](asset-results.json)为准。本页截图、`icon-generation.json`及原构建、页面回放输出保留为回退前的历史记录。

[历史截图：最终素材与页面对照](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-12-hd-ui-assets/final-assets-overview.png)

## 素材与来源

| 素材 | 完成结果 | 来源及记录 |
| --- | --- | --- |
| Xbox 手柄 | 微软官方正面透明图1488×1032；兼容JPEG同步为1488×1488 | [手柄来源](../../../src/assets/ctrl/README.md)、[前序热点及页面验证](../2026-11-12-xbox-controller-art-quality/README.md) |
| Series X / S | 微软官方2000×1500产品照，整理为1536×1536透明PNG；兼容原图和存量主机资源同步 | [主机来源](../../../src/assets/console/README.md)、[生成记录](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-12-hd-ui-assets/console-generation.json) |
| 界面图标 | 9个SVG统一尺寸与比例、清理第三方class、采用主题图标颜色规则 | [资源说明](../../../src/assets/README.md)、[深浅色对照](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-12-hd-ui-assets/icons-dark.png) |
| 应用图标 | 恢复用户原版源文件、50张平台PNG及ICNS、ICO，53个文件与提交 `e33f554a` 内容一致 | [回退核验](checks/app-icon-restore.json) |
| 其他素材 | 背景原图3840×1402；存量iOS各色应用图标1024px RGB；启动和连接图标为SVG | [全量清单](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-12-hd-ui-assets/assets-after.json) |

主机处理以alpha ≥248的主体向外扩展两像素，保留原始抗锯齿，清理广告图地面投影。主体保留原生像素，最长边上限1320px，透明画布承担布局留白，页面继续绘制接触投影。Series X主体1017×1205，Series S主体845×1214。

应用图标回退后的实际资源数量、格式与修改清单见[资源检查](asset-results.json)及[全量清单](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-12-hd-ui-assets/assets-after.json)。透明Series S存量PNG继续承接旧Series S JPEG引用；前端构建使用的手柄与两款主机PNG合计约3.54MB。

游戏封面等远程图片沿用`RemoteImage`按CSS尺寸及最高DPR 3取图的逻辑；相关源选择测试已通过。

## 验证结果

| 检查 | 结果 | 证据 |
| --- | --- | --- |
| 实际资源格式 | 回退后80个资源、164项通过；53个原版图标哈希、50张平台PNG尺寸与模式、ICNS各帧、ICO原版六个尺寸及资源引用有效 | [检查结果](asset-results.json)、[检查脚本](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-12-hd-ui-assets/check-assets.py) |
| 页面加载与显示密度 | 10组页面、52项通过；图片加载完整，位图像素满足实际DPR；18个SVG尺寸视图；控制台错误0、横向溢出0 | [页面数据](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-12-hd-ui-assets/browser-results.json)、[密度校验](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-12-hd-ui-assets/browser-checks.json) |
| 相关测试 | 6文件、50项通过，覆盖震动统一设置、快捷键编辑、输入路由及高清远程图片源 | [Vitest输出](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-12-hd-ui-assets/checks/vitest.txt) |
| lint | 高清素材任务的原lint通过；回退复核遇到既有`SessionService.test.ts`的`style/max-statements-per-line`错误 | [历史输出](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-12-hd-ui-assets/checks/lint.txt)、[回退核验](checks/app-icon-restore.json) |
| 类型检查 | `pnpm typecheck`通过 | [输出](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-12-hd-ui-assets/checks/typecheck.txt) |
| 构建 | 原版图标回退后`pnpm build`通过 | [回退构建输出](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-12-hd-ui-assets/checks/app-icon-restore-build.txt) |
| 导航覆盖 | 217处交互点全部支持，违规0 | [输出](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-12-hd-ui-assets/checks/navigation.txt) |
| 工具语法 | 2个JavaScript及3个Python脚本通过 | [汇总](checks/results.json) |

## 界面回放

| 场景 | 窗口 / DPR | 截图 |
| --- | --- | --- |
| 默认深色，两款主机切换 | 646×875 / 2 | [Series X](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-12-hd-ui-assets/home-series-x-dark.png)、[Series S](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-12-hd-ui-assets/home-series-s-dark.png) |
| 默认浅色，两款主机切换 | 646×875 / 2 | [Series X](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-12-hd-ui-assets/home-series-x-light.png)、[Series S](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-12-hd-ui-assets/home-series-s-light.png) |
| 桌面宽窗浅色 | 1280×800 / 1 | [Series X](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-12-hd-ui-assets/home-series-x-wide-light.png)、[Series S](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-12-hd-ui-assets/home-series-s-wide-light.png) |
| 英文浅色窄窗 | 390×844 / 1 | [Series X](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-12-hd-ui-assets/home-series-x-390-light-en.png)、[Series S](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-12-hd-ui-assets/home-series-s-390-light-en.png) |
| 应用与界面图标 | 646×875 / 2，完整页面截图 | [深色](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-12-hd-ui-assets/icons-dark.png)、[浅色](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-12-hd-ui-assets/icons-light.png) |

当前图标预览按32/64/128px分别加载原版64/128/256px PNG，原版应用源图及界面图标使用SVG。浏览器临时窗口覆盖已恢复。

验证入口`index.html`复用现有RPC和手柄夹具，并在独立包装中补齐当前主机缓存合同；页面、路由及几何导航由`src/main.ts`装配。`icons.html`用于平台PNG和SVG在浏览器中的清晰度检查。

## 复核

```sh
python3 dev-docs/validation/2026-11-12-hd-ui-assets/check-assets.py
pnpm lint:fix
pnpm typecheck
pnpm build
pnpm test:navigation-coverage
```

后续桌面构建使用恢复后的原版应用图标；实机启动、真实手柄和串流验收沿用仓库的用户手动流程。
