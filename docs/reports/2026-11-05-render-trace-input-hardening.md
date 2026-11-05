# 渲染、日志与输入七项问题修复

2026-11-05。七项代码修复及自动验证完成：前端59文件609项测试、Rust58项测试、类型检查、生产构建与56组生产模块供帧回放全部通过。执行基于main当前工作区，基础提交为`82e1fed2c570ac7eab07270147ce7be3e1572be0`，主窗口直接完成。

关联[RFC](../rfcs/2026-11-05-render-trace-input-hardening.md)。上一轮[渲染与手柄输入延迟优化](2026-11-05-render-input-latency-optimization.md)继续作为基线，其指标与源码指纹保存原采样身份；本轮结果独立保存到[验证证据](../../dev-docs/validation/2026-11-05-render-trace-input-hardening.metrics.json)。

## 七项交付

| 问题 | 完成行为 | 回归证据 |
| --- | --- | --- |
| 普通/SR WebGL context残留 | 终态销毁先停止回调、解绑事件、清理GPU对象和timer，再主动loseContext，canvas缩为1×1 | 两种renderer均验证释放一次、重复销毁安全、扩展异常隔离；调用loseContext时回调、截图等待和GPU对象已清零 |
| trace队列缺少硬预算 | 有界通道及8192行/8MiB账本，单行64KiB，按等级预留余量，try_send入队 | 停滞writer、并发producer、行数/字节上限、在途reservation释放及超大行回归通过 |
| 文件生命周期占用发布线程 | 现有writer独占文件创建、轮转、裁剪、flush与序号分配，配置和正文按代次FIFO处理 | 并发100行全部排空；单文件4096B预算、轮转序号、writerThread、配置失败/发布态回滚及裁剪保护通过 |
| 输入保留字段字节序 | 多字节保留字段统一little-endian | 输入报文回归通过，零值字段保持既有字节结果 |
| Pointer多帧/计数错位 | 预算前过滤有效Element target，显式读取DOM事件字段形成快照，编码全部有效帧与事件 | 两帧编码、原事件后续变化、无效target过滤、后续Mouse/Keyboard偏移和uint8溢出拒绝通过 |
| renderer初始化与回调竞态 | 重复init复用promise，初始化期间更新配置，GL准备完成后启动单链；迟到回调检查请求归属 | 两种renderer验证初始化中0→60fps及颜色更新、暂停恢复后的旧回调隔离、准备取消与销毁清零 |
| Tauri setup失败引发panic | 服务初始化错误先反馈，再请求exit(1)，setup正常返回；macOS关闭窗口及退出清理使用try_state | 错误反馈→exit(1)顺序和成功路径通过；cargo check覆盖macOS窗口保护 |

## 日志预算与诊断口径

队列账本同时保留行数与`Vec.capacity()`，覆盖队列及writer正在处理的正文缓冲。编码器限制增长容量并提前拒绝超大行；64KiB行预算为序号前缀和换行预留空间。通道额外保留16个控制槽，writer使用64KiB固定缓冲、256项批处理及40ms flush。

| 新行等级 | 接受该行时的总pending行数上限 | 总pending正文缓冲容量上限 |
| --- | ---: | ---: |
| Debug / Raw | 6144 | 6MiB |
| Key | 7936 | 7.75MiB |
| Essential | 8192 | 8MiB |

各等级共用同一份账本。显式配置请求等待writer回执，等待期间释放发布锁；配置失败恢复发布侧profile、dimensions与generation，writer继续持有原output。日常事件进行有界JSONL编码与try_send。文件预算包括fileOpened和正文，写入前先计入整行预算，部分写入失败也保留保守的文件容量记账。

trace继续沿schema v3，`fileOpened.payload`增加writerThread与队列/单行预算。writer在实际写入时分配seq，跨轮转保持单调。`traceBudgetNotice`保留reason=`writerQueuePressure`，同时提供essential/key/debug/rawDropped，以及oversized/configuration/ioDropped和pending/peak/max rows、bytes。

丢弃等级与原因在同一事务累计，避免并发提取造成证据拆散。notice由writer直接提交，周期60秒，Sync、配置与Shutdown强制提交；提交失败恢复计数供下次尝试。分析新trace时先核对全部等级，Key/Essential计数大于0的窗口标记为证据受损，oversized/configuration/io计数用于解释丢弃来源。

## 验证结果

| 检查 | 结果 |
| --- | --- |
| `pnpm lint:fix` | PASS |
| `pnpm test` | 59文件609项PASS |
| `pnpm typecheck` | PASS |
| `pnpm build` | PASS，313模块 |
| `cargo fmt -p xbxrc` | PASS |
| `cargo test -p xbxrc --lib` | 58项PASS |
| `cargo check -p xbxrc` | PASS，输出无编译警告 |
| 既有生产供帧回放 | 56组PASS |
| recorder生成样本审计 | 9文件18行、6个traceRun、1条budget notice；schema v3字段完整、文件内序号递增、run内序号唯一 |
| `git diff --check` | PASS |

供帧回放复用生产帧源与renderer模块，视频、GPU和时钟使用受控替身。持续10秒、60fps源、RVFC45/50Hz的冷准备场景，两种canvas renderer仍提交600张唯一图像，GPU补交付150/100帧；所有结束资源检查通过。Rust writer内部简化JSON单测用于队列和文件行为；schema审计使用完整recorder生成样本。

复算供帧：

```sh
node dev-docs/validation/2026-11-03-sr-cold-preparation-audit.cjs /tmp/xbxrc-2026-11-05-hardening-supply-replay.json
```

## 实机验收

代码和自动回归已完成。实际驱动释放、Tauri事件循环退出码、浏览器GL呈现和输入到游戏画面响应接续Browser stable / S4实机验收，由用户执行启动、串流、Windows/ROG及30分钟长测。重点覆盖SR开关、30/60fps切换、前后台、窗口关闭、断开重连与快速按下/释放。

新设备trace继续同时核对网络RTT/jitter/loss、Browser恢复链、media supply、steady supply、submit/present长尾与decode→candidate→submit帧供给delta，并核对各等级trace丢弃计数。实机结论沿设备证据记录，自动样本用于验证调度、资源、预算及编码合同。

本轮修改保留在当前工作区。分支装配、技术栈、媒体/输入归属、几何导航与发行入口沿既有仓库约定执行。
