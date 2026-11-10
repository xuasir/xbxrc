# better-xcloud 四项串流改进计划 RFC

## Status

- Completion: 代码实施、自动回归与浏览器像素/设置验收完成；设备串流收益接续用户实机验收。
- Current State: completed
- Owner: 主窗口直接执行，单窗口协调。
- Last Updated: 2026-11-10
- 依据：[六方向源码审计](../reports/2026-11-10-better-xcloud-streaming-reference-audit.md)。
- 实施结果：[实施报告](../reports/2026-11-10-better-xcloud-selected-streaming-improvements.md)。
- 用户确定的范围：防止分辨率下降体验开关、局部 GPU 优化、服务区域动态选择、xHome 跨区域发现与主机地址补候选。

## Background

better-xcloud 的相关能力有现成实现线索，xbxrc 已具备设备画像、独立 VideoFrame 供给、公共输入、Rust 会话与信令基础。本次沿这些模块补齐功能与消费链，分批核对外部行为和成本。

进一步定位得到三个关键事实：

1. [InputService](../../src/player/app/input/InputService.ts) 已无条件执行 10ms 解码反馈上限，并把处理后的 metadata 发布到输入队列与事件。体验开关的基础工作是显式配置化及观测隔离。
2. [区域选择 compiler](../../crates/xbox-streaming/src/policy/session/compiler.rs) 当前先匹配 force_region_ip，再匹配 preferred_region_name；桌面配置只传前者。force_region_ip 同时用于授权请求，动态服务区域需要独立配置和明确优先级。
3. [get_configuration](../../crates/xbox-streaming/src/session/api/session.rs) 已定义，当前串流主线尚缺实际调用与配置事实消费。[resolve_remote_play_context](../../src-tauri/src/mods/streaming/context.rs) 的 configuration_resolved=true 来自主机列表匹配；这条标记需要准确区分列表事实与当前会话配置事实。

## Goal

- 用户可以显式控制分辨率保持策略，并看到本次有效状态。
- WebGL2 使用更清晰的精度、坐标和资源合同，延续既有 CAS/USM/SR 图像保护与逐帧预算。
- 云服务器区域选项来自当前账户服务响应，所选区域被实际查询与会话创建消费。
- xHome 可以从有效区域发现目标主机，并沿发现区域创建、轮询和关闭会话。
- 当前 xHome configuration 中的媒体地址能够成为当前 ICE 代次的有效补候选。

## Scope

实施位于 main 的 Browser 媒体主线。Vue/TypeScript 负责设置、WebGL2、Browser 输入报文与 addIceCandidate；Rust 负责持久配置、区域、主机发现、当前配置事实、候选决策与信令编排。两条开发线继续复用公共 DTO 和领域合同。

本次交付边界是这四项改进。WebGPU、新输入功能、游戏专属配置、多玩家和原生媒体开发接续各自任务。既有 VideoFrame/Worker 最新槽、反馈监督、认证续期、TURN、保活、关闭和几何导航承担当前职责。

## Plan

### 工作包 1：防止分辨率下降体验开关

**用户行为**

- 设置名称：`防止分辨率下降`，归入「串流体验 / 分辨率」，标记实验性。
- 描述：`优先保持清晰度；设备压力较高时可能增加卡顿。实际画质随网络和服务端策略变化。`
- 持久键建议：`prevent_resolution_drops`；默认 false。新配置与缺字段的旧配置均规范化为 false，原有隐式 10ms 限幅改为用户主动开启；已显式保存的布尔选择按用户偏好保留。
- 云与主机共同使用这项体验偏好；下一次连接生效。本次状态由会话配置快照展示，保存操作使用现有生效提示。
- 有效状态同时依据当前 metadata 反馈能力；能力缺省时保留偏好并标明未生效原因，独立 VideoFrame 供给沿现有路径运行。

**机制与接入**

1. 在 Rust defaults/config_policy/grouping/service、StreamingConfigSnapshot、领域输入计划、Rust/TS 投影和前端 schema 中贯通该字段。
2. 把 InputService.normalizeFrameMetadata 拆成「协议时间规范化」与「分辨率保持策略」。开关开启时，仅在上报 decode delta 大于 10ms 时限幅，并保留非负 render delta；关闭时保留规范化后的未限幅反馈。
3. PlaybackService 的原始 observation 先进入诊断；输入发送使用独立副本。stats.videoFrameProcessed 保持每帧一次，携带原始/规范化事实；transport 策略另外输出模式、实际调整次数与调整差值的有界摘要。
4. 原始 receiveTime、expectedDisplayTime 和 callback 时间保留来源。expectedDisplayTime 作为预计呈现值记录；解码耗时由可用的 processingDuration/RTC 统计验证，缺证状态保留 unknown。
5. 当前无条件覆盖 frameRenderedTimeMs 的行为纳入协议规范化审计，确保时钟、字段来源、非有限值和缺证策略明确；策略限幅始终作用于上报副本。

**验收**

- 新配置与旧配置缺字段均默认关闭，显式开/关偏好保留；云/主机投影与下一连接生效。
- 报文回放覆盖 decode delta 小于/等于/大于 10ms、render delta 为负、时钟边界及缺字段，确认原始对象与诊断计数一致。
- 输入打包、metadata 队列年龄、释放与心跳继续满足当前预算。
- 同游戏、同请求档位和同网络条件下做开关 A/B；同时记录实际源尺寸、降档事件、RTT/jitter/loss、解码供给、冻结及提交长尾。目标是减少本地反馈压力导致的降档，服务端和 WebRTC 拥塞仍形成真实网络决策。

### 工作包 2：吸收局部 GPU 优化

**直接实施项**

1. 为视频、锐化、present 与 FSR 相关采样器显式声明 highp，核对目标 WebGL2 的编译支持。
2. 用 gl_VertexID 生成现有三顶点全屏三角形，收敛 vertexBuffer 与 attribute 状态；资源统计同步反映对象变化。
3. 为 USM/CAS 增加 sourceTexelSize uniform，按实际处理输入尺寸变化更新，复用现有 uniform 缓存。原始源尺寸、锐化 FBO 尺寸和最终输出尺寸分别消费。
4. 明确 shader 的链接与释放时机：fragment 在完成对应链接后释放；延迟创建的 present/EASU program 使用可重建的共享 vertex 源码或独立临时 shader，所有 program 链接后释放临时对象。初始化失败、context restore 和退出按当前资源闭环处理。

**图像与成本门槛项**

- 将源纹理 Y 翻转移入明确的采样坐标函数，上传、EASU、锐化 FBO、present 和截图共同遵循一份方向约定；通过像素与成本对照后采纳。
- 色彩处理亮度权重按当前 SDR 浏览器/VideoFrame 色彩事实核对 Rec.709。饱和度处理与 CAS/USM 噪声门限分别评估；锐化门限公式维持已有验证基线，色彩变化作为明确的像素行为变化验收。
- CAS 已具备黑色分母保护、抑噪、增益与过冲上限，继续复用这些数学合同。每帧 pass 数、空间采样规模和 CPU 像素拷贝预算延续当前基线。

**接入模块**

- [Renderers.ts](../../src/player/infra/render/Renderers.ts)
- [SuperResolutionWebGL2Renderer.ts](../../src/player/infra/render/SuperResolutionWebGL2Renderer.ts)
- [sharpening-math.ts](../../src/player/infra/render/sharpening-math.ts)、既有像素回放与生命周期验证。

**验收**

- 黑色、近黑、文字、噪声、彩色细线、上下方向图与运动画面；覆盖原画、USM、CAS、SR 和组合。
- 1:1、真实放大、缩放裁切、窗口尺寸变化、截图以及 context lost/restored。
- 数学等价项逐像素核对；高精度与色彩项按明确参考输出和误差预算核对。
- 按目标设备比较上传 CPU、GPU 完成代理、read-to-submit、提交/呈现 P95/P99、功耗与退出资源；同质量且成本稳定的细节进入正式装配。

### 工作包 3：服务返回区域驱动选择界面

**用户行为**

- 「连接与主机 / 云游戏」常规界面只提供一个 `云服务器区域` 入口，选项为 `自动（服务推荐）` 加当前服务返回的有效列表；说明为 `选择下一次云串流使用的服务器区域。`
- 首次加载、空列表、凭据缺省、刷新失败与过期缓存使用现有行状态和错误提示；已保存选择在列表更新后保留用户意图，实际回退显示有效区域。
- 授权路由日常使用自动策略。现有 force_region_ip 的已保存值保留，入口放入默认折叠的高级连接区域，名称为 `云服务登录兼容`；通过独立弹层调整 `登录地区`，说明为 `用于获取云服务授权，通常保持自动；登录问题排查时可调整。`
- 常规服务器选择与高级登录兼容各自使用任务明确的名称、说明和所在位置。登录兼容弹层提示 `串流服务器区域在云游戏设置中选择`；常规选择的自动项始终使用服务推荐区域。
- 服务器区域下一次连接生效；登录兼容使用现有授权更新和下一连接提示。串流 badge 显示实际控制面区域，媒体路径沿现有候选对诊断展示。

**Rust 合同**

1. 复用 parse_session_access_context 与 Region；增加现有 data RPC 的 getStreamingRegions(targetType) typed 请求、DataProvider 方法和安全 DTO：name、displayName/shortName、isDefault、continent、列表状态与 freshness。
2. Cloud/Home 按各自 token scope 查询。baseUri 与 token 留在 Rust；可用列表过滤空 name、无效 origin，按稳定名称去重。DTO 与缓存绑定账户及授权代次，配置路由变化后失效。
3. 新持久键建议：`xcloud_region_name`，默认空字符串。Cloud 选择优先级为：当前有效显式 name → 服务默认 → 首个有效区域。失效 name 保留偏好，输出明确回退原因。授权地区通过授权响应影响可用列表，服务器选择消费当前列表与显式服务器偏好。
   - 旧配置兼容只在首次迁移进行：旧 force_region_ip 能匹配当时有效服务器区域且服务器偏好尚未设置时，将匹配结果保存为显式 region name，界面显示该选择；授权路由值同时保留。
   - 用户选择 `自动（服务推荐）` 后清除服务器 name，消费服务默认区域；迁移版本与配置修订确保后续加载保留该选择。
4. 区域选择 helper 同时供会话 compiler 和 XcloudService 消费；目录/overlay 缓存 scope 使用实际选中 host，切区后的刷新沿现有 scope/lease 与旧响应拒绝合同。market、语言保持各自配置范围。
5. Home 的区域列表供主机发现使用；已发现目标主机的有效控制面绑定在 Home 创建链中优先消费。

**接入模块**

- [access.rs](../../crates/xbox-streaming/src/session/access.rs)、[session compiler](../../crates/xbox-streaming/src/policy/session/compiler.rs)
- [data service](../../src-tauri/src/mods/data/service.rs)、[data RPC](../../src-tauri/src/mods/data/rpc.rs)、[data types](../../src-tauri/src/mods/data/types.rs)
- [XcloudService](../../src-tauri/src/mods/data/services/xcloud_service.rs)、[RPC contract](../../src/shared/rpc/contract.ts)、[Setting.vue](../../src/pages/Setting.vue)、[设置 schema](../../src/pages/settings/setting-page-schema.ts)、中英文文案。

**验收**

- 自动与指定区域、名称大小写/去重、区域消失、新账户、token 更新、登录兼容变化、离线缓存、服务拒绝。
- 常规界面保持一个区域入口，高级登录兼容默认折叠；名称和说明区分连接服务器与获取授权的用途。旧授权偏好迁移、自动项实际使用服务默认、用户设置与迁移并发均验证。
- 查询与创建 HTTP 实际 origin 与有效 region 一致；切区前旧目录结果按 scope 拒绝。
- 手柄选择、焦点恢复、深浅主题、宽窄窗口和加载状态验证。

### 工作包 4A：xHome 跨区域发现

**查找合同**

1. StreamingQueryService 使用工作包 3 的有效 Home regions，逐区域构造既有 WebApiSessionGateway。
2. 列表查询默认区优先，空列表或可恢复网络失败接续其余区，取得可用主机列表后返回。目标主机查找优先有效目标缓存区，再默认区和其余区，持续到目标命中或预算耗尽。
3. 初始预算：每区域 5s，整体 30s，单窗口顺序请求；预算覆盖等待并接受取消。401 交回既有 scoped token recovery，按现有额度恢复；服务资格失败形成可诊断终止结果。全空列表与全部请求失败分别表达。
4. 每个结果绑定 discoveryRegionName 与内部有效 origin；按 serverId/id/deviceId 稳定身份去重。缓存使用账户、token 代次和目标身份，初始内存 TTL 5min；账户退出、路由变化和区域失效撤销。
5. resolve_remote_play_context 取得目标身份与发现区域，公共 session compiler 使用该绑定。创建、状态/配置查询、SDP/ICE、保活和关闭均使用同一 session base URL。
6. DataHostSummary 的原始 region 继续表达服务字段；发现区域使用独立字段或内部类型，保留来源语义。前端传目标身份，Rust 验证和选择本次区域。

**验收**

- 默认区空+其他区命中、首区超时、全空、全失败、401 恢复、403、取消与总预算。
- 首区有其他主机、目标位于后续区；重复身份、旧缓存、失效区域和账户切换。
- 从发现到创建、配置、信令、保活、关闭的一次完整 HTTP 回放证明 origin 一致。

### 工作包 4B：当前 configuration 到 ICE 的消费闭环

**事实与时序**

1. 在现有 Rust 会话/信令编排内取得当前 Home session 的 configuration。会话具备可读配置状态后按既有 polling 节拍查询；204 作为 pending。配置查询与远端 ICE 查询共享启动期限，各次 await 后核对 session cancel token。
2. 通过已有 SessionSignalingState 管理当前配置端点事实与补候选提交记录，原始列表地址在 negotiation plan 内作为显式 fallback。配置事实有独立 configuration generation；ICE restart 使用新的传输凭据重建提交记录。
3. 纠正 configuration_resolved 的来源语义：主机列表能力为 console-list；当前 get_configuration 成功为 session-configuration。运行投影和 trace 明确来源，列表能力与当前配置分别记录。
4. 已知字段显式解析 ipAddress/port、ipV4Address/ipV4Port、ipV6Address/ipV6Port，规范化 IpAddr/u16。实际配置地址优先，列表地址作为有来源标记的补充；非法、重复和过期地址单列拒绝原因。

**候选策略**

1. 公共 negotiation policy 根据当前配置事实生成 UDP host 补候选：每个有效地址使用服务端声明端口，并加入固定 9002 兼容端口。兼容端口有独立来源标记；按地址/端口/传输绑定去重。
2. mid、mLineIndex、ufrag 从当前远端 answer 的媒体/BUNDLE 传输推导，或复用同传输远端候选字段。候选 priority/foundation 使用标准且稳定的生成规则，既有 IcePolicy 继续承担地址族偏好与 Teredo 处理。
3. 远端候选先完成现有归一化与旧代次过滤，再合并当前配置候选，候选终止标记最后交付。远端 response=None/空列表时，配置补候选可以独立形成当前代次增量。
4. 同代次记录每个已提交补候选；重复轮询和同 Peer 画质协商保持幂等，ICE restart/Peer 重建按新代次重新绑定。服务快照的新鲜度过滤与配置补候选的新鲜度分别消费各自事实。
5. 配置 pending 与远端 EOC 共同形成完成判断：在现有 ICE 总期限内处理可能的配置补充，完成或明确放弃后发出最终 EOC；退出取消即时停止。Browser 消费继续走现有 RPC/poll 与 WebRtcTransport.addIceCandidate。

**接入模块**

- [session API](../../crates/xbox-streaming/src/session/api/session.rs)、[signaling API](../../crates/xbox-streaming/src/session/api/signaling.rs)
- [negotiation compiler](../../crates/xbox-streaming/src/policy/negotiation/compiler.rs)、[ICE policy](../../crates/xbox-streaming/src/session/signaling/ice.rs)
- [signaling flow](../../crates/xbox-streaming/src/session/signaling/flow.rs)、[signaling state](../../crates/xbox-streaming/src/session/signaling/state.rs)
- [streaming context](../../src-tauri/src/mods/streaming/context.rs)、[streaming projection](../../src-tauri/src/mods/streaming/projection.rs)、[WebRtcTransport](../../src/player/infra/webrtc/WebRtcTransport.ts)、[Browser runtime](../../src/streaming/runtime/browser-runtime.ts)。

**验收**

- IPv4/IPv6、声明端口/9002、非法地址、去重、当前配置优先和列表 fallback。
- 204→有效配置、配置晚到、空远端 ICE、仅 EOC、有效候选+EOC、查询失败和期限耗尽。
- 旧配置响应、旧 ufrag、同 Peer 画质协商、ICE restart、Peer 重建、退出取消。
- 用实际 Browser addIceCandidate 消费回放确认补候选的 mid/mline/ufrag 属于当前传输。
- 实机覆盖 LAN、家庭 NAT、异地、IPv4/IPv6、Teredo 与 TURN；记录选中来源、连接耗时、首帧耗时和失败原因。

## 执行顺序与里程碑

主窗口逐批执行，每批完成针对性回归与 RFC 状态更新。

| 里程碑 | 内容 | 进入下一批的证据 |
| --- | --- | --- |
| M0 | 配置/区域/端点/来源 DTO 与时序合同 | 默认迁移、有效状态与来源命名确定 |
| M1 | 动态区域列表与 xHome 跨区发现，工作包 3+4A | 区域回放、查询/创建 origin 一致、取消/认证回归 |
| M2 | 当前 configuration 与 ICE 注入，工作包 4B | 空/晚到 ICE、EOC、代次与 Browser 消费闭环 |
| M3 | 分辨率保持开关，工作包 1 | 配置开关、raw/wire 隔离、报文与输入预算回归 |
| M4 | GPU 细节，工作包 2 | 图像方向/数学、生命周期、逐帧供给与成本对照 |
| M5 | 联合验证与收口 | 自动验证通过，实机待验项与报告清楚登记 |

## Validation

媒体与网络对照采用同一稳定窗口和同一会话/媒体/渲染代次，至少包含：

| 维度 | 证据 |
| --- | --- |
| 网络健康 | RTT、network jitter、loss/NACK、接收 bytes/packets、jitter buffer、选中候选对 |
| 恢复链健康 | 首帧、冻结/恢复、关键帧与反馈动作、Browser 可观测恢复事实；能力缺项标记 unknown |
| media supply | 接收→解码→VideoFrame reader 同窗供给 |
| steady supply | Worker/latest-slot/在途/ACK、提交间隔与连续供帧 |
| submit/present 长尾 | read-to-submit、上传 CPU、GPU 完成代理、呈现观察 P95/P99；物理上屏接续设备证据 |
| 帧供给 delta | 接收/解码/reader/提交/呈现差分，覆盖与陈旧释放分别配平 |

- [x] 各工作包的确定性回放与有意义的回归通过。
- [x] Rust 修改执行 cargo fmt；公共 streaming 与受影响 Tauri 库测试通过。
- [x] 前端修改执行 pnpm lint:fix、pnpm typecheck、pnpm build 与全量 Vitest。
- [x] 新诊断采用事件或有界摘要，纳入现有生产 trace schema、预算与隐私投影检查。
- [ ] 用户实机验证覆盖 macOS WKWebView 与 Windows WebView2 的适用能力。
- [x] 完成后输出 Report，更新任务账本；实机收益与自动验证分别记录。

## Risks

- 分辨率保持会改变远端收到的客户端压力反馈；在重负载下可能提高延迟或冻结。开关状态、调整量与 raw 指标共同提供验证依据。
- 常规服务器区域与高级登录兼容分别承担连接选择和授权排查；自动项、旧配置迁移、目录 scope 与失效回退需要一致消费。
- 跨区扫描增加发现时延；目标缓存、每区期限、总期限与即时取消控制成本。
- 当前 configuration 的可用时间和端口可达性取决于服务/网络；9002 为兼容候选，实际选中路径证明收益。
- 较早交付 EOC 或复用旧传输字段会影响后续补候选；当前代次与统一完成判断是发布前门槛。
- sampler 精度、色彩与翻转会改变输出或导入成本；以像素和目标设备证据决定最终细节。

## Progress

- [x] 用户选择四项方向。
- [x] 定位当前实现、补充隐含行为和消费链缺口。
- [x] 制定配置、迁移、模块、执行次序及验证计划。
- [x] 收到代码执行指令。
- [x] M0 配置/区域/端点/来源合同与迁移。
- [x] M1 动态区域与跨区发现；完整 HTTP 回放确认控制面 origin 一致。
- [x] M2 当前 configuration 到 ICE、晚到配置、EOC、当前代次与 Browser 消费。
- [x] M3 默认关闭的分辨率保持开关；raw/wire 隔离与可用状态。
- [x] M4 局部 GPU 精度、顶点、texel uniform、链接资源；120 组像素对照一致。
- [x] M5 联合自动回归、浏览器设置验收、报告与账本收口。

## Execution Notes

- Date: 2026-11-10 | Status: completed
- Update: 四项实现均完成，完整控制面 origin、晚到 configuration、传输结束标记及旧代次回放通过；设备运行收益见实施报告的实机验收表。
- Decision: 按用户批注，分辨率保持默认关闭，新配置及旧配置缺字段均使用 false，开关下一连接生效。常规界面统一使用云服务器区域；授权路由沿自动策略与旧偏好运行，配置入口归入默认折叠的高级云服务登录兼容。
- GPU Decision: 保留现有上传 Y 翻转与亮度权重，实施 sampler highp、gl_VertexID、处理尺寸 texel uniform 与链接后 shader 释放。方向函数迁移和色彩权重变更继续按目标设备图像/成本门槛评估。
- Validation: 公共串流库 162 项、Tauri 142 项、前端 890 项、生产 trace 合同 2 项通过；cargo fmt、pnpm lint:fix、typecheck、build 通过。WebKit WebGL2 RGBA8 120 组对照最大码值误差 0；中英、深浅、宽窄设置与手柄焦点/失败重试验收通过。
