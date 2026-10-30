# 公共显示帧率缺口修复与自动验证

2026-10-30。本轮已完成中性 auto 原生呈现、完整源预算下的持续帧差保护、实时提交年龄、媒体首候选期限和上传/提交 CPU 分段计时。全量 47 文件/399 项测试、lint、类型检查、构建及四组离线回放通过。设备最终吞吐、超分保留效果和实际屏幕呈现继续由新构建手测日志验收，S4 保持 In Progress。

## 证据与故障定位

沿用[最新网络/渲染基线](2026-10-30-network-render-trace-audit.md)，对应 `runtime-trace-1793329037119-1.jsonl`，日志 SHA256 为 `5506151d0a26ea2b391ee1715f1c47580e1f611486e4d78204264ff0bc6cf6e2`。基线在修改前核对构建源码指纹；本报告对应后续修复源码。基线文件保留原始结果。

- SR 1440p 稳态接收/解码约59.94fps，34.475秒内1498个原始 RVFC、1498个接受候选、1498次实时处理，43.45fps；完整候选→处理交付成立，解码→候选缺口约16.5fps。
- 同窗网络丢失率0.02422%，freeze增量0。10:58:18的集中丢包与冻结独立保留网络专项证据；持续 SR 缺口定位到解码后阶段。
- SR 同步处理均值14.84ms，接近16.67ms帧预算，具有主线程调度压力关联。新增 CPU 分段计时支持下一轮区分 video 上传与 GL 命令提交的成本。
- 原生 video 稳态推进计数约60fps、候选约52fps；关闭 SR 的实际日志管线为启动回退后的 video。普通 WebGL 与 SR 共用纹理上传和候选入口，回归覆盖两种 canvas 管线。
- 旧 telemetry 提交年龄中位161ms，实时资源中位8ms。缓存采样间继续 draw 时，旧年龄累加形成虚高。启动候选在媒体到达前5001ms失败，与原5秒期限一致。

恢复链边界沿基线保留：PLI增量2、NACK70、关键帧解码11、RTX包846；浏览器内部逐请求配对、ReferenceChain、cleanAnchorCommitted、DisplayStable与修复有效率继续unavailable。browser runtime 门禁通过，完整输出供给由接收、解码、候选与实际处理差值独立验收。

## 修复行为

| 范围 | 最终行为 |
| --- | --- |
| 中性 auto | SR关闭、锐化0、亮度/对比度/饱和度100时直接装配 video，复用浏览器呈现。显式管线与增强意图继续按用户配置装配。 |
| 完整供给预算 | 预期fps取 `min(同窗接收, 同窗解码, rendererTargetFps)`，覆盖阈值98%；候选不足归类 `frameDeliveryLimited`，候选→提交损失归类 `renderStarvation`。 |
| 有限保真回退 | 接收/解码/video/候选/提交均有完整3秒窗口，再持续3秒确认本地欠帧且video推进充分，才执行既有video回退。CPU平均成本作为分项证据。 |
| 效果与试探 | 回退效果以完整源预算验收；video推进充分时允许回调稀疏。健康维持15秒后仅一次增强试探，55/60fps试探失败立即回退，后续保持video。用户变更显示意图时重新建立策略上下文。 |
| 实时提交年龄 | snapshot读取当前同代际renderer的 `lastDrawAtMs`，补 `rendererSubmitAgeObservedAtMs`；FPS、sampleId和3秒/15秒窗口继续共享采样事实。代际变化时实时年龄缺证。 |
| 候选首帧期限 | 实际源候选到达后启动5秒期限，隐藏时暂停；硬错误沿PlaybackService即时处理。失败资源事件记录renderer代际、kind和原始reason。 |
| 成本证据 | 普通WebGL/SR热路径累计上传和命令提交CPU总耗时、最大值及样本数；同代际完整计数差形成均值，接入统计窗口、trace和诊断面板。截图采用独立记账。 |

回退过程保留接收帧率、媒体传输与用户超分意图。实际增强处于回退时，面板通过 active状态与准确fallback reason表达当前运行管线。

## 生产策略回放

[回放脚本](2026-10-30-display-deficit-policy-replay.cjs)直接转译生产策略模块。基线仅替换 `browser-render-health.ts` 为本轮实施前的暂存版本，其余依赖采用当前源码；模块SHA256与日志SHA256均保存在[统计结果](2026-10-30-display-deficit-fix-validation.metrics.json)。历史重跑需要保留该暂存基线。

35份SR监督采样中，32份具备完整3秒供给覆盖：

| 结果 | 旧策略 | 修复后策略 |
| --- | ---: | ---: |
| renderStable | 29 | 0 |
| frameDeliveryLimited | 3 | 32 |
| 首次回退 | 无动作 | seq1638，sampledAt112428ms / observedAt112643ms |
| 首次动作证据 | — | coverage3000ms，持续3058ms |

原始35份telemetry的31次renderStable与此处完整窗口的29次采用不同覆盖分母。脚本按共享sampleId、采样时间与renderer代际读取监督窗口。

动作后效果使用受控供给单列验证：6000ms持续60→44fps触发回退；9000ms同代际完整窗口验证video输出60fps、回调52fps；24000ms完成15秒健康证据后试探；27000ms增强仍55fps，触发`enhancementTrialFailed`；30000ms重新确认video60fps，后续至60000ms保持回退。30fps源和用户30fps预算两组均保持健康。

日志回放证明归因与首次动作；受控供给证明效果准入、试探和回滚。回放中的video60fps表达受控输入，设备效果继续由新日志确认。

## 自动验证

- 全量 `vitest run`：47文件、399项通过；覆盖普通WebGL/SR的50.4/60与40/60提交缺口、60→44候选缺口、30fps源与限帧、缺证、重复采样、前后台隔离、有界试探与实时年龄。初始新增回归在旧实现得到7失败/52通过。
- `pnpm lint:fix`、`pnpm exec vue-tsc --noEmit`、`pnpm build`、`git diff --check`全部通过。本轮实施为前端显示与统计路径，既有暂存Rust改动由其原任务保留。
- 原供给回放9组通过：有完整身份的30fps供给持续300次候选/300次处理；身份实际合批的4fps/3.7fps案例保留原语义。
- 48帧偏移回放3组均保持稳态30fps。普通WebGL/SR冗余操作回放4组保持61次实时处理/61次GPU draw、0次FBO分配、0次重复处理。
- 等尺寸RCAS、单pass放大、EASU+RCAS与截图并发记账继续通过；新增分段计时在受控8ms上传/3ms提交下准确记录11ms处理总耗时，完整统计差投影为1.5ms上传/0.5ms提交。

复算策略：

```sh
node dev-docs/validation/2026-10-30-display-deficit-policy-replay.cjs <trace.jsonl> <output.json>
```

## 设备验收

自动实机运行继续暂停。按用户既定Mac+主机手测方式，用新构建保持相同主机、1440p源、60fps预算和持续前台，分别记录关闭SR的中性auto与开启SR各至少30秒。核对实际renderer代际/kind、完整3秒/15秒接收→解码→video推进→候选→提交窗口、供给delta、回退动作与试探结果、上传/提交CPU成本、freeze和丢包。

当前`presentFps`在canvas路径来自实际处理计数，video路径来自播放推进计数；GPU执行时间、物理scanout、submit-to-present与glass-to-glass的逐帧P95/P99继续unavailable。设备验收须同时观察画面流畅度、超分实际active状态和同窗供给，ROG1440p与2h/8h稳定性继续归属S4后续验证。
