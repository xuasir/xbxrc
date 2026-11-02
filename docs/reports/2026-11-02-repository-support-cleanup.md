# 仓库辅助内容精简与对齐

2026-11-02。范围为 `dev-docs/`、`docs/`、`scripts/`、`tests/`；以当前源码、调用关系、后续 RFC 和正式回归覆盖作为删减依据。

## 精简结果

| 内容 | 处理与依据 |
| --- | --- |
| 10 份任务快照 | 合并为[历史任务索引](../project-task.archive.md)，73 次重复 ID 按最新快照归并；1321 条历史任务可检索，46 条原进行中/待验任务保留在[当前账本](../project-task.md) |
| 旧恢复层启用、迁移指南及基线 | 删除 3 份文件；当前实现使用 receive/ReferenceChain/RecoveryLedger 闭环，旧 SimplifiedRecoveryCoordinator 入口已消失；历史 RFC 标记接续关系 |
| FSE XInput 验证指南、SDL3 切换备注 | 删除 2 份旧指南；统一手柄 RFC 与报告承载当前后端、owner 和 Windows/ROG 待验项；旧 gate/常驻采样任务标记 Superseded |
| superpowers 显示策略计划 | 删除与既有 RFC/Report 重复的待办清单 |
| 公共启动临时复现 | 删除 3 个断言修复前缺陷的 `.cjs/.rs/.py` 文件；审查报告和 metrics 保留证据，验证入口对齐正式源码回归 |
| pipeline 分析 | 合并 packet-to-frame 两个版本，采用完整的 2026-04-15 分析；frame-to-display 同步进入 `docs/isu/`，两份均标注历史方案和当前恢复边界 |
| Python 测试 | 浏览器稳定性测试从 `scripts/tests/` 迁入 `tests/runtime-trace/`，与生命周期门禁统一发现；全部夹具继续覆盖有效场景 |
| 文档与工具入口 | 补齐四目录职责和专项验证索引，修正 FFmpeg SDK 到 `src-tauri/resources/ffmpeg/`，对齐发布/构建/测试命令 |

任务账本由 309540 字节缩到约 78 KB；四目录合计减少约 1.5 MiB。历史完成任务保留 ID、标题、状态、日期与现存方案/报告入口；原始完整备注和快照可按历史索引记录的 Git 基线读取。历史未完成备注及原账本执行记录继续保留。

## 引用与保留边界

对齐 23 条移位/根目录误写链接；261 条已删除文件的链接改为带原路径的历史文字。历史文件的恢复方式在对应文档中说明，当前有效的本地 Markdown 链接均指向现存文件或目录；检查支持行号后缀和锚点。

浏览器供帧/SR60fps 工作仍处于实机验收阶段，其 dated metrics、回放和固定复杂度基线继续保留。最新分析依赖早期 Python 复算模块和生产模块回放，依赖链已核对。发布、技能同步、FFmpeg 装配与官方网页对照采样脚本均有有效用途和引用。

## 验证

- `python3 -B -m unittest discover -s tests/runtime-trace -p 'test_*.py'`：21 项通过；生命周期矩阵 22 场景、23 份 JSONL 夹具全部使用。
- `pnpm release:verify-pipeline`：macOS/Windows bundle、collect、publish 与 latest.json 合同通过。
- `node scripts/measure-browser-runtime.mjs dev-docs/validation/browser-runtime-optimization-baseline.json`：固定基线与当前 AST 测量入口正常，runtime 可变绑定 74。
- 11 个 `.mjs` 的 `node --check` 与 `scripts/`、`tests/` 下 Python 源文件的 AST 语法检查通过。
- 四目录本地 Markdown 文件/目录引用、任务 ID 保全和 `git diff --check` 通过。

下一步继续按[浏览器 RFC](../rfcs/2026-10-28-browser-runtime-simplification-and-stability.md)收口像素读取成本、drawable 配对和 SR60fps 新日志验收。
