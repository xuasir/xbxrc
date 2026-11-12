# 主机示意图片

首页使用 `series-x-transparent.png` 与 `series-s-transparent.png`，均为 1536×1536 RGBA PNG。原图为微软官方 2000×1500 透明产品照，获取日期 2026-11-12，图片归 Microsoft / Xbox 所有。

| 型号 | 商店页面 | 官方透明原图 |
| --- | --- | --- |
| Series X — Carbon Black | [微软商店](https://www.microsoft.com/en-us/d/xbox-series-x/8wj714n3rbtl) | [2000px PNG](https://cdn-dynmedia-1.microsoft.com/is/image/microsoftcorp/b00-Xbox-Series-X-Carbon-Black-Console-Angled?fmt=png-alpha&wid=2000) |
| Series S — Robot White | [微软商店](https://www.microsoft.com/en-us/d/xbox-series-s/942j774tp9jn) | [2000px PNG](https://cdn-dynmedia-1.microsoft.com/is/image/microsoftcorp/b00-Xbox-Series-S-White-Console-Angled?fmt=png-alpha&wid=2000) |

处理脚本以 alpha ≥248 的主体向外扩展两像素，保留原始透明抗锯齿，移除广告图的地面长投影。主体保留原生像素，最长边上限1320px，居中置于透明画布；接触投影和环境光继续由界面绘制。

```sh
python3 scripts/prepare-console-images.py \
  --source-x /tmp/xbxrc-series-x-master.png \
  --source-s /tmp/xbxrc-series-s-master.png \
  --sync-ios-assets
```

`series-x.png` 同步透明高清图，`series-s.jpeg` 同步白底高清兼容图；`iosapp` 存量主机资源同步透明 PNG。首页按 `consoleType` 选择对应图片。

来源哈希、透明通道、深浅背景和首页回放见[高清素材验证记录](../../../dev-docs/validation/2026-11-12-hd-ui-assets/README.md)。
