# 视频纹理上传入口优化验证

2026-10-30。普通WebGL与SR已将新video帧导入切换为现有纹理对象上的6参数 `texImage2D(video)`，使WebKit具备GPU帧拷贝尝试条件。48文件/413项全量、lint、类型检查、构建及离线回放通过。实际GPU命中、上传成本下降、保持SR效果的60fps与像素输出由新构建设备日志/画面验收。

## 定位依据

[14:39新日志](2026-10-30-display-deficit-fresh-trace-validation.md)源码匹配：两次SR处理44.53/44.64fps，496次处理加权总耗时14.82ms、视频上传14.67ms、GL命令提交0.149ms，上传占同步处理98.95%。同窗约60fps接收/解码与候选→处理完整交付成立；网络丢失0.03872%、RTT中位11ms，SR两段freeze增量0。原生video回退恢复约59.94fps推进计数。

读取公开[WebKit源码](https://github.com/WebKit/WebKit/blob/main/Source/WebCore/html/canvas/WebGLRenderingContextBase.cpp#L3584)，`texImageSource(HTMLVideoElement&)` 的GPU拷贝尝试要求：TexImage2D、完整源矩形、level0、RGB或RGBA、UNSIGNED_BYTE。此前稳态 `texSubImage2D(video)` 使用软件分支。Cocoa后端将当前videoFrame交给 `copyVideoSampleToTexture`，实际命中由视频帧类型和后端决定；失败时浏览器沿其原有软件路径完成相同WebGL调用。

引用源码为2026-10-30读取的main版本；两份源码SHA256保存于metrics。已安装WebView的具体实现、驱动同步等待与GPU执行耗时继续由实机证据核验。上传计时包含转换、拷贝和等待，其分项因果占比继续缺证。

## 代码改变

- 普通WebGL逐新帧直接 `texImage2D(TEXTURE_2D, 0, RGBA, RGBA, UNSIGNED_BYTE, video)`；去掉video源的空存储预分配，沿现有源尺寸读取输入，输出canvas尺寸按原计划独立处理。
- SR逐新帧直接 `texImage2D(TEXTURE_2D, 0, RGB, RGB, UNSIGNED_BYTE, video)`；沿用已有video纹理、RGB格式、Y翻转、颜色处理和单/双pass采样绑定。
- 首次与源尺寸变化仍执行即时错误检查，稳态错误读取继续最多约1Hz。纹理对象、帧身份、latest-only、用户限帧、前后台、截图、context恢复、硬错误回退与软回退策略继续使用现有所有者。
- renderer资源快照增加 `videoUploadMethod=texImage2D`，随现有低频资源trace发布，表达应用调用方式。GPU拷贝命中状态继续unavailable。

WebGL API以同一纹理对象定义当前video图像，后台存储复用或重新定义由WebView/驱动决定。本轮没有为视频逐帧创建WebGLTexture、ImageBitmap、VideoFrame或另建媒体队列。

## 自动验证

新增2个回归在原实现得到2失败/49通过，修复后renderer两文件62项通过。全量48文件/413项通过；`pnpm lint:fix`、`pnpm exec vue-tsc --noEmit`、`pnpm build`、`git diff --check`通过。覆盖新帧单次导入、重复身份跳过、RGB/RGBA调用合同、纹理对象复用、尺寸增长、FBO与单/双pass采样、截图并发、context恢复与资源关闭。

[生产调用回放](2026-10-30-video-texture-import-audit.cjs)复用生产模块及既有DOM/WebGL替身。基线取本轮实施前的暂存renderer，模块SHA256和结果保存在[metrics](2026-10-30-video-texture-import-validation.metrics.json)；历史重跑需要保留该基线版本。

每组提供60个新video帧及一次暂停后截图，实际处理60次、截图1次：

| 路径 | 优化前满足WebKit尝试条件的导入 | 优化后 | 纹理对象数 | GPU draw调用 |
| --- | ---: | ---: | ---: | ---: |
| 普通WebGL等尺寸 | 0/61 | 61/61 | 1 | 61 |
| 普通WebGL锐化后放大 | 0/61 | 61/61 | 2 | 122 |
| SR等尺寸RCAS | 1/61 | 61/61 | 1 | 61 |
| SR融合放大 | 1/61 | 61/61 | 1 | 61 |
| SR EASU+RCAS | 1/61 | 61/61 | 2 | 122 |

五组对象数量、处理/截图记账和draw调用与基线一致，全部video导入使用TexImage2D。普通video空预分配减少一次；FBO存储分配和采样维持原合同。此回放验证API准入与操作计数，GPU实际命中、后台存储和像素正确性由设备验收。

原9组供给回放、3组48帧偏移回放、4组冗余工作回放均通过。正常身份供给维持30fps，偏移稳态30fps；身份实际合批的4fps/3.7fps用例保留原语义，截图与单双pass记账一致。

## 设备验收与限制

本轮修改限于前端renderer及其资源事实，网络、浏览器恢复与源供给按14:39基线保留。新运行继续核对同窗网络健康、浏览器恢复可观测事实、media/steady supply、接收→解码→候选→提交delta与提交新鲜度；ReferenceChain、cleanAnchorCommitted、DisplayStable、NACK逐请求有效率、GPU执行、真实screen present及submit-to-present的P95/P99继续保持观测边界。

自动实机操作保持暂停。手测用新构建、同一主机、1440p/60fps持续前台开启SR至少30秒，核对构建指纹、实际SR/generation、videoUploadMethod、上传/提交CPU成本、完整3s/15s候选与处理率、回退/试探、画面方向/颜色及截图。以保留SR效果、上传耗时显著下降、实际处理稳定达到预期源预算98%为验收；Windows/ROG按相同合同做配对测试，后台存储/驱动差异按平台日志评估。

复算：`node dev-docs/validation/2026-10-30-video-texture-import-audit.cjs <output.json>`。
