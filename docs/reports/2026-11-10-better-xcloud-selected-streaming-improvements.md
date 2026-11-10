# better-xcloud 四项串流改进实施报告

## Summary

- Related RFC: [四项改进计划](../rfcs/2026-11-10-better-xcloud-selected-streaming-improvements.md)。
- 2026-11-10，主窗口直接完成 main Browser 主线的四项改进，公共协议与领域逻辑继续由 Rust 提供，设置、Browser 输入与 WebGL2 沿现有 Vue/TypeScript 装配。
- 代码、自动回归、真实浏览器像素与设置交互验收完成。设备功耗、物理上屏、实际串流降档与网络路径收益接续用户实机验收。

## Delivered

| 改进 | 交付行为 |
| --- | --- |
| 防止分辨率下降 | 新配置及旧配置缺字段均默认关闭，显式偏好持久化，下一连接生效。开启时对上报副本应用 10ms 反馈代理限幅，原始观测继续用于统计。诊断面板显示关闭/生效/等待元数据/元数据不可用。 |
| 局部 GPU 优化 | sampler 显式 highp，gl_VertexID 全屏三角形，按实际处理尺寸缓存 texel uniform，program 链接完成即释放临时 shader；减少 vertex buffer 与 attribute 状态。 |
| 动态服务器区域 | 常规入口为“云服务器区域”，包含服务推荐自动项及当前有效区域。高级设置默认折叠，内含“云服务登录兼容”，独立“登录地区”弹层解释授权用途。保存、加载、失效选择、失败与重试状态清楚展示。 |
| 主机串流补齐 | 默认区域空/失败时跨区域发现，目标查找继续到命中；发现区域贯穿控制面。读取当前 session configuration，绑定当前远端传输补 ICE，晚到配置与 EOC 统一完成，重启按新代次重新绑定。 |

## Changes

### 配置、区域与主机发现

- 配置新增 `prevent_resolution_drops=false`、`xcloud_region_name=""`，以及内部一次性迁移标记 `xcloud_region_migrated`。配置更新与迁移共用 mutation 锁；用户明确选择自动后保持服务默认语义。
- 旧 `force_region_ip` 可以在首次有效列表加载时迁移成匹配的服务器名称，原授权偏好保留。常规服务器偏好和高级登录地区各有名称、说明与焦点作用域。
- 公共 [区域 compiler](../../crates/xbox-streaming/src/policy/session/compiler.rs) 优先消费当前有效显式区域，随后为服务默认和首个有效区域。区域名称大小写匹配；[服务区域解析](../../crates/xbox-streaming/src/session/access.rs) 过滤空名与非法 origin，并按稳定名称去重。
- `getStreamingRegions` 使用 Cloud/Home 各自授权 scope，前端 DTO 仅包含名称、显示字段、默认标记、更新时间和偏好。token、baseUri 与控制面选择留在 Rust。
- [跨区发现](../../crates/xbox-streaming/src/session/discovery.rs) 每区 5s、整体 30s、撤销检查 50ms。目标缓存区域优先，其后默认与其他区域；401/403 交回上层认证/资格处理，可恢复失败继续，全空与全失败分别表达。
- [查询服务](../../src-tauri/src/mods/data/services/streaming_query_service.rs) 维护账户、token 内容/代次、授权路由 scope 的 5min 内存缓存。结果绑定 `discoveryRegionName`；旧账户、token 或路由响应在提交前拒绝。
- [主机上下文](../../src-tauri/src/mods/streaming/context.rs) 使用 Rust 发现结果选择本次区域；发现失败传播至现有错误处理。完整 HTTP 回放确认创建、状态、configuration、SDP/ICE、保活与关闭使用同一 origin。

### 当前 configuration 与 ICE

- [地址解析与候选策略](../../crates/xbox-streaming/src/policy/negotiation/console.rs) 显式解析 IPv4/IPv6、声明端口及 9002 兼容端口，统计非法地址/端口，去重后按 UDP host 候选生成稳定 foundation 与 priority。
- [信令状态](../../crates/xbox-streaming/src/session/signaling/state.rs) 保存本次 configuration、独立配置代次、当前远端 BUNDLE/媒体传输与已补候选。当前配置优先，列表 fallback 在配置完成或放弃后补充。
- [信令编排](../../crates/xbox-streaming/src/session/signaling/flow.rs) 在会话撤销和 ICE 代次核对后一次性提交响应。配置等待预算 10s，每次查询 1500ms；204 继续等待，404/405/501 明确走兼容 fallback。诊断原因包含 pending、resolved、unsupported、request-failed、deadline。
- 空远端 ICE 也可提交配置补候选；远端 EOC 等待配置完成或放弃后最后交付。独立传输保留各自结束标记，旧 ufrag 的结束标记拒绝。同 Peer 画质协商保持幂等，ICE restart/Peer 重建清理旧记录并重新绑定。
- 候选来源贯通 Rust、Tauri 和 Browser DTO：`session-configuration`、`console-list`、各自 `:compatibility-9002`、`teredo-derived`。来源字段用于客户端观测，HTTP 信令保持既有候选协议字段。
- Browser 即便已经 connected，也在 `configurationPending` 期间继续现有 ICE 轮询，晚到配置沿既有 `addIceCandidate` 消费。

### 分辨率反馈与 GPU

- [反馈规范化](../../src/player/app/input/frame-feedback.ts) 在 uint32 编码前处理有限值、非负单调时间与时间差；开启时只对 wire 副本限幅，关闭时发送规范化的完整差值。[InputService](../../src/player/app/input/InputService.ts) 每帧发布一次原始统计事件。
- rVFC 的 receive、expectedDisplay、callback、processingDuration 保留来源。expectedDisplayTime 的反馈解码字段明确标记 `expected-presentation-proxy`；实际 processingDuration 独立统计。缺少 RTP/receive 元数据时保留观察并显示策略不可用。
- `resolutionFeedbackObserved` 使用 1Hz 有界摘要，记录模式、有效状态、原因、调整量、原始反馈差值和 processingDuration 样本。新事件同步纳入生产 trace 白名单与大小/采样预算。
- [WebGL2 renderer](../../src/player/infra/render/Renderers.ts) 和 [FSR pass](../../src/player/infra/render/fsr1-reconstruction-pass.ts) 使用相同高精度、全屏三角形与临时 shader 生命周期；USM/CAS texel uniform 按处理输入尺寸更新。
- 上传 Y 翻转、当前亮度权重、CAS/USM 数学保护与 pass 数延续当前图像基线。坐标迁移、色彩权重变化继续按目标设备图像/成本证据评估。

## Validation

| 验证 | 结果 |
| --- | --- |
| `cargo fmt --all` | 通过 |
| `cargo test -p xbox-streaming --quiet` | 162 项通过 |
| `cargo test -p xbxrc --lib --quiet` | 142 项通过 |
| `pnpm lint:fix` | 通过 |
| `pnpm typecheck` | 通过 |
| `pnpm build` | 通过，386 个模块 |
| `./node_modules/.bin/vitest run --reporter=dot` | 98 个文件、890 项通过 |
| `pnpm test:trace-production` | 2 项合同检查通过 |
| `git diff --check` | 通过 |
| WebKit WebGL2 RGBA8 像素读回 | 120 组通过，最大码值误差 0 |
| 真实设置页面交互 | 中英、深浅、1280×720 与 720×900、保存/失效/失败重试、高级折叠、选择器焦点与关闭恢复通过 |

重点回放覆盖：默认迁移与显式自动、区域有效性与大小写、目标位于后续区域、超时/撤销/资格失败、完整控制面 origin、204→有效配置、空 ICE/仅 EOC/晚到配置、配置期限终止原因、旧响应与代次、独立传输结束标记、原始/wire 报文、阈值/非法时间/时钟边界，以及 GPU 初始化/释放与尺寸 uniform 更新。

像素对照用旧 shader 基线和当前生产 shader，实际编译、链接后释放临时 shader 再绘制。覆盖黑、近黑、方向、文字、彩线、噪声；1:1、3 倍放大、缩小；USM、CAS、EASU+锐化。[记录](../../dev-docs/validation/2026-11-10-better-xcloud-improvements/pixels.metrics.json)与[回放入口](../../dev-docs/validation/2026-11-10-better-xcloud-improvements/run-pixels.mjs)已保存。`--serve` 在 `http://127.0.0.1:1430` 提供真实浏览器 fixture；本机无头 Chromium 输出采集未成功，本次像素证据来自 Codex in-app WebKit。

设置验收使用真实 Setting.vue 与现有设置壳层，fixture 替换 RPC/事件以重现服务响应。检查了默认关闭、动态区域保存、保存项消失、加载失败保留偏好、重新打开重试、高级授权说明与独立焦点作用域。[窄窗口登录兼容截图](../../dev-docs/validation/2026-11-10-better-xcloud-improvements/login-compatibility-narrow.jpg)已保存。

## Risks

- 分辨率保持改变服务端收到的客户端压力反馈。源尺寸是否更稳定、延迟/冻结是否增加，需要同网络、同请求档位实机 A/B；网络拥塞继续由 WebRTC 反馈处理。
- 当前 configuration 的返回时间、地址与声明/9002 端口可达性取决于实际服务和网络。候选注入与代次已回放验证，选中路径及连通收益需要 LAN/NAT/异地设备证据。
- 像素一致证明采样与输出等价；目标 GPU 完成时间、提交/呈现长尾、功耗与物理上屏由设备测量确认。

## Follow-up

| 实机验收 | 同窗记录 |
| --- | --- |
| 分辨率保持关闭/开启 A/B | 实际源尺寸、降档、RTT/jitter/loss、原始代理差值、processingDuration、调整量、首帧与冻结/恢复 |
| LAN、家庭 NAT、异地、IPv4/IPv6、Teredo、TURN | 发现区域与实际控制面区域、候选来源与选中候选对、连接/首帧耗时、失败原因 |
| macOS WKWebView、Windows WebView2 | 原画/USM/CAS/SR组合、窗口尺寸与context恢复、截图方向、资源释放 |
| GPU 与整体供帧 | 网络健康、恢复链健康、media supply、steady supply、read-to-submit和submit/present P95/P99、接收/解码/reader/提交/呈现帧差分、设备功耗 |

实机记录沿现有 runtime trace 导出与分析工作流，保留同一会话/媒体/渲染代次和稳定观察窗口；能力缺项记为 unknown。
