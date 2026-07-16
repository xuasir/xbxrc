---
name: analyze-ios-runtime-trace
description: Analyze XBXRC iOS Runtime Trace JSONL files and exported bundles. Use when Codex needs to verify iOS trace schema v3, reconstruct App/auth/cache/cloud-library/image/UI timelines, audit operationId pairing, check production/dev file budgets and retention, detect queue-pressure loss, or scan trace payloads for tokens, handles, account identities, title IDs, and full URLs.
---

# Analyze iOS Runtime Trace

Use this skill for Swift-owned `runtime-trace-ios-*.jsonl` files and `XBXRC-iOS-Trace-*.jsonl` exports. Keep this workflow independent from Rust and desktop runtime trace analysis.

## Quick Start

1. Locate the user-provided JSONL file or the directory containing raw iOS trace files.
2. Run:

   ```bash
   python3 -B scripts/analyze_ios_runtime_trace.py <trace-or-directory> --strict --require-flow all
   ```

3. Read `gate`, `schema`, `sequence`, `privacy`, `budget`, `coverage`, and `pairing` before investigating individual errors.
4. Re-open raw rows around the reported `seq` and `tsMs` anchors before claiming causality.
5. Read [`references/ios-trace-contract.md`](references/ios-trace-contract.md) for field semantics, budgets, required flows, and privacy rules.

All resource paths are relative to this `SKILL.md` directory.

## Analysis Workflow

1. Confirm the scope: startup failure, login failure, game-library blank screen, cache behavior, metadata hydration, image fallback, UI presentation, or file-budget audit.
2. Run the JSONL analyzer on every relevant raw file together. Prefer the raw `runtime-trace-ios-*` directory when checking physical file sizes and retention.
3. Use `--session-id <id>` to isolate one launch when an export contains multiple sessions.
4. Use `--require-flow startup`, `library`, or `all` to turn missing critical anchors into gate failures.
5. Treat `traceBudgetNotice` as evidence that debug/raw rows were dropped. Essential/key rows remain the reliable lane.
6. Reconstruct the earliest abnormal transition with `operationId`, `generation`, `pageIndex`, `seq`, and `tsMs`.
7. Separate evidence from hypotheses. Report missing anchors as instrumentation gaps.

## Critical Timelines

- Startup/auth: `appLaunchStarted -> authRestoreStarted -> authRestoreSucceeded|authRestoreFailed`
- Cloud access: `cloudAccessBoundaryStarted -> cloudAccessBoundarySucceeded|cloudAccessBoundaryFailed`
- Cache: `cacheRestoreStarted -> cacheRestoreHit|cacheRestoreMiss|cacheRestoreRejected|cacheRestoreFailed|cacheRestoreSkipped`
- Catalog: `catalogRefreshStarted -> catalogRefreshCommitted|catalogRefreshFailed|catalogRefreshCancelled|catalogRefreshDiscarded`
- Metadata page: `metadataPageStarted -> metadataPageCommitted|metadataPageUnchanged|metadataPageFailed|metadataPageCancelled|metadataPageDiscarded`
- UI: `libraryPageAppeared -> skeletonPresented|contentPresented`
- Image fallback: `imageCandidateStarted -> imageCandidateSucceeded|imageCandidateFailed -> imageCandidatesExhausted|preferredImageUpdated`

## Reporting Contract

Return results in this order:

1. Scope: files, sessions, row count, time range, active profiles.
2. Gate: PASS/FAIL and exact violations.
3. Timeline: smallest useful chain with `seq`, `tsMs`, domain, event, and operationId.
4. Coverage: observed critical flows and missing anchors.
5. Budget: physical file size, configured limit, retention count, and pressure notices.
6. Privacy: sensitive-key, raw URL, callback, bearer, or identity violations.
7. Cause: highest-confidence explanation supported by trace rows.
8. Gaps and next action: missing instrumentation or experiment required.

## Guardrails

- Treat raw rotating files as the authority for file-size and retention gates. Treat `XBXRC-iOS-Trace-*` as an aggregate export whose physical size can exceed one writer-file budget.
- Validate `seq` within each `sessionId`; a multi-launch export can restart sequence numbers across sessions.
- Accept a truncated final JSONL line as a recoverable tail. Count malformed complete lines as corruption.
- Keep production evidence expectations to essential/key events. Dev traces can contain debug/raw detail.
- Do not expose sensitive values found in a trace. Report the file, line, field class, and violation type only.
