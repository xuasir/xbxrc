# 本地环境依赖隔离 RFC

## Status

- Completion: 已完成
- Current State: completed
- Owner: Codex 主窗口，单窗口直接执行
- Last Updated: 2026-11-08
- Execution Authorization: 用户在七项[审计结果](../reports/2026-11-08-local-environment-dependency-audit.md)后明确要求“先解决”；按该授权执行全部七项，优先完成校验环境隔离。
- Final Report: [本地环境依赖隔离修复 Report](../reports/2026-11-08-local-environment-isolation.md)。

## Goal

测试通过自身夹具决定代理、时钟、目录及 Git 行为；运行期语言由应用配置决定，冷却采用单调时钟；辅助采集工具通过显式参数决定浏览器路径。

## Scope

1. E1：沿用 HttpTransport 与会话网关，为回环 HTTP 测试装配明确的直连客户端。
2. E2：trace 测试使用独立临时运行目录，清理限定在本次运行；验证并行进程隔离。
3. E3：发行模拟的 Git 操作隔离系统/全局配置、hooks、模板及仓库环境变量，fixture 与各子进程共享同一套显式配置。
4. E4：以事件握手和可控时钟验证 pending、取消及晚到响应；真实 HTTP 合同测试采用充足的失败保护期限。
5. E5：云游戏目录和详情补充信息从应用 locale 派生目录语言，通过已有 CloudCatalogScope 保持缓存语言身份。
6. E6：认证检查冷却使用可注入的 Instant 与 Duration；Token 绝对有效期和签名继续使用 UTC。
7. E7：官方采集脚本接受明确的浏览器可执行路径，临时 profile 使用系统临时目录，启动前校验参数和文件。

## Implementation Boundaries

- 保持 Tauri + Vue 3 + TypeScript + Rust 和既有 HTTP、会话、信令与认证合同。
- 本轮的环境输入隔离沿现有构造器、配置 Provider 与测试夹具扩展。
- 当前工作区另有非串流 HTTP 审计；本轮修改共享文件时保留已有改动，局部处理目录语言、冷却及测试客户端装配。
- CI 使用现有 beta/stable 入口和现有平台工具链；main 开发构建保持暂停。

## Plan

1. 完成回环客户端、日志目录和 Git fixture 隔离。
2. 收敛异步测试计时，完成语言、冷却与浏览器路径参数处理。
3. 运行环境干扰验证、适用回归、格式与构建检查。
4. 更新审计结论、生成 Report 并收口任务记录。

## Validation

- [x] 无 NO_PROXY 且代理指向失败地址时，回环 HTTP 合同回归通过。
- [x] trace 测试在独立临时目录运行，并行测试进程保留各自日志。
- [x] 注入失败 hooks、全局忽略规则、Git 模板及仓库环境变量时，两版型发行模拟通过。
- [x] pending、取消、旧响应与超时回归由事件/虚拟时间推进。
- [x] 不同进程 LANG 下，应用 locale 与目录语言的映射保持一致。
- [x] 认证冷却首调用、截止边界及 in-flight 回归使用注入的单调时间通过。
- [x] 浏览器路径缺失/无效/显式提供及 no-launch 参数验证通过，profile 使用独立系统临时目录。
- [x] Rust fmt、相关 crate/Tauri 回归、cargo check、生命周期门禁与发行合同检查通过。
- [x] 适用脚本语法、前端类型与构建检查通过，最终差异和文档链接有效。

## Progress

- [x] 审计七项环境依赖并取得用户执行授权。
- [x] P1 校验环境隔离。
- [x] P2 计时与运行期输入收敛。
- [x] P3 验证与收口。

## Execution Notes

- 2026-11-08：从当前 main 工作区直接执行；保留已完成的 Windows Instant 修复及其他任务的现有改动。
- 2026-11-08：七项代码修改完成。回环客户端显式 no_proxy；trace recorder/writer 使用独立 TempDir；Git fixture 与子进程共享隔离配置；HTTP 顺序由事件闸门控制，逻辑期限使用暂停时钟；目录语言跟随应用 locale；认证冷却使用 Option<Instant>；采集脚本要求显式 browser-path，并提供独立系统临时 profile。
- 2026-11-08：清空 NO_PROXY、注入失败代理的六个 Rust 包共 277 项回归通过。两进程各 23 项 trace 回归通过；en_US.UTF-8 / zh_CN.UTF-8 / C 下各 3 项目录回归通过。9 项发行合同及两版型模拟、4 项工具参数回归、674 项前端测试、lint/typecheck/build、3 项生命周期门禁、actionlint 和 Browser 依赖合同通过。
- 2026-11-08：共享目录刷新与缓存接口按当前 HTTP 任务对齐；六包完整回归 279 项、最终 Tauri 回归 88 项、生产 cargo check 和格式检查通过。最终 Report 已生成，任务完成。Windows runner 的最终结果接续 release/test 新构建。
