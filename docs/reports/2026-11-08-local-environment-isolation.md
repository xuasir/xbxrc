# 本地环境依赖隔离修复 Report

## Summary

- 日期：2026-11-08。
- Related RFC：[本地环境依赖隔离 RFC](../rfcs/2026-11-08-local-environment-isolation.md)。
- [审计](2026-11-08-local-environment-dependency-audit.md)中的七项修复全部完成。校验通过夹具决定 HTTP 连接、目录、Git 配置和时间推进；运行期目录语言来自应用配置，认证冷却采用单调时间，辅助采集工具接受明确的浏览器路径。
- 本轮代码保存在当前 main 工作区，保留此前 Windows 注册表时间下溢修复和共享 HTTP 任务的改动。

## Delivered

| ID | 交付 | 结果 |
| --- | --- | --- |
| E1 | [会话 HTTP 装配](../../crates/xbox-streaming/src/session/flow.rs)和[测试客户端](../../crates/xbox-streaming/src/session/flow.test.rs) | 沿既有 HttpTransport 注入 `no_proxy` 客户端，Session/Signaling gateway 与 scheduler 共用同一客户端。CI 的 NO_PROXY 环境兜底已移除。 |
| E2 | [trace recorder](../../src-tauri/src/mods/runtime_trace/service.rs)与[writer 夹具](../../src-tauri/src/mods/runtime_trace/writer.test.rs) | 每个测试持有独立 TempDir，writer 先关闭文件与线程，再清理自己的目录；消除源码目录、共享根目录和 PID 重用假设。 |
| E3 | [Git 夹具配置](../../scripts/fixture-git.mjs)与[发行模拟](../../scripts/release-verify-pipeline.mjs) | fixture Git 和发行子进程使用独立配置、空 hooks/template/ignore/attributes；清理继承的 GIT_* 输入，显式提供作者、签名与换行策略。脚本自身路径决定配置来源，fixture 路径决定 RELEASE_REPO_ROOT 和 GITHUB_SHA。 |
| E4 | [异步会话回归](../../crates/xbox-streaming/src/session/flow.test.rs)与[时间合同](../../crates/xbox-streaming/src/session/operation.rs) | 请求观察使用通知，响应使用可控闸门；取消、晚到创建、旧 ICE、并发清理与 shutdown 的先后由事件控制。HTTP 期限为 10 秒，失败保护为 30 秒；逻辑超时通过虚拟时间验证，跨 runtime 原点使用真实单调时钟。 |
| E5 | [目录语言装配](../../src-tauri/src/mods/data/service.rs)与[目录 scope](../../src-tauri/src/mods/data/services/xcloud_service.rs) | 应用 locale 的 `zh*` 映射到 zh-TW，其余映射到 en-US，market 维持 US。云目录与主机安装库补充资料接收明确 CloudCatalogScope，缓存身份保留语言维度。 |
| E6 | [认证运行态](../../src-tauri/src/mods/auth/runtime_state.rs)与[回归](../../src-tauri/src/mods/auth/runtime_state.test.rs) | 冷却使用 Option&lt;Instant&gt; 与 Duration。首次检查执行认证流程；15 秒边界、in-flight、过期凭据和清理凭据后的检查通过注入时间验证。 |
| E7 | [采集参数](../../scripts/official-xbox-capture-options.mjs)、[采集入口](../../scripts/official-xbox-capture.mjs)与[工具回归](../../tests/tools/official-xbox-capture.test.mjs) | 启动时要求 `--browser-path` 指向可执行普通文件；`--no-launch` 连接已有调试浏览器。帮助与无效参数在文件输出和网络连接之前返回。默认 profile 每次创建独立系统临时目录，`--profile` 支持显式复用。 |

## Changes

- HTTP 装配沿用既有 transport、网关和凭据 Provider；Tauri + Vue 3 + TypeScript + Rust 边界保持一致。
- tempfile 和 Tokio test-util 通过开发依赖提供，Cargo.lock 已同步。生产 Browser 依赖合同通过。
- 认证 Token 的绝对有效期、请求签名与 trace 时间戳继续采用 UTC；进程内冷却与操作期限采用单调时间。
- `release:verify-pipeline` 纳入失败 Git 环境回归；`test:tools` 纳入发行前端校验阶段。beta/stable 沿用既有入口，main 开发构建保持暂停。
- 共享目录刷新和缓存接口按当前 HTTP 任务的接口对齐，保留账户、语言与区域校验。

## Validation

| 验证 | 结果 |
| --- | --- |
| 清空 NO_PROXY/no_proxy，将六个 HTTP/HTTPS/ALL_PROXY 大小写变量指向 `http://127.0.0.1:9` | 六个 Rust 包共 277 项通过；在允许回环监听的沙箱外执行。 |
| 六包完整 Rust 回归 | 279 项通过：AuthFlow 1、目录 flow 4、串流 141、协议 7、WebAPI 44、Tauri 82。 |
| 共享 Tauri 新增回归后的最终复核 | `cargo test -p xbxrc --lib --locked --offline`：88 项通过。 |
| 并行 trace 测试进程 | 两个进程各 23 项通过；recorder 保留活跃输出，各自析构仅清理自己的目录。 |
| LANG 干扰 | en_US.UTF-8、zh_CN.UTF-8、C 下各 3 项目录语言/缓存合同通过。 |
| 失败 hooks、模板、全局忽略、Git 配置和仓库环境变量 | 9 项发行合同通过；Browser 与 Native WIP 两版型模拟成功，错配和篡改产物按合同拒绝。 |
| 浏览器路径与 profile | 4 项 Node 回归通过；显式可执行文件、缺失/无效路径、no-launch、帮助与目录隔离覆盖。 |
| 前端与生命周期 | 69 个文件、674 项前端测试通过；lint、typecheck、build 和 3 项生命周期门禁通过。 |
| 格式、生产构建与工作流 | cargo fmt、cargo check --lib、Browser 依赖检查、6 个脚本语法检查、actionlint 1.7.12 与 diff check 通过。 |

## Risks

- 本次实测平台为 macOS；Windows runner 的最终兼容结果接续 release/test 新构建。
- HTTP 合同回归需要操作系统允许回环监听。本次沙箱内执行曾收到 PermissionDenied，随后在允许监听的执行环境完成代理干扰验证。
- 采集工具已完成参数与文件系统合同回归；实际浏览器启动、Xbox 登录及串流采样沿既有实机验收流程完成。

## Follow-up

将本轮改动同步到 release/test，复核 Windows 校验与打包。main 开发构建继续保持暂停。
