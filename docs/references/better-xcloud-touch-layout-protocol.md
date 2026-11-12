# better-xcloud 专用触控布局：布局协议

better-xcloud 用游戏索引选择社区 JSON 模板，再通过 Xbox 网页内部的 `changeLayoutForScope()` 将控件交给 TAK。协议可以按三层理解：社区分发包装、微软 Layout/Manifest/Context、串流布局命令。控件的 `action`、`axis`、`layer` 同时描述操作语义，位置和外观由布局字段描述。

核对日期：2026-11-12。better-xcloud 代码版本 `76db611e610c45ea996f6d35a9df5f980e8f0f85`，社区布局版本 `d66bd2925ee4518a955955b0b52890a407575285`。微软实现依据冻结的公开发布 JavaScript；[来源与 SHA-256](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-12-tak-layout-and-engine/sources.json)、[回放说明](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-12-tak-layout-and-engine/README.md)。

引擎执行过程见[TAK/Babylon 触控引擎实现](/Users/guo.xu/Documents/code/games/xbxrc/docs/references/tak-babylon-touch-engine.md)。

## 1. 游戏身份与布局身份

| 字段或 ID | 形式 | 消费位置 |
| --- | --- | --- |
| `/titleinfo` 的 `titleid` | 十六进制字符串 | 当前前台游戏识别 |
| 社区索引文件名 | 十进制 Xbox TitleId，例如 `2030093255.json` | 查询游戏的模板与默认布局 |
| `product_id` | 商店 ProductId，例如 `9NKX70BBCDRN` | 社区支持列表、商品展示 |
| `layouts[]` 的模板名 | 例如 `forza` | 加载可供多个游戏复用的模板文件 |
| `default_layout` / 模板中的布局 key | 例如 `layout-1`、`rightstick` | 用户选择与默认选项 |
| 官方已加载布局的 `id` | TAKX 中布局 JSON 的文件名去掉扩展名 | `showLayoutId` 查询 |
| `scope` | 当前游戏 TitleId，或系统/用户作用域 | 保存基础布局和用户覆盖 |

better-xcloud 使用 `parseInt(titleid, 16).toString()` 得到社区查询键。`focused` 决定游戏处于前台时应用社区布局，后台菜单时显示通用控制器。

来源：[游戏消息监听](/Users/guo.xu/Documents/code/games/better-xcloud/src/modules/touch-controller.ts:330)、[Forza Horizon 5 索引](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-12-tak-layout-and-engine/examples/forza-title.json)。

## 2. 社区分发：游戏索引与模板

Forza Horizon 5 的游戏索引原文：

```json
{
  "name": "Forza Horizon 5",
  "product_id": "9NKX70BBCDRN",
  "default_layout": "layout-1",
  "layouts": ["forza"]
}
```

加载路径为：

```text
touch-layouts/2030093255.json
  → touch-layouts/layouts/forza.json
  → layouts["layout-1"]
```

模板采用下列包装结构，示例控件节选为左摇杆：

```json
{
  "schema_version": 1,
  "layouts": {
    "layout-1": {
      "name": "Layout 1",
      "author": "redphx",
      "content": {
        "left": {
          "inner": [
            {
              "type": "joystick",
              "axis": {
                "input": "axisXY",
                "output": "leftJoystick",
                "deadzone": {"threshold": 0.1, "radial": true}
              }
            }
          ]
        }
      }
    }
  }
}
```

`schema_version: 1` 属于社区分发包装。`name` 和 `author` 供设置界面与提示使用；微软正式 Layout 文件接受 `content`、`styles`、`definitions` 和 `$schema`。

`GhPagesUtils.getUrl()` 优先使用本地保存的 gh-pages commit，缺省时使用分支 URL。`ids.json` 保存 ProductId 支持列表；`#customLayouts` 按 TitleId 缓存索引，`#baseCustomLayouts` 按模板名缓存展开结果。

当前加载实现用 `async forEach` 启动各模板请求，再用固定 1 秒延迟等待布局管理器。模板完成顺序和多个模板的同名布局覆盖顺序受异步请求影响；加载失败会以 `null` 缓存。理解该代码时，需要分别核对“管理器已就绪”和“所有模板请求已完成”。

来源：[分发 URL](/Users/guo.xu/Documents/code/games/better-xcloud/src/utils/gh-pages.ts:27)、[索引与模板加载](/Users/guo.xu/Documents/code/games/better-xcloud/src/modules/touch-controller.ts:132)、[完整 Forza 模板](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-12-tak-layout-and-engine/examples/forza-layouts.json)。

## 3. 社区布局怎样进入官方引擎

better-xcloud 对发布脚本匹配 `this._perScopeLayoutsStream=new`，将该对象保存为 `BX_EXPOSED.touchLayoutManager`，并发出 ready 事件。该对象对应官方 `LayoutCoordinator`。

实际应用参数为：

```javascript
window.BX_EXPOSED.touchLayoutManager.changeLayoutForScope({
  type: "showLayout",
  scope: "2030093255",
  subscope: "base",
  layout: {
    id: "System.Standard",
    displayName: "System",
    layoutFile: selectedCommunityLayout
  }
});
```

`selectedCommunityLayout` 是模板里的 `{name, author, content}` 对象。`showLayout` 接收已经可供运行时消费的对象，直接写入当前 scope；官方 bundle 加载路径则执行 schema 校验、引用展开、旧格式转换和资源解析。

这解释了社区文件的宽容格式。实际回放确认：`name/author` 会触发正式 Layout schema 的额外字段校验；Forza 模板的一个 `knob.stroke` 仅声明 `opacity`，依赖运行时默认值。抽取 `content` 并为该 stroke 补充 `type: "solid"` 后，当前冻结的 latest 校验器验证通过。

社区选择 key `layout-1` 和注入描述符 `id: "System.Standard"` 各自承担选择与运行时身份。官方内置标准布局在本快照中使用带固定后缀的 `System.Standard-D37853CB-9134-4D40-AE55-EDFBFF9A3368`。

来源：[暴露内部对象](/Users/guo.xu/Documents/code/games/better-xcloud/src/modules/patcher/patcher.ts:276)、[注入布局](/Users/guo.xu/Documents/code/games/better-xcloud/src/modules/touch-controller.ts:249)、[直接对象与 ID 路径](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-12-tak-layout-and-engine/source-excerpts/tak-core-protocol.txt:1053)。

## 4. 微软 Layout / Manifest / Context

| 文件 | 主要字段 | 作用 |
| --- | --- | --- |
| Layout | `$schema`、`content`、`styles`、`definitions` | 控件、位置、外观、复用值与引用 |
| Manifest | `$schema`、`layouts`、`assets`、`contextFile`、`defaultLayout`、`version`、`versionName` | 布局目录、资源目录、默认布局及 bundle 版本 |
| Context | `$schema`、`state`、`definitions`、`styles`、`allowedStateValues` | 动态游戏状态、跨布局复用定义与颜色 |

当前 latest 编译校验器要求 Layout 的 `content`，Manifest 的 `layouts` 与 `version`。Manifest 的 `version` 使用四段版本字符串；它描述 bundle 自身版本，`$schema` 描述文件格式版本。

官方分发单元 `.takx` 是 ZIP 内容，可以以 ArrayBuffer 或 Base64 交给 loader。典型目录为：

```text
takxmanifest.json
context.json
layouts/neutral/gameplay.json
layouts/en-US/gameplay.json
assets/neutral/@1.0x/fire.png
assets/neutral/@2.0x/fire.png
```

loader 先加载 `neutral`，再逐级加载语言和地区目录，让具体 locale 覆盖同名条目。图片按 DPI 目录选择，图标/资产随后转换为渲染引用。已加载布局 ID 来自布局文件名，布局 Map 使用小写 key 处理查询。

本快照向服务端声明最高 Layout `4.1.0.0`、Manifest `2.0.0.0`。内置校验器覆盖 Layout `1.2 / 2.0 / 3.5 / latest`、Manifest `1.1 / 2.0 / latest`、Context `3.5 / latest`；版本别名将多组历史 URL 映射到这些实现。旧版 converter 处理 `icon`、`threshold`、上边缘槽位及部分上下方向 action。

schema URL 在这里同时承担格式标识。校验函数随发布脚本内置，回放可离线运行；本轮访问三条 latest raw schema URL 得到 404，字段证据采用冻结的内置校验器和已保存的 Microsoft Learn 文档。

来源：[bundle 展开](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-12-tak-layout-and-engine/source-excerpts/tak-core-protocol.txt:434)、[版本与转换器](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-12-tak-layout-and-engine/source-excerpts/tak-core-protocol.txt:65)、[schema 检测](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-12-tak-layout-and-engine/source-excerpts/tak-core-protocol.txt:380)、[微软 Schema 文档](https://learn.microsoft.com/en-us/gaming/gdk/docs/reference/system/touchadaptationkit/touchadaptationkit-reference)。

## 5. 位置协议：区域、轮盘与槽位

| `content` 字段 | 内容 |
| --- | --- |
| `left / right / center` | 轮盘，含 `inner` 和 `outer` |
| `upper.right` | 上侧的一组控件 |
| `lower.leftCenter / lower.center / lower.rightCenter` | 下侧三个位置 |
| `layers` | 可动态激活的布局层 |
| `sensors` | 陀螺仪、加速度计等非视觉控件 |

位置主要通过语义槽位声明。宿主提供屏幕尺寸、DPI、方向、安全区和视频区域，TAK 根据槽位、玩家的轮盘位置/大小/旋转设置，生成具体 socket 坐标。

`inner` 可放至多 4 个控件；`outer` 有 8 个物理槽位，沿轮盘顺时针排列。`outer` 中的子数组构成 control cluster，占用当前及后续共两个物理槽位。基础布局中的 `null` 留出槽位，`blank` 是显式空控件。

来源：[Content 文档](https://learn.microsoft.com/en-us/gaming/gdk/docs/reference/system/touchadaptationkit/layout/game-streaming-touch-layout-content)、[Wheel 文档](https://learn.microsoft.com/en-us/gaming/gdk/docs/reference/system/touchadaptationkit/layout/game-streaming-touch-wheel)、[cluster 槽位遍历](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-12-tak-layout-and-engine/source-excerpts/tak-core-geometry.txt:401)。

## 6. 输入协议：action 与 axis

`action` 支持单个操作、组合操作，以及客户端布局动作：

```json
"action": "rightTrigger"
"action": ["gamepadA", "leftBumper"]
"action": {"type": "layer", "target": "AimTouchpad"}
```

常见值为 `gamepadA/B/X/Y`、`menu/view`、`leftBumper/rightBumper`、`leftThumb/rightThumb`、方向键、扳机及摇杆极向动作。普通 button 的 trigger action 对应扳机完全按下；`axis` 和 throttle 可产生连续模拟值。

| 映射 | 示例 | 含义 |
| --- | --- | --- |
| 双轴 | `axisXY → leftJoystick` | 二维位移映射到完整摇杆 |
| 单轴 | `axisX → rightJoystickX` | 一个维度映射到一个轴 |
| 极向 | `axisUp → rightTrigger` | 一个方向映射到正幅值 |
| 鼠标 | `axisXY → relativeMouse`，`axisY → relativeMouseY` | 相对鼠标位移或速度，具体由控件类型决定 |

`axis` 可以是对象或对象数组。每条映射可带 `deadzone`、`responseCurve`、`sensitivity`，引擎再叠加玩家灵敏度。映射分类要求输入和输出维度匹配。

本发布快照的 `deadzone` 和 `responseCurve` 有明确的源码行为差异，公式及回放值见[引擎的轴映射分析](/Users/guo.xu/Documents/code/games/xbxrc/docs/references/tak-babylon-touch-engine.md)。参数使用应以具体引擎版本和值域验证为依据。

来源：[Action 文档](https://learn.microsoft.com/en-us/gaming/gdk/docs/reference/system/touchadaptationkit/types/game-streaming-touch-action)、[Axis 文档](https://learn.microsoft.com/en-us/gaming/gdk/docs/reference/system/touchadaptationkit/types/game-streaming-touch-axis)、[映射分类与管线](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-12-tak-layout-and-engine/source-excerpts/tak-core-input.txt:1212)。

## 7. layer：临时操作面板

`content.layers` 按名字保存局部覆盖。按住带 layer action 的普通按钮会激活对应层，释放时关闭；`toggle: true` 使激活状态保留到下一次切换。

Forza 的 `dPadDisplay` 覆盖右侧内轮盘为方向键，并用外轮盘的多个 `blank` 控件覆盖基础按钮。左侧运动控件继续使用基础布局。

合并规则是协议的重要部分：

| layer 中的值 | 合并结果 |
| --- | --- |
| 区域或数组项缺省 | 沿用基础内容 |
| 数组项 `null` | 继承对应基础控件 |
| `{"type":"blank"}` | 用空控件覆盖该槽位 |
| 控件对象 | 替换相应控件 |
| cluster 数组 | 合并或替换 cluster，并调整两槽位占用关系 |

`LayerManager` 按当前运行时 layout ID 保存激活流；`UserInterface` 按 active Set 顺序将各层叠加到基础 content。多个层会产生顺序相关的覆盖结果。

来源：[LayerManager](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-12-tak-layout-and-engine/source-excerpts/tak-core-protocol.txt:930)、[数组与区域合并](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-12-tak-layout-and-engine/source-excerpts/tak-core-protocol.txt:1251)、[最终布局流](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-12-tak-layout-and-engine/source-excerpts/tak-core-protocol.txt:1360)。

## 8. 外观字段与交互状态

`styles.default` 提供默认样式，`idle / activated / moving / disabled` 等分支覆盖对应状态。button 常用 `faceImage / background`；joystick 常用 `knob / outline`；throttle 有上下轴与指示器样式。`faceImage.type: "icon"` 引用内置语义图标，`asset` 引用 bundle 图片。

| 字段 | 本快照语义 |
| --- | --- |
| `visible: false` | 视觉触控控件退出可交互集合 |
| button/touchpad/joystick 的 `enabled: false` | 保持 Disabled 外观，普通手柄输入继续处理 |
| button 的 layer/layout action + Disabled | 释放该客户端布局动作 |
| sensor 的 `enabled: false` | 关闭该传感器数据订阅 |
| `toggle` | 持续保持或切换 action 状态 |
| `pullAction` | 按住拖拉时增加一个动作或方向分段动作 |
| `scale` | 调整控件占据的视觉/交互尺寸 |

“游戏暂时不可执行动作”的视觉提示，可以由 `enabled` 表达；完整输入屏蔽需要明确选择可交互状态和输入路由语义。

来源：[Button 文档](https://learn.microsoft.com/en-us/gaming/gdk/docs/reference/system/touchadaptationkit/controls/game-streaming-touch-button)、[button 状态提交](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-12-tak-layout-and-engine/source-excerpts/tak-core-input.txt:871)、[可交互条件](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-12-tak-layout-and-engine/source-excerpts/tak-core-input.txt:294)、[sensor 订阅](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-12-tak-layout-and-engine/source-excerpts/tak-core-input.txt:2103)。

## 9. Context、引用与动态状态

Context 的 `state` 保存游戏提供的动态值。布局可以通过 JSON Pointer 引用这些值，例如布局文件位于 `layouts/neutral/gameplay.json`，Context 位于 bundle 根目录：

```json
{
  "type": "button",
  "action": "gamepadA",
  "enabled": {"$ref": "../../context.json#/state/fireEnabled"},
  "visible": {"$ref": "../../context.json#/state/fireVisible"}
}
```

解析器深拷贝原布局，展开内部 `#/definitions/...` 和允许的外部 Context 引用，检测循环引用，再校验展开后的 Layout。Context 的颜色样式与布局颜色合并；资产引用随后替换为渲染 key。

状态更新过程为：

```text
Base64 patch
  → UTF-8 解码
  → JSON Patch 操作数组
  → 更新 context.state 的副本
  → 重新展开并校验 Layout
  → 发布布局和控件 props
```

patch 的路径相对 `context.state`，例如 `/fireVisible`。实现为缺少开头斜线的 `path` 补上 `/`；`move/copy` 的 `from` 由 JSON Patch 自身处理。单条操作失败会被收集，后续操作继续执行，成功修改进入新 state；整包解码或 JSON 解析失败保留原 Context。

引用或展开后校验失败时，BundleProcessor 尝试原始 Context，再尝试此前已转换的布局。`allowedStateValues` 随 Context 声明允许值元数据；本快照的 patch 函数直接更新 state，布局展开后的校验承担后续检查。

来源：[引用展开](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-12-tak-layout-and-engine/source-excerpts/tak-core-protocol.txt:18)、[patch 更新](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-12-tak-layout-and-engine/source-excerpts/tak-core-protocol.txt:294)、[Base64 解码与处理](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-12-tak-layout-and-engine/source-excerpts/tak-core-protocol.txt:700)、[转换与回退](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-12-tak-layout-and-engine/source-excerpts/tak-core-protocol.txt:829)。

## 10. 串流消息协议

布局命令由 `message` DataChannel 的消息路由承载。外层信封的 `content` 为字符串；`showlayoutv2` 与 `patchstate` 再解析内部 JSON。普通触控命令使用 `type: "Message"`，路由处理器收到 Transaction 时执行 cancel。

| target 后缀：`/streaming/touchcontrols` | content | 内部动作 |
| --- | --- | --- |
| `/showlayout` | 布局 ID 字符串 | `showLayoutId` |
| `/showlayoutv2` | `{"layoutId":"gameplay","patch":"Base64…"}` | `showLayoutId`，可同时更新状态 |
| `/showtitledefault` | 可为空 | `showTitleDefault` |
| `/hide` | 可为空 | `hide` |
| `/patchstate` | `{"patch":"Base64…"}` | `patchState` |

`layoutId: ""` 解析为内置标准控制器；`showtitledefault` 选择当前标题 bundle 的默认布局。

可在 Node 中构造用于检查的 wire 示例：

```javascript
const operations = [
  { op: "replace", path: "/fireVisible", value: false }
];
const patch = Buffer.from(JSON.stringify(operations), "utf8").toString("base64");
const envelope = {
  type: "Message",
  target: "/streaming/touchcontrols/patchstate",
  content: JSON.stringify({ patch })
};
console.log(JSON.stringify(envelope));
```

better-xcloud 的通用布局回退使用本地 `RTCDataChannel.dispatchEvent(new MessageEvent(...))`，让官方消息监听器消费 `showlayoutv2 + layoutId:""`。它另行直接调用 LayoutCoordinator 应用社区对象。

客户端还报告 `/streaming/characteristics/touchinputenabledchanged` 的 `touchInputEnabled`，以及 `clientdevicecapabilities` 的 `maxTouchBundleLayoutVersion / maxTouchBundleManifestVersion`。在扩展触控模式下，better-xcloud 修改 session configuration 的 `enableTouchInput=true` 和 `maxTouchPoints=10`。

来源：[消息路由](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-12-tak-layout-and-engine/source-excerpts/touch-message-router.txt:8)、[转为引擎动作](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-12-tak-layout-and-engine/source-excerpts/game-stream-bridge.txt:8)、[本地回退消息](/Users/guo.xu/Documents/code/games/better-xcloud/src/modules/touch-controller.ts:29)、[配置扩展](/Users/guo.xu/Documents/code/games/better-xcloud/src/utils/xcloud-interceptor.ts:243)、[能力通知](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-12-tak-layout-and-engine/source-excerpts/touch-message-router.txt:89)。

## 11. scope 与 base / override 的选择规则

每个 scope 保存 `{base, override}`：

| 保存值或命令 | 结果 |
| --- | --- |
| `override` 是布局对象 | 优先显示用户覆盖 |
| `base` 是布局对象 | 显示游戏指定布局 |
| `base === null` | 选择当前标题默认布局 |
| `base === undefined` | 显示 Blank，仅保留宿主可用的原生触摸区域 |
| `showTitleDefault` | 将对应选择置为 `null` |
| `hide` | 将对应选择置为 `undefined` |
| `reset` | 删除整个 scope 记录，并清理待应用命令/patch |

`base` 承接游戏或服务端请求，`override` 承接用户选择。手柄优先输入模式、鼠标模式条件和 `disableTak` 还会影响最终可见布局。scope 管理布局选择，动态 Context 属于当前触控 bundle。

来源：[作用域存储与最终选择](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-12-tak-layout-and-engine/source-excerpts/tak-core-protocol.txt:1053)、[用户覆盖](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-12-tak-layout-and-engine/source-excerpts/game-stream-bridge.txt:104)。

## 12. 实际游戏布局体现的语义

| 游戏/模板 | 具体配置 |
| --- | --- |
| Forza | 左摇杆 `axisXY → leftJoystick`，steering 图标 |
| Forza | 油门 `rightTrigger`，gasPedal 图标；拖拉附加 `gamepadA` |
| Forza | 刹车 `leftTrigger`，brakePedal 图标 |
| Forza | `dPadDisplay` layer 展开方向键 |
| Forza throttle 变体 | `axisUp → rightTrigger`、`axisDown → leftTrigger`，`relative/sticky` |
| Halo 默认 `rightstick` | touchpad 位移映射到右摇杆，AimTouchpad 层开启陀螺仪 |
| Halo `relativemouse` 变体 | touchpad 映射相对鼠标，射击 action 同时激活瞄准层 |

这些配置将操作位置、图标、按钮组合、轴映射和临时操作面板组合成游戏专属体验。默认 Halo 路径使用右摇杆；相对鼠标变体的最终效果由会话鼠标输入能力决定。

本次保留两份社区模板及对应索引、三份通过官方校验器的最小文件，以及 41 个协议/引擎回放场景。建议先掌握游戏身份、槽位合并和 `action/axis`，再沿下一篇的输入状态机阅读渲染实现。
