# 浏览器标准 SR 输出与最新帧供给修正验证

2026-10-29：用户要求的标准 SR 目标和帧供给修正已完成代码与自动验证。接收目标帧率保持用户设定，设备运行效果进入新构建手测。

依据：[成果计划](../../docs/isu/2026-10-29-browser-policy-stability-and-native-quality.md)、[实施 RFC](../../docs/rfcs/2026-10-28-browser-runtime-simplification-and-stability.md)。机器可读的档位复算、源码身份与验证范围见[验证数据](2026-10-29-browser-standard-output-and-latest-frame-validation.metrics.json)。

## 标准输出合同

基础档继续采用协商档与真实源档的当前计算方式。显示需求选择标准目标，ladder、renderer plan 与实际 SR backing 使用同一尺寸对，CSS 视口承担最终显示映射。目标容纳真实源时采用标准尺寸；源越过目标时保留完整源尺寸，既有稳定重算继续处理后续目标。

| 实际源 | 显示需求 | SR 处理输出 |
| --- | --- | --- |
| 1280×720 | 1600×900 / 1707×960 | 1920×1080 |
| 1920×1080 | 2000×1125 | 2560×1440 |
| 2560×1440 | 3200×1800 | 3840×2160 |
| 2560×1440 | 1600×900 | 2560×1440，passthrough |

标准目标覆盖 1080p、1440p、2160p，真实源下限继续贯穿 GPU 分配和回退。源宽高发生不同步变化时，超出目标的源成对保留，避免组合成临时混合尺寸。

## 最新帧调度与接收权限

同一 VideoFrameSource 持有一个 RVFC 和一个 rAF 轮询入口，metadata、统计与 renderer 共用帧身份和取消代际。轮询读取 `getVideoPlaybackQuality` 的总帧减丢帧计数，并提供 WebKit 计数兼容入口；实际推进时交付最新 marker，重复轮询继续等待。

RVFC 与轮询按已交付增量互相抵扣，晚到 metadata 继续进入原始输入反馈；计数暂时缺证时保留已有基线，恢复后继续抵扣。首次 RVFC 零计数建立当前帧身份，后续正增量正常交付。源 FPS 估算继续使用原始 metadata 的计数/媒体时间增量。

标准 WebGL 与 SR 都收到新帧后立即尝试处理。明确用户显示上限由累计节拍执行，候选保持 latest-only；隐藏预算暂停本地绘制，恢复后处理当前最新帧。截图、候选首帧交接、GPU 故障回退与旧代际取消继续使用现有责任模块。

接收帧率请求始终保持用户目标，网络适应沿既有有限码率请求执行。SDP 帧率属性、source/codec 配置及 sender 的 `maxFramerate` 权限分别有回归；实际接收、解码和本地供给变化继续分列观测。

面板 callback FPS 和周期交付计数采用新帧 marker 数，video advancement 采用统一帧增量。原始 RVFC 次数、轮询补足次数与计数来源单列在媒体资源快照；共享轮询的在途上限和退出归零进入报告门禁。

## 自动证据

| 验证 | 结果 |
| --- | --- |
| `pnpm test` | 44 文件 / 326 项通过 |
| `pnpm lint:fix` | 通过 |
| `pnpm exec vue-tsc --noEmit` | 通过 |
| `pnpm build` | 266 模块，生产构建通过 |
| Python 浏览器稳定性报告 fixtures | 18 项通过 |
| `git diff --check` | 通过 |

调度用例使用受控时钟和真实 renderer 实现，GPU 采用 mock：RVFC 15fps、实际源 30/60fps、60/144Hz 显示、mediaTime=0 的 10s 用例保持提交数与供给数的首尾差至多 2 帧，重复实时处理为 0。已有 90/144/165Hz、32/34ms 源、latest 覆盖、暂停恢复、截图、实际资源变化与 GPU 故障回归一同通过。

新增边界覆盖首次零计数、计数领先/落后 RVFC、不同计数起点、暂时缺证及恢复、原始 RTP metadata、source FPS 对齐、丢帧计数独立性、取消后的晚到回调和标准输出成对守卫。

## 源码身份与后续手测

当前为 `bec3acfb` 工作树修改，按 `src-tauri/build.rs` 同一算法计算 726 个输入路径，源码 SHA-256 为：

```text
4f1e7668766be3f1e6329f0b732dfa0d07a0de78f2fd4a5cdff13626bca38800
```

[14:24 Mac/cloud 日志](2026-10-29-browser-policy-fresh-trace-validation.md)的 `fac1d02f…` 版本作为修复前基线；其同窗中位 video advancement 25.98fps、callback 15.28fps、SR submitted 13.48fps 与前后台切换事实继续保存。

16:47 新构建日志已完成[持续前台查验](2026-10-29-browser-standard-output-fresh-trace-validation.md)，源码指纹与本批修正完全匹配。源下限、1440p 同尺寸与720p→标准1080p输出通过；完整3s/15s窗口显示接收/解码/video约30fps，marker与SR提交记录约3.6fps。用户补充实际观感明显好于3fps，后续优先核验video/canvas可见层与绘制记账，实际视觉帧率继续取证；周期采样边界与共享样本继续验收。

自动启动应用和串流继续按用户约定暂停。下一次手测使用新构建，保持连续前台并打开统计面板，核验真实源/标准输出、计数来源、补足交付次数、五段同窗帧差及提交年龄；再执行 SR 开关配对和窗口切换恢复。CPU 提交、GPU/真实上屏与网络缓冲分别取证。

本批 Rust 生产源码改动为 0。同范围当前为 62 个生产文件、15,222 行、6,898 语句、1,297 分支，较基线减少 452 行和 141 语句；runtime 直属可变绑定保持 74。完整 S4 功能、平台、性能配对和长测继续按实施 RFC 验收。
