专注 `crates/xbxengine/core/src/transport/rtc/` 的拆解，建议按“先定边界，再搬文件”的方式做，不要一上来按目录机械平移。

## Status

- Completion: 已完成
- Current State: completed
- Owner: TBD
- Last Updated: 2026-04-09

**状态**
- 已完成，进入维护态（2026-03-23）。

**先给结论**
- 目标不是把 `rtc` 拆成很多目录，而是把它拆成 4 个清晰层：
  - `facts / projection`：纯状态归约层
  - `policy / recovery / bwe / executor`：纯决策层
  - `connection / stream / protocol / sdp`：边界适配层
  - `stack / pipeline / session`：主线编排层
- `stack.rs` 应该最后收敛成 orchestrator，而不是继续承载业务逻辑。见 [`/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stack.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stack.rs)

**命名收口**
- 外层 `crates/xbxengine/core/src/media/` 继续表示通用媒体内核，不改语义。
- `transport/rtc/media/` 统一改名为 `transport/rtc/stream/`。
- `stream/` 专指 RTC 传输侧的媒体流适配层，负责 RTP / frame / sink / route，不再和外层 `media/` 混名。
- 这次迁移的重点不是“目录更多”，而是“职责更清楚，路径名一眼可读”。

**推荐拆解顺序**

1. **先冻结主线入口**
- 保持 `rtc/mod.rs` 对外 API 稳定，只调整内部实现，不先改调用方。
- 先把现在已经存在的分层边界固定下来：`facts`、`session`、`projection`、`policy`、`executor`、`recovery`、`connection`、`stream`。见 [`/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/mod.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/mod.rs)

2. **把纯数据和纯决策先剥出来**
- `facts.rs` 保留事件事实，不要混入运行时副作用。
- `projection/*` 只做事实归约，不直接调用连接或媒体服务。
- `policy/*` 和 `recovery/*` 只产出 proposal / action，不直接执行网络和媒体操作。
- `executor/*` 只负责把 action 落地。
- 这一层拆干净后，后面搬 `connection` 和 `stream` 才不会互相牵扯。见 [`/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/facts.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/facts.rs) 和 [`/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/projection/mod.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/projection/mod.rs)

3. **把 `connection/` 收窄成边界适配层**
- `connection/service.rs` 这类文件现在通常会同时管：
  - peer connection
  - data channel
  - metrics
  - rumble
  - control channel
- 拆的时候优先按“副作用类型”切：
  - `connection/peer`：WebRTC peer lifecycle
  - `connection/data_channel`：输入/控制/消息通道
  - `connection/metrics`：RTT、loss、transport path、TWCC
  - `connection/control`：session control / reconnect staging
- 原则是：`connection` 负责“连接对象”，不负责“恢复决策”。
  见 [`/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/connection/mod.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/connection/mod.rs)

4. **把 `stream/` 原 `media` 收窄成媒体 ingress/sink**
- `stream/` 现在主要是视频源、音频、包路由、NACK 调度、帧节奏。
- 拆法建议：
  - `stream/video_source`：视频 RTP 到 frame 的组帧链
  - `stream/audio`：音频接入和输出
  - `stream/packet_router`：包分类和路由
  - `stream/nack_scheduler`：NACK 相关调度
  - `stream/frame_cadence`：帧节奏与延迟窗口
- 原则是：`stream` 负责“RTC 侧媒体数据流”，不负责会话恢复策略。
 见 [`/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stream/mod.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stream/mod.rs)

5. **把 `pipeline/` 压缩成会话循环**
- `pipeline` 当前是“媒体会话循环 + 监督”的位置，适合保留成主循环壳。
- 里面的 recovery 触发、决策和执行如果还在，就继续往 `recovery/policy/executor` 推。
- 让 `pipeline/session.rs` 只做：
  - 消费事实
  - 更新 session 投影
  - 发起编排动作
- 见 [`/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/pipeline/mod.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/pipeline/mod.rs)

6. **最后整理 `stack.rs`**
- `stack.rs` 最终应该只剩三类职责：
  - 组装各子系统
  - 暴露统一 trait / facade
  - 转发上层命令到对应层
- 它不应该再包含 policy 判定、恢复细节或媒体算法。
  见 [`/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stack.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stack.rs)

**我建议的目标目录形态**
- `rtc/facts.rs`
- `rtc/projection/{connection,media,recovery,bwe,diagnostics,snapshot}.rs`
- `rtc/policy/{bwe,recovery,reconnect,planner}.rs`
- `rtc/recovery/{signal,diagnosis,escalation,coordinator,startup,policy}.rs`
- `rtc/executor/{peer,...}.rs`
- `rtc/connection/{peer,data_channel,control,metrics,rumble,service}.rs`
- `rtc/stream/{audio,video_source,packet_router,nack_scheduler,frame_cadence,sink,service}.rs`
- `rtc/pipeline/{session,supervisor}.rs`

**实际执行时的注意点**
- 一次只搬一个边界，别同时重命名太多模块。
- 先移动“纯类型 / 纯函数”，后移动“有副作用服务”。
- 每次迁移都要保证 `mod.rs` 的 re-export 还是稳定的。
- 任何会改外部调用面的拆分，最后再处理。

**最稳的第一刀**
- 我建议先拆 `connection/service.rs` 的内部职责，然后把 `projection/*` 和 `facts.rs` 冻结成纯模型层。
- 这两步收益最大，也最不容易引发大范围联动。

如果你要，我下一步可以直接给你出一份“`transport/rtc` 具体拆分路线图”，按 `第 1 周 / 第 2 周 / 第 3 周` 写到文件级，并把 `media -> stream` 的重命名顺序一起排进去。

**执行进度（2026-03-23）**
- 已完成：
  - `facts / projection / policy / executor` 骨架已落地，`session policy` 已接回主线。
  - `transport/rtc/media -> transport/rtc/stream` 命名收口已完成。
  - `connection/service.rs` 已按 negotiation / builder / lifecycle / data_channel / transport_metrics / helpers 收窄。
  - `pipeline/ingress.rs`、`pipeline/observation.rs` 已从 `pipeline/session.rs` 中拆出。
  - `pipeline/session_loop.rs` 已新增，视频会话 loop、ingress 提交、transport observation 消费与 fact 写回已从 `pipeline/session.rs` 中剥离。
- 本轮新增进展：
  - `stack/negotiation.rs` 已新增，`create_offer`、`apply_remote_description`、`add_remote_ice_candidates` 与本地 ICE 查询桥接已从 `stack.rs` 中拆出。
  - `stack/media_pipeline.rs` 已新增，legacy frame pipeline 挂载和音频播放会话管理已从 `stack.rs` 中拆出。
  - `stack/lifecycle.rs` 已新增，runtime config 同步、rebuild reset 流程与 stop 生命周期编排已从 `stack.rs` 中拆出。
  - `stack/runtime_port.rs` 已新增，render/runtime 读写入口与 runtime stats 合并逻辑已从 `stack.rs` 中拆出。
  - `stack/transport_session.rs` 与 `stack/runtime_stats.rs` 已新增，transport fact/command 桥接与 media snapshot 合并逻辑已从 `stack.rs` 中拆出。
  - `stack/input_loop.rs` 已新增，输入轮询、游戏手柄采样与输入流状态已从 `stack.rs` 中拆出。
  - `recovery/runtime_state.rs` 已新增，`recovery/coordinator.rs` 中的 recovery profile、runtime diagnosis label、coupling state、fresh output 与 decoder backend failure 等 pure runtime 逻辑已抽离；`bwe/policy.rs` 已改为直接依赖该模块的 coupling 类型。
  - `recovery/nack_outcome.rs` 已新增，最近一次 NACK outcome 的窗口判断、cloud startup budget 与 delta/reference 帧分支已从 `recovery/coordinator.rs` 中拆出。
  - `recovery/hard_stall.rs` 已新增，硬停滞判定、decoder reset / reconnect candidate 升级条件已从 `recovery/coordinator.rs` 中拆出。
  - `recovery/decoder_backend_failure.rs` 已新增，decoder backend failure 的信号门控、profile 判定与 reset spacing 逻辑已从 `recovery/coordinator.rs` 中拆出。
  - `recovery/repeat_suppression.rs` 已新增，`WaitKeyframe` / `AdapterIdleTimeout` 的重复抑制窗口判定已从 `recovery/coordinator.rs` 中拆出。
  - `bwe/policy/coupling.rs` 已新增，TWCC 下的 recovery coupling hold / startup backoff / reference-chain backoff 分支已从 `bwe/policy.rs` 中拆出。
  - `bwe/policy/twcc_rules.rs` 已新增，TWCC 下的 RTT / loss / cooldown-ramp 主链规则已从 `bwe/policy.rs` 中拆出。
  - `bwe/policy/hybrid_rules.rs` 已新增，`hybrid` 模式下无 TWCC 输入时的 RTT / loss / cooldown-ramp 规则已从 `bwe/policy.rs` 中拆出。
- 当前判断：
  - `pipeline/session.rs` 已基本回到“会话壳 + 任务拉起”职责，`pipeline/` 这条线达到本 RFC 预期。
  - `stack.rs`、`recovery/coordinator.rs`、`bwe/policy.rs` 仍保留可接受的维护厚度，但职责已经收正，不再把“继续按行数细拆”作为目标。
  - `recovery/coordinator.rs` 已回退到编排壳；`bwe/policy.rs` 已按 `coupling / twcc_rules / hybrid_rules` 收口，主文件已回到“共享上下文组装 + 模式分发”角色。
- 阶段结论：
  - 如果按“目录职责迁移”和“主线权责收口”衡量，本 RFC 已完成。
  - 如果后续出现新的维护压力，应按真实变更点做局部调整，而不是继续为了降行数机械细拆。
- 收尾结论：
  - 本次 `transport/rtc` RFC 改造到此结束，后续转为维护态。
  - 后续工作以行为回归、增量需求和真实维护压力驱动的局部调整为主，不再以继续拆分文件为目标。
