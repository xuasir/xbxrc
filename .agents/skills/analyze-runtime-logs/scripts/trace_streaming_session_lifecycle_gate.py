#!/usr/bin/env python3
"""Streaming operation/resource/recovery gate. Exit 0=PASS, 2=FAIL or missing evidence."""
from __future__ import annotations

import argparse
import json
import sys
import time
from collections import Counter
from pathlib import Path

READY_PHASES = {"ready", "sessionReady", "playing", "started"}
TERMINAL_PHASES = {"closed", "failed", "Closed", "Failed"}
TERMINALS = {"operationFinished", "operationCancelled", "operationTimedOut"}
RECOVERY_EVENTS = {"recoveryProposed", "recoveryGranted", "recoveryCommitted", "recoveryObserved"}
COMMIT_EVENTS = {"recoveryCommitted", "exchangeOfferResult", "submitIceResult", "pollIceResult", "runtimeLaunchPortReady", "mutationCommitted"}
PREPARATION_STAGES = {"startup", "sessionReady", "sessionPreparation", "waitingSessionReady", "preparation"}


def audit(paths, *, require_recovery=False, require_cleanup=False, require_closed=False,
          allow_active=False, expected_revision=None, expected_source_sha256=None, max_age_seconds=None):
    issues, gaps = [], []
    counts = Counter()
    operations, cleanups, recoveries, resources, startup_sessions = {}, {}, {}, {}, {}
    builds = []
    drains = {}
    rows = []
    run_order = {}

    def issue(code, row=None, **details):
        entry = {"code": code, **details}
        if row:
            entry.update({"file": row.get("_file"), "seq": row.get("seq"), "event": row.get("event")})
        issues.append(entry)

    for path in paths:
        age = time.time() - path.stat().st_mtime
        if max_age_seconds is not None and (age > max_age_seconds or age < -60):
            gaps.append({"code": "staleTrace", "file": str(path), "ageSeconds": round(age, 1)})
        with path.open(encoding="utf-8") as handle:
            for line_number, line in enumerate(handle, 1):
                if not line.strip():
                    continue
                try:
                    row = json.loads(line)
                    if not isinstance(row, dict):
                        raise ValueError("row must be an object")
                except (json.JSONDecodeError, ValueError):
                    issues.append({"code": "invalidJsonl", "file": str(path), "line": line_number})
                    continue
                row["_file"] = str(path)
                rows.append(row)
                run_order.setdefault(row.get("traceRunId", "legacy"), len(run_order))

    # 新 trace 按 recorder 内序号还原跨文件和并发写入顺序；墙钟调整保持独立。
    rows.sort(key=lambda row: (run_order[row.get("traceRunId", "legacy")], row.get("seq", 0)))
    for row in rows:
        name = row.get("event", "")
        payload = row.get("payload") or {}
        if not isinstance(payload, dict):
            continue
        counts[name] += 1
        session = payload.get("sessionId") or row.get("sessionId")
        stage = payload.get("stage")
        op_id = payload.get("operationId") or payload.get("admissionId")
        if name in RECOVERY_EVENTS or name in {"recoveryDrainPending", "recoveryDrainFinished"}:
            stage = "connectionRecovery"
        run = row.get("traceRunId", "legacy")
        resource_key = (run, session)
        key = (run, None if stage == "startup" else session, op_id, stage, payload.get("runtimeGeneration"))
        if name == "runtimeBuildInfo":
            builds.append(payload.get("buildFingerprint", payload))
        if name == "sessionResourceCreated":
            generation = payload.get("generation")
            if not session or generation is None:
                issue("resourceIdentityMissing", row)
            else:
                previous = resources.get(resource_key)
                if previous and not previous["closed"]:
                    issue("overlappingResource", row, generation=generation)
                resources[resource_key] = {"generation": generation, "closed": False, "ready": False, "revoked": False}
                startup_sessions[(run, payload.get("startupGeneration"))] = session
        if name in {"sessionProgress", "sessionMonitorTick", "sessionExecutionStarted"}:
            progress = payload.get("progress", payload)
            phase = progress.get("phase") or progress.get("playerState")
            resource = resources.get(resource_key)
            if resource and phase in READY_PHASES:
                resource["ready"] = True
            if resource and phase in TERMINAL_PHASES:
                resource["terminalObserved"] = True
        if name == "operationStarted":
            if not op_id or not stage:
                issue("operationIdentityMissing", row)
            elif key in operations and not operations[key].get("terminal"):
                issue("overlappingOperation", row, operationId=op_id)
            else:
                operations[key] = {"start": row, "payload": payload, "terminal": None}
        if name in TERMINALS:
            operation = operations.get(key)
            if not operation:
                issue("operationStartMissing", row, operationId=op_id)
            elif operation["terminal"]:
                issue("duplicateOperationTerminal", row, operationId=op_id)
            else:
                operation["terminal"] = row
                deadline = operation["payload"].get("deadlineMs")
                elapsed = payload.get("elapsedMs")
                if isinstance(deadline, (int, float)) and isinstance(elapsed, (int, float)) and elapsed > deadline + 2_000:
                    issue("operationDeadlineExceeded", row, operationId=op_id, elapsedMs=elapsed, deadlineMs=deadline)
            associated = session or startup_sessions.get((run, payload.get("generation")))
            if name == "operationTimedOut" and stage in PREPARATION_STAGES and resources.get((run, associated), {}).get("ready"):
                issue("startupTimeoutAfterReady", row)
        if name in RECOVERY_EVENTS:
            if not op_id:
                issue("recoveryIdentityMissing", row)
                continue
            recovery = recoveries.setdefault(key, {"proposed": 0, "granted": 0, "committed": 0, "observed": 0})
            field = name.removeprefix("recovery").lower()
            if field == "granted" and recovery["proposed"] != 1:
                issue("grantWithoutProposal", row)
            if field == "committed":
                if recovery["granted"] != 1:
                    issue("commitWithoutGrant", row)
                if recovery["committed"] and payload.get("action") != "runtimeRebuild":
                    issue("duplicateRecoveryCommit", row)
                if recovery["committed"] >= 2:
                    issue("recoveryActionBudgetExceeded", row)
            if field == "observed" and (recovery["committed"] == 0 or payload.get("result") != "stableMedia"):
                issue("recoveryEffectUnproven", row)
            recovery[field] += 1
            if field in {"proposed", "granted", "observed"} and recovery[field] > 1:
                issue("duplicateRecoveryStage", row, stage=field)
        if name == "cleanupStarted":
            if not op_id:
                issue("cleanupIdentityMissing", row)
            elif key in cleanups and not cleanups[key].get("finished"):
                issue("duplicateCleanupStarted", row)
            else:
                cleanups[key] = {"start": row, "stage": stage, "finished": False, "pending": False}
            resource = resources.get(resource_key)
            if resource and stage == "remoteCleanup":
                if payload.get("generation") is not None and payload["generation"] != resource["generation"]:
                    issue("oldCleanupGeneration", row, generation=payload["generation"])
                else:
                    resource["revoked"] = True
        if name in {"cleanupFinished", "cleanupPending"}:
            generation = payload.get("generation")
            resource = resources.get(resource_key)
            stale_cleanup = stage == "remoteCleanup" and resource and generation is not None and generation != resource["generation"]
            if stale_cleanup:
                issue("oldCleanupGeneration", row, generation=generation)
            cleanup = cleanups.get(key)
            if not cleanup:
                issue("cleanupStartMissing", row, operationId=op_id)
            elif name == "cleanupFinished":
                if cleanup["finished"]:
                    issue("duplicateCleanupFinished", row)
                cleanup["finished"] = True
                if stage == "remoteCleanup" and resource_key in resources and not stale_cleanup:
                    resources[resource_key]["closed"] = True
            else:
                cleanup["pending"] = True
        if name == "recoveryDrainPending":
            drains[key] = row
        if name == "recoveryDrainFinished":
            if key not in drains:
                issue("recoveryDrainStartMissing", row)
            drains.pop(key, None)
        if name == "createResultUnknown":
            issue("createOutcomeUnresolved", row)
        if name in COMMIT_EVENTS:
            if payload.get("isCurrent") is False or payload.get("current") is False:
                issue("staleOperationCommitted", row)
            if resources.get(resource_key, {}).get("revoked"):
                issue("commitAfterResourceRevocation", row)
            if op_id and operations.get(key, {}).get("terminal"):
                issue("commitAfterOperationTerminal", row)
            generation = payload.get("generation")
            if generation is not None and resource_key in resources and generation != resources[resource_key]["generation"]:
                issue("oldGenerationCommitted", row, generation=generation)
    if not allow_active:
        for key, row in drains.items():
            issue("recoveryActionDrainUnfinished", row, operationId=key[2])
    for key, operation in operations.items():
        if not operation["terminal"] and not allow_active:
            issue("operationUnfinished", operation["start"], operationId=key[2])
    for key, recovery in recoveries.items():
        operation = operations.get(key)
        if not operation:
            issues.append({"code": "recoveryOperationMissing", "operationId": key[2]})
            continue
        terminal = operation.get("terminal")
        if recovery["granted"] and not recovery["committed"] and terminal and terminal.get("event") != "operationCancelled":
            issue("grantedRecoveryNeverExecuted", terminal, operationId=key[2])
        if terminal and terminal.get("payload", {}).get("result") == "observed" and recovery["observed"] != 1:
            issue("recoveryObservationMissing", terminal, operationId=key[2])
    for key, cleanup in cleanups.items():
        if not cleanup["finished"] and not allow_active:
            issue("cleanupUnfinished", cleanup["start"], operationId=key[2], pending=cleanup["pending"])
    if not operations:
        gaps.append({"code": "commonLifecycleEvidenceMissing"})
    if require_recovery and not recoveries:
        gaps.append({"code": "recoveryEvidenceMissing"})
    if require_cleanup and not cleanups:
        gaps.append({"code": "cleanupEvidenceMissing"})
    if require_closed:
        if not resources:
            gaps.append({"code": "resourceEvidenceMissing"})
        for (run, session), resource in resources.items():
            if not resource["closed"]:
                issues.append({"code": "remoteResourceUnclosed", "sessionId": session, "generation": resource["generation"]})
    if expected_revision:
        if not builds:
            gaps.append({"code": "buildEvidenceMissing"})
        for build in builds:
            revision = build.get("gitCommitShort") or build.get("git_commit_short") or build.get("revision")
            if not revision or not expected_revision.startswith(str(revision)):
                gaps.append({"code": "buildRevisionMismatch", "expected": expected_revision, "actual": revision})
    if expected_source_sha256:
        if not builds:
            gaps.append({"code": "buildEvidenceMissing"})
        for build in builds:
            actual = build.get("source_fingerprint_sha256") or build.get("sourceFingerprintSha256")
            if actual != expected_source_sha256:
                gaps.append({"code": "buildSourceMismatch", "expected": expected_source_sha256, "actual": actual})
    return {"lifecycleGate": "PASS" if not issues and not gaps else "FAIL", "files": [str(path) for path in paths],
            "scope": "activeInspection" if allow_active else "completeLifecycle",
            "summary": {"pendingActionDrains": len(drains), "operations": len(operations), "recoveries": len(recoveries), "cleanups": len(cleanups), "resources": len(resources)},
            "violations": issues, "evidenceGaps": gaps, "eventCounts": dict(counts)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("traces", type=Path, nargs="*")
    parser.add_argument("--latest", action="store_true")
    parser.add_argument("--max-age-seconds", type=float)
    parser.add_argument("--expected-revision")
    parser.add_argument("--expected-source-sha256")
    parser.add_argument("--require-recovery", action="store_true")
    parser.add_argument("--require-cleanup", action="store_true")
    parser.add_argument("--require-closed", action="store_true")
    parser.add_argument("--allow-active", action="store_true", help="inspect an active capture; unfinished resources remain outside closure proof")
    args = parser.parse_args()
    paths = args.traces
    if args.latest and not paths:
        candidates = list(Path("runtime-logs").glob("runtime-trace-*.jsonl"))
        if candidates:
            paths = [max(candidates, key=lambda path: path.stat().st_mtime)]
    if not paths or any(not path.is_file() for path in paths):
        print(json.dumps({"lifecycleGate": "FAIL", "evidenceGaps": [{"code": "traceMissing"}]}))
        return 2
    report = audit(paths, require_recovery=args.require_recovery, require_cleanup=args.require_cleanup,
                   require_closed=args.require_closed, allow_active=args.allow_active,
                   expected_revision=args.expected_revision, expected_source_sha256=args.expected_source_sha256, max_age_seconds=args.max_age_seconds)
    print(json.dumps(report, ensure_ascii=False, indent=2))
    return 0 if report["lifecycleGate"] == "PASS" else 2


if __name__ == "__main__":
    sys.exit(main())
