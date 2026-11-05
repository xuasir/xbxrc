# Browser 最新帧渲染调度

主机与云游戏的 Browser 渲染调度已统一为有效新候选即时提交、超额候选直接跳过、后续最新图像覆盖旧图像。两种增强 renderer 的累计deadline等待、旧候选rAF重试、fresh等待状态及GPU补供帧准备链已收掉。显式帧率预算、零预算暂停、截图和资源生命周期保持有效。

关联[RFC](../rfcs/2026-11-05-browser-latest-frame-scheduling.md)与[复算指标](../../dev-docs/validation/2026-11-05-browser-latest-frame-scheduling.metrics.json)。主窗口在main工作区直接执行，既有渲染/输入/trace修复继续保留。

## 实现结果

- `FramePacer`按当前最邻近预算槽即时准入，每槽最多一次，半槽相位余量吸收正常回调抖动，1ms余量对齐时钟取整与半槽浮点边界；迟到跳到当前槽。被跳过的候选由后续最新帧接替，renderer持有的定时等待队列为0。
- 普通锐化与SR收到有效候选立即尝试导入/绘制；初始化已有最新源图像时直接提交。每个video共用一个RVFC和一个计数轮询，原始metadata、去重抵扣及取消epoch沿同一事实源。
- GPU像素只做dev惰性低频诊断，每约500–602ms尝试一个8–50ms配对，最多保留8份差异摘要；先交付候选再导入诊断像素。off阶段探测context/导入/定时读取为0，诊断变化对候选资格的影响为0。
- 新字段`rendererRateLimitDropCount`贯穿renderer→StatsService→runtime trace/诊断/面板。历史局部等待槽的`latestOverwriteCount`兼容值为0；回调合批推进继续由帧源计数独立解释。截图单独计数，GPU错误仍走既有回退。
- 当前main的Browser `PlaybackService`按同一renderer工厂装配主机和云游戏。网络接收、解码、恢复及native WIP沿各自现有运行时边界。

## 自动验证

60文件591项全量测试通过；本轮渲染相关6文件153项通过，包含两个生产renderer、真实共享帧源、诊断、截图及统计面板。旧逐帧像素补供给用例按历史合同保留在日期化报告；当前测试改为诊断与交付独立，并保留冷/稳态CPU预算、异步GPU等待、尺寸/身份/可见性变化、context丢失、迟到回调与关闭清零回归。

| 场景 | 提交与验证结果 |
| --- | --- |
| 60fps候选，毫秒时钟与±3ms相位抖动 | 两种renderer各600个候选立即提交600次，运行期renderer rAF等待为0 |
| 同槽突发序号1/2/3，随后2秒停顿 | 先提交1，跳过2/3；rAF推进保持提交数不变；新序号4到达立即提交4 |
| 60fps源，30fps显式预算 | 每预算槽最多一次，10秒回放301次，窗口边界量化差1帧；稳态间隔32–34ms，暂停/恢复通过 |
| 90/144/165Hz高速候选，60fps预算 | 10秒回放分别600/601/601次，窗口边界量化差1帧，迟到时直接取当前槽 |
| 稀疏RVFC，静态/运动像素，诊断off/on | 两种renderer的导入图像序列逐项相同，诊断读取每10秒≤40次，像素补交付0 |
| 初始化/暂停/截图/context/关闭 | 单链初始化、旧回调忽略、截图计数独立、最后消费者关闭后GPU/回调/定时器归零 |

`pnpm lint:fix`、`pnpm typecheck`、`pnpm build`（313模块）与`git diff --check`通过。独立回放加载生产FramePacer并校验9组节拍/突发/暂停结果，metrics同时绑定11个生产文件SHA与全量测试报告SHA。

复算：

```sh
./node_modules/.bin/vitest run --reporter=json --outputFile=/tmp/xbxrc-latest-frame-tests.json
node dev-docs/validation/2026-11-05-browser-latest-frame-scheduling-audit.cjs /tmp/xbxrc-latest-frame-tests.json dev-docs/validation/2026-11-05-browser-latest-frame-scheduling.metrics.json
```

## 设备证据与边界

此次自动验证确认即时准入、直接跳帧和诊断独立。既有设备日志属于修改前样本：健康SR约60fps解码、47.79fps候选、47.46fps提交，候选交付缺口仍需新构建的主机/云游戏持续运动场景验收。GPU实际执行、WebView可绘制图像更新、物理submit-to-present及glass-to-glass由设备测量确认。

后续分别固定video、普通锐化、SR各60秒，核验同窗received→decode→video→candidate→submit、限帧跳过、提交间隔P95/P99及帧龄；网络freeze与恢复段单列。自动回归支持两种会话使用同一策略，整机帧率与长测继续进入Browser stable/S4验收。

2026-11-05 14:42新构建设备日志已完成[独立分析](../../dev-docs/validation/2026-11-05-latest-frame-scheduling-fresh-trace-analysis.md)。源码指纹匹配，首段SR累计352候选中84个被固定时隙限帧跳过，23.86%；即时供给53.14→提交40.19fps，video稳态约60fps。生产回放复现相位冲突，原自动回放的稳定相位覆盖存在盲区；严格Browser运行态门禁PASS，真实增强供给仍待修复。优先让常规源帧率下的新候选直接放行，显式限帧沿独立轻量限流验证；候选源缺口与GPU/物理present继续单独取证。

用户随后授权继续优化，[默认准入修复](2026-11-05-browser-frame-admission-fix.md)已完成：常规60fps及以上候选直通，显式低FPS使用滚动准入；SR context恢复沿新候选订阅去重。60文件610项测试、12组节拍与19组设备入口回放通过，初版固定槽的日期化自动报告与采样源码身份沿原证据保留。帧率收益和video→candidate剩余缺口接续新设备样本。
