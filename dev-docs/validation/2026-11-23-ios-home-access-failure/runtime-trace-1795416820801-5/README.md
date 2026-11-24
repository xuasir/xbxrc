# iOS 云连接间歇性授权超时分析

## 范围

输入 `/Users/guo.xu/Downloads/runtime-trace-ios-1795416820801-5.jsonl`；90行、47,468字节、1个session、production，seq 41307–41396，上海时间14:53:40.800–15:05:34.096。原始字节与SHA-256已归档，用户原件保留。

`fileOpened.reason=budgetRotate`，`exportRequested.allFiles=false`。该文件为第5个轮转分片，包含4个云连接失败样本；成功尝试需要其他分片形成对照。上轮用户背景为Wi-Fi裸连、主机待机、唤醒开启、PC能连接，本分片的实际启动目标为Cloud。

## Gate

分片结构gate PASS：schema、sequence、privacy、budget和条件配对违规均为0。要求完整启动与目录流程时gate FAIL，5项缺失：4项先前启动/页面锚点需其他分片，缓存恢复结果另有记录缺口。串流成功合同FAIL，28项缺失；4次授权前失败的成功锚点与清理计数按实际资源创建解释。

## 时间线

| generation | 授权开始（上海） | UniFFI调用耗时 | seq |
| --- | --- | ---: | --- |
| 11 | 14:53:40 | 13.655 s | 41308 → 41313 |
| 12 | 14:53:56 | 7.498 s | 41322 → 41327 |
| 13 | 14:54:05 | 14.032 s | 41336 → 41339 |
| 15 | 14:54:26 | 6.342 s | 41364 → 41366 |

4次均为`RetryableAuthentication("Network error: request timed out")`，`timeout=true`、`retriable=true`，随后`accessPrepareFailed → terminalSelected(runtime.failed) → failed`。失败状态target=cloud，授权准备后续会话阶段待成功尝试形成锚点。外层授权边界与UniFFI边界耗时差为2–38ms，耗时集中在该调用范围内。

## 覆盖、预算与隐私

连续90行完整解析；essential17行、key73行，序列违规和隐私违规均为0。文件47,468字节符合8MiB预算，writer声明最多4文件；Downloads可见1份raw输入，App实际保留集合需全部导出核验。压力通知与fileOpened累计丢失计数均为0。

网络维度有4次认证请求超时，另有26次元数据页提交，页处理耗时1.515–11.804秒。恢复链、media supply、steady supply、submit/present长尾与帧供给delta需要成功串流的媒体/渲染分片，当前4次启动停在授权准备。

## 原因与机制

当前源码中，认证HTTP客户端使用`for_commands`：建连预算5秒，单次请求预算12秒，单调用有限总预算18秒，网络重试模式为`Never`。Trace的elapsedMs涵盖完整授权链；前面已完成步骤的耗时加上某个请求触发的预算，能够形成6–14秒的不同失败时长。每次失败具体命中了哪个请求和预算，需子阶段事实确认。

iOS每次`prepare_cloud_access`调用完整`refresh_and_finalize`，选择All范围，依次涉及OAuth续期、设备令牌、SISU、Web XSTS、Streaming XSTS、Home凭据，再获取Cloud凭据。必需步骤失败会结束这一轮；Home凭据是当前Cloud启动的一个必需请求点。PC支持有效凭据快照与SISU复用，并按目标范围续期，减少重新建连和认证请求。

按当前源码，OAuth续期失败会携带`OAuth refresh grant failed:`前缀。这4个错误保留普通Upstream网络超时格式，可进一步指向OAuth续期之后的设备、SISU、Web/Streaming XSTS或Home请求。Cloud offering回退失败走固定Cloud缺失说明；当前4个错误来自此前必需请求链。具体认证端点与DNS、连接、TLS、读取阶段仍待细分诊断，源码对照的身份由hash保留。

概率性的解释是：响应在各自固定预算内完成时流程前进；任一必需步骤超时会终止该次连接。重复执行多步完整认证增加遇到暂时响应慢或路径异常的机会。本分片的4个失败样本支持阶段判断，整体成功率需同一目标的成功分片。

## 后台耗时与下一步

14:54:51.956进入后台，15:05:28.122恢复；元数据页失败的636.453秒壁钟耗时覆盖该后台区间，timeout字段为false。4次访问授权超时均在进入后台之前发生，两组耗时按各自阶段解释。

修复优先级为对齐PC的有效凭据复用、按目标范围续期，并补充下游认证步骤的脱敏类别与计时；沿用公共Rust认证与作用域合同。设置中的“导出全部 Trace”用于取得同一会话成功/失败对照，当前导出为“导出当前 Trace”。

## 归档与清理

`analysis.json`、`attempts.json`、`evidence-anchors.json`、三个gate报告、源码摘录/hash和原始raw副本构成本轮证据。业务源码编辑、构建测试进程与临时产物均为0；用户原始日志与此前工作区改动保留。
