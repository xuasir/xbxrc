# Docs Guide

`docs/` 是仓库唯一文档根目录。

## Structure

- [`project-task.md`](project-task.md)
  - 当前任务追踪入口
  - 小任务直接内联记录
  - 复杂任务只保留摘要，并引用 RFC / Report
- [`isu/`](isu/)
  - 探索期脑暴、方案雏形、问题空间沉淀
- [`rfcs/`](rfcs/)
  - 复杂任务的方案、范围、进度、验证计划与执行记录
- [`reports/`](reports/)
  - 复杂任务完成后的结案总结
- [`references/`](references/)
  - 不属于任务账本、RFC、Report 的长期参考文档
- [`test-assets/`](test-assets/)
  - 测试画像、样例数据与验证辅助文档
- `project-task.archived.*.md`
  - 历史归档快照

## Working Model

- 默认开发任务通过 `task-run` 进入工作流
- 简单任务直接执行并登记到 `project-task.md`
- 复杂任务先进入 RFC 路径，澄清完成后再执行
- 探索性问题通过 `deep-brainstorm` 产出 ISU，再视情况转任务
- 历史追溯优先使用 `history-search`、`module-history`、`decision-trace`、`similar-work`

## Writing Rules

- `project-task.md` 保持轻量，优先记录当前任务和最近完成项
- RFC 记录执行中的进度、里程碑、决策和风险
- Report 只记录完成态结果
- ISU / RFC / Report 模板跟随 skill 维护：
  - [`deep-brainstorm` ISU 模板](../.agents/skills/deep-brainstorm/references/isu-template.md)
  - [`task-run` RFC 模板](../.agents/skills/task-run/references/rfc-template.md)
  - [`task-run` Report 模板](../.agents/skills/task-run/references/report-template.md)
- 参考文档进入 `references/`，不再散落在根目录
- `docs1/` 已退役，后续文档统一进入 `docs/`
