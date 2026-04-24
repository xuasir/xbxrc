继续推进 SDL3，建议按“先并行验证，后单点切换，最后删除旧路”的顺序做，避免再次出现“名字切了，底层没换”的假迁移。

**计划**

1. 先把目标收窄成“真实 SDL3 物理接入验证”
   - 只验证设备发现、热插拔、标准按钮/摇杆/扳机输入、基础 rumble
   - 验收对象固定为 Windows/macOS 上的 Xbox 官方手柄
   - 当前阶段不碰 `logical pad`、`xbxengine`、前端导航逻辑

2. 新建真正的 `ohmygamepad-sdl3` 实现
   - 不再 re-export `ohmygamepad-gilrs`
   - 至少补齐 `source`、`event`、`backend`、`service` 四层
   - 输出和 `ohmygamepad-core` 现有契约一致的 `DeviceLifecycleEvent` 与 `RawDeviceSample`

3. 先做旁路接入，不改默认 selector
   - `host` 保持现在的 `gilrs + 平台 haptics` 主线
   - 通过单独 probe、临时测试入口或 feature flag 跑 SDL3 后端
   - 先拿到真实设备数据，再决定是否切主线

4. 做 SDL3 能力对照表
   - Windows：输入、普通 rumble、trigger rumble、热插拔
   - macOS：输入、普通 rumble、热插拔
   - 记录与当前 `gilrs + macos-gccontroller + win-xbox-haptics` 的差异
   - 明确哪些能力已经覆盖，哪些能力还缺

5. 设计 haptics 收敛策略
   - SDL3 能稳定覆盖的能力直接并入主线
   - SDL3 覆盖不到、但业务必须保留的能力单独列出来
   - 在这一步决定 `macos-gccontroller-haptics` 和 `win-xbox-haptics` 是保留增强层，还是彻底下线

6. 完成真实主线切换
   - `selector` 切到 SDL3
   - `host` 切到 SDL3 service
   - DTO / contract / 持久化配置增加兼容迁移
   - 旧配置中的 `gilrs` matcher 自动迁移或兼容解析

7. 最后删除旧主线路径
   - 删除 `gilrs` 作为默认桌面输入后端的职责
   - 删除临时 feature flag / 探针适配层
   - 只在文档明确允许的情况下保留平台专属增强模块

**每阶段的退出条件**

- Phase A：SDL3 后端能独立跑起来，并在 Windows/macOS 上拿到正确输入
- Phase B：SDL3 基础 rumble 在目标设备上稳定
- Phase C：SDL3 与现有主线的能力差异被完整列清
- Phase D：完成默认切换且配置兼容
- Phase E：删除旧主线路径，仓库里不再存在“默认双轨”

**我建议的实际落地顺序**

- 第一步做 `ohmygamepad-sdl3` 最小后端
- 第二步补一个 SDL3 probe/demo
- 第三步做 Windows Xbox 官方手柄验证
- 第四步做 macOS Xbox 官方手柄验证
- 第五步再决定主线切换

最关键的约束只有一个：先拿到真实 SDL3 输入与 rumble 证据，再改 selector 和协议语义。

---

## 2026-04-22 执行结果（直切收尾）

- 已完成：`ohmygamepad-sdl3` 不再是单行 re-export，已补齐 `event/source/backend/runtime/service` 五层边界。
- 已完成：默认 selector 与 host 装配切到 SDL3 单轨；`gilrs + 平台 haptics` 不再作为默认主路径。
- 已完成：协议与前端 contract 收口到 SDL3 语义；旧配置里的 `gilrs` matcher 读取时兼容映射到 `sdl3`。
- 已完成：`cargo check/test` 覆盖 `ohmygamepad` 关键 crate 与 `xbxrc` 主包，全链路通过（详见 RFC Execution Notes）。
- 待补：Windows/macOS Xbox 官方手柄实机输入/rumble 记录需在目标设备执行并回填到 RFC Validation。
