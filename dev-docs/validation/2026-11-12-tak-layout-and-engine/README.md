# better-xcloud 布局协议与 TAK/Babylon 引擎核对证据

核对日期：2026-11-12。本目录保存公开发布脚本的来源与哈希、关键原文摘录、社区布局样本、官方 schema 最小文件，以及直接提取原方法执行的隔离回放。

分析正文：[布局协议](/Users/guo.xu/Documents/code/games/xbxrc/docs/references/better-xcloud-touch-layout-protocol.md) · [TAK/Babylon 引擎](/Users/guo.xu/Documents/code/games/xbxrc/docs/references/tak-babylon-touch-engine.md)。

## 文件与来源

| 文件 | 内容 |
| --- | --- |
| [sources.json](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-12-tak-layout-and-engine/sources.json) | 六份微软发布脚本 URL、原件/格式化文件 SHA-256、better-xcloud 两个 commit |
| [source-excerpts/index.json](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-12-tak-layout-and-engine/source-excerpts/index.json) | 七份摘录文件、36 段原文的源文件行号、摘录行号和 SHA-256 |
| [examples/forza-title.json](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-12-tak-layout-and-engine/examples/forza-title.json) / [halo-title.json](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-12-tak-layout-and-engine/examples/halo-title.json) | 冻结游戏索引 |
| [examples/forza-layouts.json](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-12-tak-layout-and-engine/examples/forza-layouts.json) / [halo-layouts.json](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-12-tak-layout-and-engine/examples/halo-layouts.json) | 冻结社区模板 |
| [examples/layout.json](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-12-tak-layout-and-engine/examples/layout.json)、[context.json](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-12-tak-layout-and-engine/examples/context.json)、[manifest.json](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-12-tak-layout-and-engine/examples/manifest.json) | 通过冻结官方 latest 编译校验器的最小文件 |
| [replay.cjs](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-12-tak-layout-and-engine/replay.cjs) | 哈希验证、AST 方法提取、VM 回放与结果断言 |
| [replay-results.json](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-12-tak-layout-and-engine/replay-results.json) | 41 个行为核对场景及具体观测值 |
| [document-checks.json](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-12-tak-layout-and-engine/document-checks.json) | 原件/可读文件哈希、36 段原文一致性、文档链接与源码行号核验 |

发布原件保存于清单的 `localRawFile`，完整可读文件保存于 `localReadableFile`。格式化使用 Terser 5.46.0，`compress=false / mangle=false / beautify=true`。仓库内摘录保留原可读文件文本，按章节选择区间；每段头部明确源文件行号，完整上下文可由清单中的发布 URL 恢复并校验。

社区源来自 better-xcloud 代码 commit `76db611e610c45ea996f6d35a9df5f980e8f0f85` 与 gh-pages commit `d66bd2925ee4518a955955b0b52890a407575285`。

## 回放结果

| 类别 | 场景数 | 覆盖 |
| --- | --- | --- |
| protocol | 20 | Layout/Context/Manifest 校验、社区包装、消息、scope、layer、Base64 JSON Patch |
| pointer | 4 | Start/Move/End、转入新控件、碰撞边界与优先级 |
| input | 7 | Disabled 视觉与动作、共享按钮、轴饱和/抵消、嵌套帧、鼠标小数、归零阈值 |
| axis | 4 | 相对/绝对摇杆、单位圆、轴向和径向 deadzone |
| axis-observation | 2 | circular / circular-inverse 的非有限值异常复现 |
| render | 4 | 空闲、脏叶节点、根更新和空集合清屏 |
| 总计 | 41 | 全部观测与记录断言一致 |

重要观测包括：layer 的 `null` 继承基础槽位、`blank` 显式覆盖为空；共享按钮释放保留其他控件贡献；模拟轴保留限幅前总和；鼠标保留小数余量；patch 部分成功的修改进入新 Context；本快照的 responseCurve 分母为零，产生 NaN/Infinity。

## 运行方式

在 xbxrc 仓库根目录运行，使用已有 Node.js 与 `typescript` 依赖：

```sh
node --check dev-docs/validation/2026-11-12-tak-layout-and-engine/replay.cjs
node dev-docs/validation/2026-11-12-tak-layout-and-engine/replay.cjs
```

默认读取 `sources.json` 的原件本地路径。恢复原件后，可把六份 `.chunk.js` 放在同一目录并显式指定：

```sh
node dev-docs/validation/2026-11-12-tak-layout-and-engine/replay.cjs /private/tmp/xbxrc-tak-replay-sources
```

脚本先校验六份 SHA-256，再使用 TypeScript AST 提取实际类、方法、辅助函数和 Base64 解码器。官方编译校验器在最小 webpack 宿主中加载；输入、指针和渲染方法在 VM 中执行。运行结束将更新 `replay-results.json`。

## 宿主与测量范围

VM 提供枚举、日志、流订阅入口等最小依赖；渲染 fixture 用计数器替代 engine、节点和 group，以核对原方法的分支。回放验证的对象是发布方法与协议行为，`realBrowserWebGLMeasured=false`。

真实 DOM 触摸、WebGL shader/GPU、浏览器功耗、触摸到远端输入时延、触觉和长时资源行为需要浏览器与实机证据。异常场景的 pass 表示与冻结快照行为一致，产品参数可用性需结合有限值和端到端验证。

三条 latest raw schema URL 在本轮返回 404；字段验证采用冻结的官方编译校验器，格式说明参考 Microsoft Learn 文档。版本声明和校验器实现随公开网页发布快照记录。

建议从两篇正文进入对应原文摘录，再用回放结果核对边界值与状态语义。
