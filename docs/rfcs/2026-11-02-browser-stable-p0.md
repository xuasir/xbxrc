# 浏览器稳定版 P0 收口 RFC

## Status

- Completion: 代码与本地检查完成，平台与长测按证据逐项验收
- Current State: validation-pending
- Owner: 当前窗口直接实施与验证
- Last Updated: 2026-11-02
- Execution authorization: 用户在当前聊天明确要求“按这个路径去改造，先解决P0问题”。
- Baseline: `65e28495`，工作树干净，前端460项/类型/生产构建通过。

## Goal / Scope

稳定默认使用 Browser Gamepad API、浏览器 WebRTC 和中性 video/audio 直出。Rust继续承担Tauri壳、认证、远端会话、信令及系统集成。原生手柄与Rust媒体分别提供默认关闭的预览开关；现有浏览器增强、公共输入与恢复合同继续保留。

本轮覆盖上一轮分析中的P0第1/2/3/4/7项。P1诊断聚合及Cargo/FFmpeg发行裁剪后续独立推进；现有同包预览依赖继续完整交付。

## Implementation contract

1. `rust_media_preview`、`native_gamepad_preview`两个独立布尔默认false。每个开关直接决定对应后端，删除旧`stream_runtime_mode/gamepad_backend/use_vulkan/polling_rate`活动配置与重复选择器；有效装配要求对应预览许可与宿主能力。RPC启动同样校验，关闭预览影响后续会话/下次应用启动，当前会话继续使用自身装配。
2. 原生引擎状态与16ms tick按预览会话创建/回收，macOS原生透明宿主在原生会话启动时配置。Browser输入独立启动；原生手柄继续按需装配。控制epoch原子捕获、停止取消、共享停止任务与单一当前runtime；启动失败清理按实例身份隔离。
3. 输入发送结果贯通到InputService；有界背压队列保留按钮边沿/中性释放和最新模拟值，成功才提交metadata/发送统计。单一重试/心跳驱动在关闭时回收；旧通道的待发数据在新通道建立前失效。
4. Player默认值统一为中性video输出。浏览器rAF采样事实替代陈旧250Hz用户合同，rAF缺少新采样100ms时定时读取真实设备，实际采样率继续从高级面板测量；共享领域原生输入参数按实际消费者保留。
5. 发布CI执行前端lint/类型/测试与相关Rust检查/测试及公共生命周期trace门禁，产物验证沿原流程。同包预览的动态依赖在进程启动时加载，macOS SDK使用包内install name、系统依赖和明确rpath，并对真实app检查依赖闭包。平台验收使用真实安装包，记录WebView/系统/设备/构建身份、输入、网络、恢复、media/steady supply、帧差及呈现观测边界。
6. 用户明确确认“FFmpeg保持26的要求即可”；应用、SDK、构建部署目标与macOS CI宿主统一为26.0。复用FFmpeg 8.1现有资产，归一化install name并把ARM64传递依赖固定随包分发。

## Validation

证据见[代码与交互验收记录](../../dev-docs/validation/2026-11-02-browser-stable-p0-validation.md)。

- [x] 新配置默认关闭预览、独立许可/能力矩阵、设置保存/重开与RPC启动许可合同。
- [x] 空装配槽、唯一实例、并发停止共享结果、停止期间重启隔离；实际服务资源回收待实机。
- [x] 背压、send失败、快速按钮边沿、中性优先、metadata提交、通道替换/退出清理回归。
- [x] 默认video、现有画质增强、SR回退/供帧/截图回归。
- [x] lint/fix、类型、473项前端测试、生产build；fmt、208项Tauri测试；22组生命周期回放与产物模拟。
- [x] macOS SDK完整依赖、签名、架构、系统版本与摘要检查；8项门禁及全部FFmpeg库的隔离加载探针。
- [x] 最终macOS app/dmg构建；app及迁移目录15个运行二进制检查，DMG只读payload的86个文件一致性与CRC检查。
- [ ] 真实运行中关闭预览、原生启动/失败/停止/重启/退出及Browser零原生实例/tick资源证据。
- [ ] Windows/ROG真实手柄/FSE/后台/触觉；macOS与Windows home/cloud取消、恢复、退出及资源回收。
- [ ] home/cloud各2小时，核心稳定路径8小时长测；GitHub CI真实runner验收。

## Risks / Evidence boundaries

WebView挂起期间采样和发送由系统调度决定，keep-active继续表达公共路由许可。浏览器内部ReferenceChain/cleanAnchorCommitted/DisplayStable与实际scanout保持各自观测边界。平台与长测完成需要真实设备/会话证据，自动实机运行沿既有暂停约定。

## Progress

- 2026-11-02 发行验收：release编译期宏在`strip=true`下丢失可加载信息；`profile.release.build-override.strip="none"`对照通过，完整app/dmg编译打包已成功。真实产物仍引用`/tmp/ffmpeg-8.1/...`，ARM64 FFmpeg依赖Homebrew X11/XCB；按P0-7修复安装包加载合同。FFmpeg功能/依赖的发行裁剪继续归P1。
- Browser会话替换回归新增两项：旧DataChannel事件解绑、旧peer事实隔离、失败后重新bind释放资源、控制及输入定时器回收；50文件/473项前端测试与类型检查通过。
- macOS交付：系统要求统一26.0；SDK使用包内install name并固定ARM64传递依赖。Tauri装配标准Frameworks并统一签名，ad-hoc与证书分别匹配Hardened Runtime策略；8项门禁和隔离版本查询通过。最终app及迁移目录15个运行二进制检查通过，DMG的CRC有效且内含86个文件与构建app逐项一致。产物SHA与构建身份见验收记录；本地包版本0.1.0，正式发行签名及实机/CI/长测继续待验。
- 2026-11-02用户再次确认“保持自动运行暂停，手动验收”；验收记录已提供具体产物与手动顺序，平台和长测状态保留validation-pending。

- [x] P0-1 预览门禁与有效配置
- [x] P0-2 原生按需生命周期代码；真实媒体/GPU/系统资源回收待实机
- [x] P0-3 输入发送可靠性
- [x] P0-4 默认值与采样合同
- [ ] P0-7 发版门禁代码与回放完成；CI runner、平台及长测待验
