# SR 兼容采样与 USM 保护优化

日期：2026-11-10；主窗口单窗口执行。用户在采样审计后授权“优化一下”。

## 交付

- EASU 兼容 gather 每组位置一次读取 RGB，再按原顺序拆通道。源码展开的 texelFetch 从 48 次降为 16 次，12 个有效空间采样点、权重、边界夹取与重建尺寸保持原合同。原编译器可能已去重，实际硬件收益接续目标 GPU 对照。
- USM 保持对齐时 5 次 texture 读取、等价完整 3×3 高斯核。复用已有四次双线性采样与中心值构建保守邻域边界；非对齐路径复用原八邻域。亮度残差从 1/255 到 2/255 使用软噪声门限，色度混合与过冲约束复用既有 CAS 参数。
- 零锐化继续跳过处理；CAS 算法、GPU pass 数量、即时解码帧提交、在途帧预算与故障降级沿现有合同执行。新增保护只使用已有采样及局部数学运算。

## 验证

7 个相关前端文件的 184 项回归通过；pnpm lint:fix、typecheck、build、脚本语法与 git diff --check 通过。

真实 WebGL2 像素验证使用独立无界面 Chromium / ANGLE SwiftShader、RGBA8 输入输出。当前生产 shader 模板与常量直接提取；优化前 EASU 与锐化模板保存在本目录。输出见 [pixels.metrics.json](pixels.metrics.json)，源码指纹见 [source-identity.json](source-identity.json)。软件 GPU 验证编译、采样及数值行为；硬件帧率、GPU 长尾与物理上屏接续设备日志。

| 场景 | 结果 |
| --- | --- |
| EASU 同尺寸、1.5 倍、2 倍及非整数尺寸四组 | 优化前后最大像素差均为 0 |
| USM 暗部噪声、细纹理、阶跃边缘、斜线、RGB 细节、非整数缩放六组 | GPU 对 CPU 参考最大差为 0–1 个码值 |
| SR 重建后 USM 强度 0 / 0.6 / 1.0 三组 | GPU 对 CPU 参考最大差均为 0 |
| 灰色背景 ±1 码值噪声 | 优化前最大新增偏移 1，优化后 0 |
| 125 / 131 码值交错细纹理，强度 0.6 | 保留增强，最大偏移 1 个码值 |
| 96 → 160 阶跃边缘，强度 0.6 | 最大光晕偏移 13 → 12 个码值 |
| 同一阶跃边缘，强度 1.0 | 最大光晕偏移 21 → 13 个码值 |
| 零锐化旁路、CAS 与优化前对照 | 最大像素差均为 0 |

采样预算回归约束 EASU 12 个 tap、4 组 RGB gather、兼容路径每组 4 次 fetch。USM 数学回归覆盖低亮/中亮/高亮噪声、细纹理、正负边缘过冲、零强度、平坦颜色与色度保护。上述结果限定于确定性图案；实际编码纹理及运动观感由新构建实机对照。

## 复算

```sh
./node_modules/.bin/vitest run src/player/infra/render/sharpening-math.test.ts src/player/infra/render/fsr1-reconstruction-pass.test.ts src/player/infra/render/Renderers.test.ts src/player/infra/render/renderer-scheduling.test.ts src/player/infra/render/decoded-video-frame-reader.test.ts src/player/infra/render/video-frame-source.test.ts src/player/app/media/PlaybackService.test.ts --reporter=dot
node dev-docs/validation/2026-11-10-sr-sharpen-sampling-optimization/run-pixels.mjs
pnpm lint:fix
pnpm typecheck
pnpm build
```

像素运行器复用 Vite 所带 esbuild，默认使用 /Applications/Google Chrome.app 中的 Chrome；XBX_SAMPLING_CHROME 可指定另一安装路径。独立临时 profile 与本地 file 页面隔离测试；本次沙箱内 Chrome 启动超时，获授权后在沙箱外验证成功。

建议使用默认直通路径，以 720p→1080p、1080p→1440p 的相同运动画面，对照原画、SR、USM 及组合。同步核对网络/恢复链健康、media supply、steady supply、帧供给 delta 和 submit/present 长尾，优先保证画面新鲜度。
