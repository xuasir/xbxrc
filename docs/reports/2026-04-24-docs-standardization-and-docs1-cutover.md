# Docs 标准化与 docs1 Cutover Report

> 说明：本 Report 仅用于复杂任务完全完成后的最终总结，不用于记录执行中的阶段性进度；中间过程应持续回写到对应 RFC。

## Summary

- Related RFC: [`docs/rfcs/2026-04-24-docs-standardization-and-docs1-cutover.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-04-24-docs-standardization-and-docs1-cutover.md)
- 本任务已完成：将仓库文档根目录统一收敛到 `docs/`，把 `docs1/` 中的 RFC、Report、archive、ISU、参考文档与测试文档迁入标准目录，并补齐新的 `docs/README.md` 与统一 `project-task.md` 入口。

## Delivered

- `docs/` 成为唯一文档根目录，标准结构收敛为 `project-task / archived / isu / rfcs / reports / references / test-assets / README`。
- `docs1/rfcs/`、`docs1/reports/`、`docs1/isu/`、`docs1/project-task.archived.*.md`、`docs1/test-assets/` 已全部迁入 `docs/`。
- 零散参考文档已收敛到 `docs/references/`。

## Changes

- 以 `docs1/project-task.md` 为现行任务台账基线，重写 [`docs/project-task.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/project-task.md)，保留进行中任务并合并近期完成项。
- 新增 [`docs/README.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/README.md) 作为统一目录说明。
- 修正 `docs/todo.md` 旧引用，改为 `docs/references/sdl3-cutover-notes.md`。

## Validation

- 人工核对 `docs/` 目录，确认 `isu/`、`rfcs/`、`reports/`、`references/`、`test-assets/`、archive 文件与 `project-task.md` 已齐备。
- 人工核对 `docs1/`，确认仅剩待删除的空目录与旧 `project-task.md` 壳层。
- 使用 `rg -n "docs/todo\\.md|docs1/" docs -g '*.md'` 复查关键旧引用。

## Risks

- 历史 RFC 与 Report 内仍保留部分绝对路径链接，本轮保持原样以避免大规模无关 churn。
- 早期归档快照中的表述和目录引用保留历史状态，后续按需清理。

## Follow-up

- 继续把新增复杂任务统一落到 `docs/rfcs/` 与 `docs/reports/`。
- 未来如需要进一步降 churn，可批量把历史文档中的绝对路径链接收敛成相对路径。
