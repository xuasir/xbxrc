# 渲染控件与顶部指标栏验证

关联 [RFC](../../../docs/rfcs/2026-11-09-render-controls-clarity.md)。本目录保存实际 Vue 组件的隔离预览与浏览器验收截图。帧率、网络指标、renderer 提交状态和持久化采用模拟输入；实机渲染吞吐与物理上屏继续通过设备 trace 验收。

## 已观察行为

- SR 开启后独立锐化显示关闭；选择清晰或柔和关闭 SR。使用默认渲染选择 Video。
- 触屏、鼠标与几何导航共享控件动作；本次应用与保存默认分别提交。
- 弹层注册完成后聚焦 SR。窄屏向下依次到达锐化、默认渲染、色彩、恢复默认，随后进入固定操作栏；区内焦点移动会显露滚动内容。
- 顶部指标栏随既有开关整体显示、隐藏；模拟 SR 回退时显示“默认渲染 · 帧率保护”，所选 SR 保留。

## 布局证据

| 场景 | 观察结果 | 截图 |
| --- | --- | --- |
| 1280 × 800，本次设置，深色 | 默认焦点 SR；标题、渲染控制、色彩和固定提交栏可见，内容区独立滚动 | [本次设置](session-controls-dark.jpg) |
| 1280 × 800，全局默认，浅色 | 默认焦点 SR；内容完整显示，提示保存后用于后续串流 | [默认设置](default-controls-light.jpg) |
| 360 × 640，本次设置，深色 | 方向键显露三档锐化；固定操作栏两行，高 104px | [滚动导航](narrow-controls-scrolled.jpg) |
| 1280 × 800，增强回退 | 指标栏单行，高 32px，宽约 526px | [顶部栏](wide-fallback.jpg) |
| 360 × 640，增强回退 | 指标栏两行，高约 42px，宽 328px | [窄屏顶部栏](narrow-fallback.jpg) |

各场景页面横向溢出检查通过。尺寸、焦点与 DOM 边界记录见 [visual-checks.json](visual-checks.json)。[修正前窄屏](before-narrow-controls.jpg)保留底部三行操作占位与默认焦点提前落到关闭按钮的对照。

## 复现预览

从仓库根目录运行：

```sh
node node_modules/vite/bin/vite.js --config dev-docs/validation/2026-11-09-render-controls-clarity/preview/vite.config.mjs
```

打开 `http://127.0.0.1:8951/?open=1&scope=session&theme=dark`。`scope=default` 查看全局默认入口；`theme=light` 查看浅色；`mode=sr&fallback=performance` 查看模拟回退。预览从 `src` 导入生产组件、样式和几何引擎；键盘方向键与 Enter 经现有 InputDispatcher 发送相同导航意图。

## 实机验收

保持 1440p/60fps，依次使用默认渲染、锐化柔和、锐化清晰、SR；检查触屏、手柄操作和顶部实际模式。普通应用后重新进入设置页核对全局默认；显式保存后开始新串流核对默认。增强回退、持续帧率和真实呈现继续由掌机日志确认。
