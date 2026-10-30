# Renderer 冗余操作清理验证

2026-10-30，本批代码与离线验证完成。用户授权先清理渲染逻辑中的多余耗时，沿现有浏览器 runtime RFC 执行；验证使用生产模块、受控输入与 DOM/WebGL 替身。

## 已完成的减法

- 普通 WebGL 零锐化直接单 pass 输出；源尺寸锐化 FBO 和呈现程序按实际需要准备。重复画质配置、单独 FPS 更新复用已应用的 uniform；完整配置统一刷新，减少重复刷新和按旧配置准备资源。
- SR 等尺寸仅准备 RCAS，实际放大优先准备融合 shader，融合编译失败后准备 EASU + RCAS。兼容选择在当前 context 内复用，uniform 只更新当前路径。
- 中性亮度、对比度、饱和度跳过对应颜色算术。直接绘制的 viewport 在初始化和尺寸变化时设置；SR 纹理单元 0 每 context 设置一次，opacity 在首次呈现和 context 恢复后设置。
- 减少重复纹理绑定；空截图队列在构造读回闭包前返回。保留首次创建、复用、源/输出尺寸变化与双 pass 切换中的正确纹理采样。
- SR context lost 后保留 restore 监听，恢复时重新准备当前程序；销毁完整移除监听并清空 uniform 引用。未就绪源尺寸使用 0，维持有效输出尺寸更新。

帧源、累计节拍、源尺寸下限、增强算法主体、用户配置和故障回退沿原路径执行。本批范围为 renderer 冗余工作；原生帧率优先的健康判据与 GPU 分段测量继续单列。

## 操作计数证据

[回放脚本](2026-10-30-render-redundant-work-audit.cjs)仅将本批三个 renderer 模块替换为 Git HEAD 版本形成优化前对照，其余源码与 harness 使用当前版本。每组输入为 60 个源帧，再执行一次 FPS 更新后的新帧；各组优化前后实时处理均为 61 次，重复处理和提交失败均为 0。

| 场景与操作 | 优化前 | 优化后 |
| --- | ---: | ---: |
| 普通 WebGL 初始化程序数 | 2 | 1 |
| SR 等尺寸 / 融合放大初始化程序数 | 3 | 1 |
| 零锐化且输出尺寸变化：GPU draw 调用 | 122 | 61 |
| 同场景：FBO 分配 | 1 | 0 |
| 普通 WebGL 重复配置与 FPS 更新后的 uniform 写入 | 24 | 0 |
| 普通 / SR 等尺寸纹理绑定 | 123 | 62 |
| 普通直接绘制 viewport 设置 | 62 | 1 |
| SR viewport / activeTexture 设置 | 61 | 1 |
| SR opacity 写入 | 61 | 1 |

GPU draw 调用按 pass 计数，实时处理按源帧计数。双 pass SR 的 30 次实时处理仍对应 60 次 GPU draw，独立截图对应额外两次 GPU draw；截图与实时新帧并发时共用一次实时处理。操作次数证明代码路径减法；GPU 执行时间、驱动优化与实际上屏效果由设备测量验收。

## 检查结果与源码身份

- `pnpm test`：45 文件 / 360 项通过。
- `pnpm lint:fix`、`pnpm exec vue-tsc --noEmit`、`pnpm build`、`git diff --check` 通过。
- 新增回归覆盖懒编译、零锐化单/双 pass 切换、中性色彩、融合失败兼容路径复用、活跃 uniform 更新、context 恢复、viewport 和采样纹理身份。
- 原 9 组供给回放及 RCAS / 融合 / EASU + RCAS / 截图记账通过；48 帧计数偏移回放保持 300 个源帧 → 300 个 marker → 300 次实时 SR 处理，稳态 30fps。

构建源码指纹为 `9fb35ae1b43c6b6b924ebb2c9a10ee97558c46f000712b1e7d79d56f01579a61`，728 个输入，采用 `src-tauri/build.rs::source_fingerprint` 算法。[验证数据](2026-10-30-render-redundant-work-validation.metrics.json)包含基线提交、逐文件 SHA-256、操作计数、供给和截图回放。离线结果中的源码摘要已与最终验证源码逐项匹配。

## 设备验收范围

| 维度 | 本批证据与后续验收 |
| --- | --- |
| 网络健康 | 本批为离线受控输入；ROG 的 loss / RTT / jitter / 接收增量由新日志确认 |
| 恢复链健康 | 本批无恢复 episode；ReferenceChain / cleanAnchorCommitted / DisplayStable 按运行时可观测边界取证 |
| media supply | 回放逐项对齐源输入、marker、实时处理和 GPU pass |
| steady supply | 30/60fps 受控供给与计数偏移回归通过；ROG 1440p 前台稳态继续配对 |
| submit/present 长尾 | mock GPU 耗时与真实呈现均 unavailable；实际纹理上传、GPU 执行和提交年龄待设备证据 |
| 帧供给 delta | 实时处理、截图、双 pass 分列；ROG 接收 / 解码 / 候选 / 提交按相同窗口对齐 |

修复前 ROG 1440p 的接收/解码 60fps、渲染约 40fps 与提交率 84% 保留为设备基线。本批减少已确认的冗余操作，改善幅度由修复后同源、同设置的日志确认。下一轮优先对齐源帧率与实际提交及长尾，再处理健康判据；完整平台、功能和 2h/8h 验收继续沿 S4 执行，自动软件操作保持暂停。
