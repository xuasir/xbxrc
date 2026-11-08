# 主线本地环境依赖审计

日期：2026-11-08。范围：`main` 当前工作区的 `src`、`src-tauri`、`crates`、`scripts`、`tests` 及发行工作流。Git 基线为 `b4fd3912`，包含上一轮 Windows `Instant` 下溢修复。

七项环境耦合已完成修复，实施与验证结果见[修复 Report](2026-11-08-local-environment-isolation.md)。下表保留修复前的审计证据，当前行为以修复 Report 为准。

| ID | 影响范围 | 环境依赖与证据 | 建议 |
| --- | --- | --- | --- |
| E1 | Rust 回环 HTTP 校验 | [HttpTransport](../../crates/xbox-webapi/src/transport.rs#L35) 使用 reqwest 默认代理发现；[测试服务器](../../crates/xbox-streaming/src/session/flow.test.rs#L930) 监听 `127.0.0.1:0`。请求会受到进程代理变量及监听权限影响。[CI](../../.github/workflows/desktop-build.yml#L36) 显式设置 `NO_PROXY`。上一轮沙箱外校验在继承本机代理时有 19 项回环测试失败，加入 CI 的 `NO_PROXY` 后同组 209 项 Rust 测试通过。 | 沿用现有 HttpTransport，为回环测试显式装配直连客户端；生产代理策略沿既有策略演进。测试入口明确声明回环监听要求。 |
| E2 | trace 文件测试 | [测试日志目录](../../src-tauri/src/mods/runtime_trace/service.rs#L475) 固定为源码根下的 `target/runtime-logs-tests`；[初始化](../../src-tauri/src/mods/runtime_trace/service.rs#L486) 会删除整个根目录。测试要求源码目录可写；多个测试进程共用 checkout 时，一个进程可删除另一个进程的日志。重置目录测试也使用[固定目录](../../src-tauri/src/mods/runtime_trace/service.rs#L681)。 | 使用系统临时目录下的独立进程/运行目录，清理限定在当前测试运行；重置测试使用独立临时根目录。 |
| E3 | 发行流水线模拟 | [临时 Git 仓库](../../scripts/release-verify-pipeline.mjs#L24) 继承调用进程的 Git 配置；提交仅覆盖作者与 GPG 签名设置，仍会执行配置的 hooks。受控环境注入已复现临时仓库提交失败。 | fixture 的 Git 操作显式隔离 hooks、模板、全局配置及 Git 仓库环境变量；真实发行来源检查继续读取真实仓库。 |
| E4 | Rust 异步校验的调度负载 | [测试计划](../../crates/xbox-streaming/src/session/flow.test.rs#L1012) 将 SDP/ICE 期限设为 120 ms；[关闭测试](../../crates/xbox-streaming/src/session/flow.test.rs#L1072) 使用真实 100 ms 超时。这些断言依赖 runner 的实际调度与回环响应速度。当前正常条件下回归通过；高负载条件下的稳定性属于后续验证项。 | 使用事件握手确定操作先后，对逻辑回归设置充足的超时保护；实际响应耗时作为独立性能证据。 |
| E5 | 云游戏目录语言 | [resolve_catalog_language](../../src-tauri/src/mods/data/services/xcloud_service.rs#L203) 从进程 `LANG` 选择语言：`zh*` 对应 `zh-TW`，其余对应 `en-US`。GUI 启动、终端启动及 Windows 环境的 `LANG` 可不同。应用已有 `locale` 与 `preferred_game_language` 配置。 | 明确目录语言的配置来源，并通过现有 CloudCatalogScope 传入；语言映射由纯函数处理。 |
| E6 | 已认证状态的检查冷却 | [认证服务](../../src-tauri/src/mods/auth/service.rs#L140) 把系统墙钟传给 [begin_check](../../src-tauri/src/mods/auth/runtime_state.rs#L126)，冷却为 15 秒。墙钟向前调整可提前结束冷却，向后调整会使 `saturating_sub` 返回 0，延长认证检查短路的时间。 | 进程内冷却使用单调时钟；Token 绝对有效期与请求签名继续使用 UTC 时间。 |
| E7 | 官方 Xbox 对照采集脚本 | [launchChrome](../../scripts/official-xbox-capture.mjs#L57) 固定使用 `/Applications/Google Chrome.app/Contents/MacOS/Google Chrome`。该脚本依赖 macOS 默认 Chrome 安装位置，作用范围为辅助采集工具。 | 允许显式浏览器路径参数，提供平台路径发现与启动前存在性检查。 |

受控验证结果：

- 当前环境运行 `node scripts/release-verify-pipeline.mjs`：退出码 0，Browser 与 Native WIP 两版型的产物流程模拟通过。
- 在该命令子进程中通过 `GIT_CONFIG_COUNT` 注入临时 `core.hooksPath`，其中 `pre-commit` 返回 1：退出码 1，失败点为 fixture 的 Git commit。临时配置与 hook 仅作用于探针子进程。
- 上一轮 Rust 同组校验：以 `NO_PROXY=localhost,127.0.0.1,::1` 在允许回环监听的环境运行，140 项 xbox-streaming、7 项协议库、62 项 Tauri 库测试通过。
- 本轮以源码核对确认 E2、E4、E5、E6、E7；对应的跨进程、负载、GUI 语言及系统调时实验作为后续修复验证。

已有的环境边界：

- [默认键位](../../src-tauri/src/mods/config/defaults.rs#L100) 通过 `include_str!` 编译进产物；`CARGO_MANIFEST_DIR` 在这里用于编译期资源定位。
- [应用日志目录](../../src-tauri/src/shell/mod.rs#L115) 由 Tauri 运行期路径 API 解析，提供临时目录回退；源码目录用于当前 trace 测试装配。
- [换行合同](../../.gitattributes) 固定文本 LF，并为 Windows 脚本指定 CRLF；[工具链](../../rust-toolchain.toml) 固定 Rust 1.93.1，CI 固定 Node 24 与 pnpm 9。
- 打包校验通过 PATH 调用 macOS 的 `plutil` / `otool` / `lipo` / `codesign`，以及 Windows 的 PowerShell / `dumpbin`；Windows workflow 显式初始化 MSVC 并固定链接器路径。这些属于平台构建与包检查的工具要求。

本轮按照 E1 回环客户端隔离 → E2 测试目录隔离 → E3 Git fixture 隔离 → E4 调度验证 → E5/E6 运行期输入收敛 → E7 辅助脚本路径配置完成七项修复。下一步通过 release/test 新构建验证 Windows runner。
