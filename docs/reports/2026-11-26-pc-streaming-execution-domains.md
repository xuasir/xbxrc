# PC 串流执行域隔离工程 Report

## Summary

- [RFC](../rfcs/2026-11-26-pc-streaming-execution-domains.md) 的 M0–M4 工程已完成；用户授权复用最近基线，最后统一实机回归一次。
- Browser 串流复用共享 WebGL 核心，通过实际宿主能力探测选择 Reader→Render 直接帧链；输入通道具备真实转移资格时装配同一个 InputCore 到独立 Worker。
- 自动验证与收口证据见[证据目录](../../dev-docs/validation/2026-11-26-pc-streaming-execution-domains/README.md)。实机收益、宿主资格和真实远端短按识别由最终回归确认。

## Delivered

| 阶段 | 完成内容 |
| --- | --- |
| M0 | 复用最近 1440p60 / USM3 基线；启动时探测离屏 context、当前 shader、VideoFrame 导入及像素截图，独立探测输入 Worker、时钟及实际 channel 转移 |
| M1 | 提取 DOM 无关 WebGL 核心；Main 适配器和 Worker 复用 shader、uniform/FBO/SR、限帧及同帧纹理缓存 |
| M2 | Reader 持续读取、latest/token 有界，MessagePort 直接交接原生帧；配置合并、暂停/重绑、截图、上下文故障、关闭和新 canvas Main 回退 |
| M3 | 100ms 数字事实、1Hz 分位数聚合、单个在途快照、执行域/代次/时钟/交付年龄、同窗 read/delivered/overwrites/stale-drop delta，接续当前 Rust renderer 局部恢复 |
| M4 | 同一个 InputCore，4ms deadline 与至少4ms状态间隔、数字/扳机/归零保序、轴合并、owner barrier、同步实际 send 接受出队、8ms反馈与原客户端 wire 时钟 |

## Changes

- Main 保留 Vue/DOM、Gamepad 采集、几何导航、原生 rVFC/音频、peer 和控制通道。Reader 与 Render 维持一次 VideoFrame transfer 和一次 CPU 提交/丢弃 ACK；各自继续独立推进。
- 新帧沿 `texImage2D(VideoFrame)` 更新纹理。同帧截图复用纹理；尺寸/context 变化使缓存失效。Main 与 Worker 使用同一实现。
- 单 link 的应用帧上界为3，现行 current/pending 交接的短暂总上界为6。显式截图至多4项等待，单批一次同步 draw/readPixels、一次 ImageBitmap 转移；能力不足时使用 RGBA 转移。
- Worker 硬故障由 PlaybackService 新建 Main renderer generation，保留 SR、锐化和颜色意图。局部恢复继续复用现有动作所有者、预算与效果观察。
- Window 撤销已转移 input channel 的对象归属。停止时先失效旧事件，等待有界 neutral 接受确认，再关闭捕获的旧 peer；新 peer 可以独立绑定。
- Rust 生产事件准入从共享 JSON 装配，执行域和既有局部停滞/音频事件沿同一描述表与隐私投影。

## Baseline

最近基线为[日志 1795692428673](../../dev-docs/validation/2026-11-26-pc-latest-log-1795692428673/README.md)：macOS Home、1440p60、锐化3、SR关闭；最后输出2994×1684。11249次完整上传、0次子上传，平均0.439ms；最后保留窗上传/绘制P95/P99均1ms，read→submit P95 2ms、P99 3ms。

该日志的前台网络中位 RTT 4ms、jitter 5ms、视频缓冲5.76ms；原始计数 received 12764、decoded 12763、renderer submit 11340。隐藏窗口贡献1053个decode和0个submit，中段还存在供给停顿；Browser恢复逐请求效果及物理上屏保留对应能力/证据资格。音频沿Opus双声道与现有补偿链执行。

## Validation

- 266项前端定向检查、2项生产日志预算及4项Rust合同检查通过；lint、typecheck、三个Worker的Vite打包与cargo fmt通过。摘要、命令结果与失败修复记录归档于证据目录。
- 主线程渲染、直接帧所有权、输入背压/短按、配置与代次、截图批次、同窗观察、neutral/peer关闭及JS/Rust日志合同采用定向冒烟。
- 前端执行 `pnpm lint:fix`、类型检查和 Vite Worker 打包；Rust日志描述表执行 `cargo fmt --package xbxrc` 与定向合同验证。
- 生产日志保留既有行宽、在途队列及327680 B/min常态预算；健康桶压缩重复累计投影，完整同窗delta及session summary/decodedSupply累计事实保留。执行域字段进入同一投影。

## 实机回归

沿最近基线使用1440p60、锐化3、SR关闭进行一次约3–5分钟的回归，在同一轮内覆盖稳定移动画面、短按/扳机/摇杆归零、打开菜单、一次截图、一次前后台；再切一次实际低源分辨率SR并截图。实际input channel支持转移的宿主会记录`input-worker`，其他宿主沿Main InputCore发送。

回归日志核对执行模式和能力原因、upload/read→submit P95/P99、转移/ACK及快照成本、3s/15s供给、received/decoded/read/delivered/submitted/local-drop delta、网络与恢复健康、GPU代理长尾和输入采样→发送。物理scanout与远端游戏识别沿实机观察确认。

## 首轮实机日志结果

2026-11-26 21:53–21:55的[最新日志证据](../../dev-docs/validation/2026-11-26-pc-latest-log-1795701183517/README.md)确认视频worker-direct与输入input-worker启用。同档前台64.044秒，接收/解码59.896、提交59.881FPS，submitted/decoded99.974%；上传/绘制P95/P99均1ms。上传累计差均值由基线0.471ms到0.606ms，read→submit窗口P95/P99中位数由2/4ms到2/3ms。单向帧交接最后保留窗P951.5ms、ACK往返P953ms；输入交接max2ms、积压/溢出0，退出neutralAccepted=true。

有效production连续段的网络、Browser公开统计恢复、media supply、steady supply与帧供给delta五项PASS；整段submit/present尾部FAIL。两次媒体供给扰动对应视频包约1秒停止推进及440个视频丢包，实际freeze四次共1.941秒；另一次samplingUnavailable与新getStats交错、约1ms后新样本完成。首次扰动后音频缓冲升到280ms，约32秒后回到80ms以内。

GPU完成upper-bound代理P95/P99由6/8ms到9/12ms，主要差异为fence插入时点中位数晚3ms；插入后完成观察的P95/P99双方均6/8ms。Main资源归零与409ms清理/flush通过；Render独立停机回执因destroyClient先解绑观察订阅而缺证。末窗read→submit P995.5ms含最后供给异常，按校准uncertainty2.5ms及同窗事实继续审查成本门。

本轮实机验证证明能力装配与稳态吞吐，完整成本、受控繁忙收益和SR/截图/前后台等功能资格继续为Validation Pending。优先补齐采样、fence、心跳年龄及停机观察，再处理供给停顿后的音频缓冲回落。分析证据已归档；归档时原应用日志目录已不存在，完整冻结压缩副本保留。

## 日志分析清理

本轮专属日志冻结、报告输出和一次性分析脚本按manifest记录清理，原始冻结gzip及最终证据留在最新日志证据目录，清理和单次冒烟结果见checks.json。

## 工程清理

本轮备份、一次性脚本、测试日志和Vite输出按专属临时清单清理；保留最终证据、最近基线原件以及现有共享构建缓存。专属临时目录已清理，共8.28MiB；本轮构建/测试会话已结束，目录存在性与磁盘空间已核验，最终结果记入证据摘要。
