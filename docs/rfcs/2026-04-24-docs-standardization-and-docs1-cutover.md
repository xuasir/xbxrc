# Docs 标准化与 docs1 Cutover RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 已完成
- Current State: completed
- Owner: codex
- Last Updated: 2026-04-24

## Background

- 仓库当前同时存在 `docs/` 与 `docs1/` 两套文档空间。
- `docs/` 已承接部分新主线追踪与 RFC，`docs1/` 仍保存完整的 RFC、Report、archive、ISU 与参考文档。
- 新的 skills 体系已经把任务入口、脑暴、历史检索与后台维护分层清楚，需要一个统一的文档根目录来承接这些能力。

## Goal

- 将 `docs/` 收敛为唯一文档根目录。
- 把 `docs1/` 的有效内容迁移到标准目录结构。
- 统一 `project-task`、RFC、Report、ISU、archive 与参考文档的存放方式。
- 让新的 skills 与 `AGENTS.md` 面向同一套目录模型工作。

## Scope

- In scope:
  - 迁移 `docs1/isu/`、`docs1/rfcs/`、`docs1/reports/`
  - 迁移 `docs1/project-task.md` 与 `docs1/project-task.archived.*.md`
  - 为 `docs/` 增加 `README.md`、`references/`、`test-assets/`
  - 收敛若干 `docs1/*.md` 参考文档到标准目录
  - 修正少量关键旧引用
- Out of scope:
  - 全量修复所有历史 RFC/Report 内的绝对路径链接
  - 重写历史文档内容与术语
  - 对历史任务做再次归档重排

## Plan

1. 定义标准目录结构并创建缺失目录
2. 将 `docs1/` 下有效文档迁移到 `docs/`
3. 以 `docs1/project-task.md` 为主，收敛 `docs/project-task.md` 为统一追踪入口
4. 新增 `docs/README.md`，明确文档语义与目录用途
5. 修正关键旧引用，并在任务完成后补 Report

## Validation

- [x] `docs/` 成为唯一文档根目录，包含标准子目录
- [x] `docs/project-task.md` 可作为统一任务追踪入口
- [x] `docs/rfcs/`、`docs/reports/`、`docs/isu/` 内容完整可见
- [x] 关键旧引用不再依赖 `docs1/`
- [x] `docs1/` 目录已移除

## Risks

- 历史文档中的绝对路径引用较多，无法在本轮完全收敛
- 部分零散文档的最佳归属并不天然唯一，需要采用“参考文档”收口策略

## Progress

- [x] Step 1: 明确标准目录与迁移策略
- [x] Step 2: 执行目录迁移
- [x] Step 3: 收敛主入口与 README
- [x] Step 4: 修正引用并补 Report

## Execution Notes

- Date: 2026-04-24 | Status: in-progress
- Update: 确认 `docs/` 作为唯一根目录，标准结构采用 `project-task / archived / isu / rfcs / reports / references / test-assets / README`。
- Decision: `docs1/project-task.md` 作为更完整的现行台账基线，迁入后覆盖旧的 `docs/project-task.md`。
- Risk/Blocker: 历史文档内存在大量绝对路径与少量 `docs/todo.md` 旧引用，本轮只修正关键路径并把其余保留为历史记录。

- Date: 2026-04-24 | Status: completed
- Update: 已完成目录迁移、README 补齐、`project-task` 收敛、Report 创建与关键 `docs/todo.md` 旧引用修正。
- Decision: 零散文档统一收敛到 `docs/references/`，避免为少量参考文档引入额外目录层级。
- Risk/Blocker: `docs1/` 已退出；历史绝对路径链接保留到后续低风险清理轮次。
