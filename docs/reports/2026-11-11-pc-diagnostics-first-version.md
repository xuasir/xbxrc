# PC 高级诊断首版实施报告

## Summary

- Related RFC: [精简诊断首版](../rfcs/2026-11-11-pc-diagnostics-first-version.md)。
- 2026-11-11，主窗口单窗口完成首版源码、自动验证与真实 Vue 组件交互验证。用户已以“先做一版”授权实施，并补充历史记录与生产常规 / 轻量档位。
- 自动异常和主动采集均保存独立时间窗口，在高级诊断逐条展示。忘记或取消导出后可回溯，记录可跨应用重启保留。
- 生产默认常规，用户在稳定运行后可切轻量；开发 / 生产有效模式由构建确定。桌面实机启动、最小化、原生保存和串流长测接续用户验收。

## Delivered

| 用户操作 / 状态 | 首版行为 |
| --- | --- |
| 请求失败 | Rust RPC 错误附带当次 `diagnosticIssueId`，失败入口直接导出对应问题。登录、主机 / 云游戏、库查询、设置、更新、鉴权恢复与串流主要失败入口共用保存动作。 |
| 后台发现异常 | 请求、媒体或输入异常创建问题记录，保留最近 10 秒可用安全摘要，采集后 10 秒详情；同类问题合并计数。 |
| 主动复现 | “开始采集”原位变为“停止并导出”，最长 10 分钟。页面收起停止界面轮询，Rust 继续管理采集任务。停止或到期后形成历史记录。 |
| 稳定运行 | 常规每 5 秒保留一次健康 / 输入周期摘要，轻量每 30 秒一次；关键事件继续记录。活动异常窗与主动采集采用已有详细合同。 |
| 忘记 / 取消导出 | 本地窗口和索引继续保留；高级诊断展示类别、时间、状态、重复次数和逐条导出按钮。重启时未完成手动采集标为中断。 |
| 导出 | 直接调用系统 ZIP 保存。通用导出优先活动采集，其后最近记录；历史条目和失败入口使用指定引用。取消保留记录，失败反馈支持重试。 |
| 开发模式 | 沿用生产核心健康证据，并提供开发细节与折叠 DevTools / 日志目录入口。开发启动可用 `XBX_TRACE_MODE=production` 复验生产策略。 |

## Implementation

### 采集、历史与成本

- `runtime_trace` 的现有 writer 同时维护常规日志与独立窗口，复用队列、回执、flush 与 IO 统计。诊断状态、期限、索引和导出范围由 Rust 持有。
- 最近 10 秒的安全周期摘要保存在 96 行 / 256 KiB 有界缓冲，包括已采样写入主日志的摘要。触发窗口时，已写行只复制到保留记录，其余行重放至 trace；记录使用自己的连续 `seq`，`sourceSequence` 关联可用的主日志序号。
- 同一 run、来源、操作、错误码在首次异常后 30 秒内合并，后文最多延续至首次异常后 40 秒。自动详细模式每分钟最多开启三段，writer 同时最多八个活动窗口。
- 记录文件位于日志目录的 `diagnostic-records/`。索引合计最多 12 条；窗口文件合计最多 64 MiB，自动单段 2 MiB，手动单段 16 MiB。清理保护活动窗口，较早记录按容量裁剪；单段超限写入截断标记。
- 无活动窗口时，常规 flush 省去诊断历史目录扫描。手动期限采用 `Instant`；界面可见时每 2 秒刷新，隐藏 / KeepAlive 停用后暂停显示轮询。
- 旧 `debug` / 手动 trace 档位从产品入口移除；兼容字段保留在存储层并按构建默认归一化。新配置 `diagnostics_lightweight=false`，修改后即时应用。

### 一致导出与安全范围

ZIP 包含 `runtime-trace.jsonl`、`manifest.json`、`config-snapshot.json`、`summary.md`。Rust 直接写入系统对话框选择的路径，导出锁贯穿对话框与实际保存，后台线程负责文件扫描与写入。

导出先打开来源文件并固定长度，两次扫描读取同一快照，轮转后的句柄继续可读。主日志遵守确认 flush 的截止序号；独立窗口声明 `sequenceScope: diagnostic-record`，以其自身序号和固定长度表达范围。活动记录可先导出当前前缀，之后继续补齐本地记录。

生产包内容上限 32 MiB，开发 80 MiB，预留 256 KiB 元数据空间；超限保留最近范围并标记。行读取在分配前限制 65536 字节。生产核心字段复用现有 trace 投影，其他 payload 省去未知文本与身份字段，会话映射为包内 `session-1` 等引用，包内 writer 信息经过安全投影并省去绝对路径。

manifest 明确来源 run、来源构建 / 模式、导出器身份、实际范围、flush / 丢失 / 截断与稀疏健康覆盖。配置快照当前为导出时的安全配置，`configurationScope` 已声明这一范围；跨 run 导出分别表达来源和导出时身份。

## Validation

| 检查 | 结果 |
| --- | --- |
| `cargo fmt` | 通过 |
| `cargo test -p xbxrc --lib` | 153 项通过，其中 runtime trace 42 项 |
| `cargo check -p xbxrc` | 通过 |
| `cargo check -p xbxrc --release` | 完整编译与最终增量编译通过 |
| `pnpm lint:fix` | 通过 |
| `pnpm typecheck` | 通过 |
| `pnpm build` | 通过 |
| 定向 Vitest | 8 个文件、81 项通过 |
| `pnpm test:navigation-coverage` | 69 个 Vue 文件、183 处交互，无违规 |
| `pnpm test:trace-production` | 2 项通过 |
| `git diff --check` | 通过 |
| 真实 Vue 组件 + 现有几何导航 | 开始 / 停止、跨页面、取消、失败重试、历史导出、生产档位、开发工具和窄窗口验证通过 |

定向前端验证覆盖 `SettingDiagnosticsSection`、`trace-export`、`browser-runtime-trace`、`stream-errors`、`useStreamExecution`、`useAppUpdater`、`use-playtimes` 与 `navigation-controls`。

Rust 回归覆盖常规 / 轻量降频、30 条健康摘要全前文保存、局部连续序号、重复异常结束后追加、后台期限、历史裁剪与重启中断、窗口容量、ZIP 固定长度快照、轮转、顺序、保留范围、大小限制及安全投影。release check 仅有既有查询服务的两项 dead_code 警告。

真实组件预览使用 RPC fixture，实际执行 Vue 组件、Focusable / FocusScope 与几何导航。验收与实机边界见[验证记录](../../dev-docs/validation/2026-11-11-pc-diagnostics-first-version/README.md)。临时本机预览服务已关闭。

## Limits / Acceptance

- 轻量首版主要降低持续写盘。健康摘要构造与 IPC 继续维持已有 1Hz 密度；每条 `window/coverage` 表达实际证据范围。设备 CPU、功耗、写盘量和提交 / 呈现长尾需要同窗实机测量。
- 自动详细模式、writer 活动窗口、单段 / 总空间以及队列 / IO 都有边界；超过预算或较早记录被清理时，证据可能部分可用。索引状态、文件截断标记与包内清单用于核对实际范围。
- 手动采集开始时完整配置固化、专用早期启动记录、手工标记与用户复现说明属于后续按实际问题扩展的方向。当前 `summary.md` 提供范围与分析入口，串流结论继续按网络、恢复链、media supply、steady supply、submit / present 长尾及帧供给 delta 复盘。

实机验收按三个流程执行：制造一次请求失败并取消导出，重启后从历史记录导出；主动采集后最小化应用，返回后停止并保存；同串流条件比较常规与轻量的写盘量、供帧及功耗。生产验收使用发行构建或开发启动的 `XBX_TRACE_MODE=production`。

## 2026-11-11 精简布局修订

按用户新要求，将常规 / 轻量切换放在生产页面首行，改为紧凑分段选择；采用一条当前档位说明、采集 / 导出操作行与历史列表。移除重复标题和多层卡片背景，历史保留操作名称、次数、时间范围与状态；同日结束时间省去重复日期，窄窗口保持导出按钮与条目并列。

历史上限调整为 12 条，Rust 索引裁剪、文件裁剪与 RPC 保留合同共用常量。旧索引及窗口文件在启动时同步清理至新上限，活动采集继续受到保护，总空间保持 64 MiB。

本轮 Rust `runtime_trace` 定向 44 项、诊断 / 导航前端 25 项、最终诊断组件 4 项、typecheck、build、191 处导航覆盖与 diff check 通过。真实组件验证首行切档、400px 内容窗口与紧凑历史布局，证据为[桌面截图](../../dev-docs/validation/2026-11-11-pc-diagnostics-first-version/compact-desktop.jpg)和[窄窗口截图](../../dev-docs/validation/2026-11-11-pc-diagnostics-first-version/compact-narrow.jpg)。

按用户对按钮比例的反馈，诊断页操作按钮由共享的 44px 高 / 16px 字号收紧为 34px 高 / 正文说明字号（默认 13px），主操作最小宽度 96px；分段切换为 30px 高、字重 500。操作与历史导出使用同一字号和间距，开发工具按钮采用内容宽度。真实 DOM 测量确认 34px / 30px 与 13px、500 字重，结果见[按钮测量](../../dev-docs/validation/2026-11-11-pc-diagnostics-first-version/button-metrics.json)。最终 `pnpm lint:fix` 通过，截图已更新为调整后的效果。
