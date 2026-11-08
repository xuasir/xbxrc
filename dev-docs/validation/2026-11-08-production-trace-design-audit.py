"""复算生产 trace 的证据密度和控制面压缩方案；输出安全汇总。"""
from __future__ import annotations

import argparse
import collections
import hashlib
import json
from pathlib import Path


def control_replay(rows: list[dict], heartbeat_ms: int) -> dict:
    monitors = [row for row in rows if row.get("event") == "sessionMonitorSnapshot"]
    groups: dict[str, list[dict]] = collections.defaultdict(list)
    for row in monitors:
        payload = row.get("payload", {})
        identity = row.get("sessionId") or payload.get("sessionId")
        groups[json.dumps(identity, sort_keys=True)].append(row)

    latest_polled: dict[str, dict] = {}
    pairs = 0
    pair_state_mismatches = 0
    for row in rows:
        payload = row.get("payload", {})
        identity = row.get("sessionId") or payload.get("sessionId")
        key = json.dumps(identity, sort_keys=True)
        if row.get("event") == "sessionStatePolled":
            latest_polled[key] = payload
        elif row.get("event") == "sessionMonitorSnapshot" and key in latest_polled:
            pairs += 1
            pair_state_mismatches += int(latest_polled[key].get("state") != payload.get("streamState"))

    retained: list[int] = []
    transitions = 0
    fields = (
        "targetType", "phase", "runtimeLaunchState", "statusTextKey", "streamState",
        "playerState", "errorCode", "shouldContinue", "shouldSendConnectToken",
    )
    for group in groups.values():
        previous_signature = None
        emitted_at = None
        last_retained = None
        for row in group:
            payload = row.get("payload", {})
            state = {key: payload.get(key) for key in fields}
            message = payload.get("errorMessage")
            state["errorMessageHash"] = hashlib.sha256(str(message).encode()).hexdigest()
            signature = json.dumps(state, sort_keys=True)
            changed = previous_signature is not None and signature != previous_signature
            transitions += int(changed)
            if emitted_at is None or changed or row["tsMs"] - emitted_at >= heartbeat_ms:
                retained.append(row["seq"])
                emitted_at = row["tsMs"]
                last_retained = row["seq"]
            previous_signature = signature
        if group and group[-1]["seq"] != last_retained:
            retained.append(group[-1]["seq"])

    polled = sum(row.get("event") == "sessionStatePolled" for row in rows)
    original_rows = len(monitors) + polled
    return {
        "kind": "proposal-replay-of-existing-control-state",
        "measurementBoundary": "Replay models first/state-change/15s heartbeat/final boundary retention. Production emitters remain unchanged.",
        "heartbeatMs": heartbeat_ms,
        "sessionGroups": len(groups),
        "originalPollingRows": original_rows,
        "polledAndMonitorPairs": pairs,
        "polledAndMonitorStateMismatches": pair_state_mismatches,
        "semanticMonitorTransitions": transitions,
        "projectedCombinedControlRows": len(retained),
        "projectedPollingRowsRemoved": original_rows - len(retained),
        "projectedPollingRowReductionPercent": round(100 * (1 - len(retained) / original_rows), 2) if original_rows else None,
        "retainedMonitorSeqs": retained,
        "finalBoundaryAssumption": "Final boundary is a design replay assumption; the supplied file ends at cleanupStarted.",
    }


def audit(trace_path: Path, root: Path) -> dict:
    data = trace_path.read_bytes()
    lines = [line for line in data.splitlines(keepends=True) if line.strip()]
    rows = [json.loads(line) for line in lines]
    if not rows:
        raise ValueError("Trace has zero JSONL rows")
    counts = collections.Counter(row.get("event", "") for row in rows)
    event_bytes: collections.Counter[str] = collections.Counter()
    for row, line in zip(rows, lines):
        event_bytes[row.get("event", "")] += len(line)
    control_events = ("sessionStatePolled", "sessionMonitorSnapshot")
    control_rows = sum(counts[event] for event in control_events)
    control_bytes = sum(event_bytes[event] for event in control_events)
    max_file_bytes = rows[0].get("payload", {}).get("budget", {}).get("maxFileBytes")
    sources = (
        "src/streaming/runtime/browser-runtime-trace.ts",
        "src/streaming/runtime/browser-runtime.ts",
        "src/player/app/stats/StatsService.ts",
        "src-tauri/src/mods/runtime_trace/policy.rs",
        "src-tauri/src/mods/runtime_trace/rpc.rs",
        "src-tauri/src/mods/streaming/service.rs",
        "src/navigation/core/vue/Focusable.vue",
    )
    return {
        "date": "2026-11-08",
        "trace": {
            "fileName": trace_path.name,
            "sha256": hashlib.sha256(data).hexdigest(),
            "bytes": len(data),
            "rows": len(rows),
            "durationSeconds": round((rows[-1]["tsMs"] - rows[0]["tsMs"]) / 1000, 3),
            "profileCounts": dict(collections.Counter(row.get("traceProfile") for row in rows)),
            "maxFileBytes": max_file_bytes,
            "fileBudgetUtilizationPercent": round(100 * len(data) / max_file_bytes, 2) if max_file_bytes else None,
            "traceBudgetNotices": counts["traceBudgetNotice"],
            "lastEvent": rows[-1].get("event"),
        },
        "controlPolling": {
            "rows": control_rows,
            "rowPercent": round(100 * control_rows / len(rows), 2),
            "bytes": control_bytes,
            "bytePercent": round(100 * control_bytes / len(data), 2),
            "rowsWithPayloadSessionIdAndEmptyEnvelope": sum(
                row.get("event") in control_events
                and bool(row.get("payload", {}).get("sessionId"))
                and not row.get("sessionId") for row in rows
            ),
        },
        "topEventsByBytes": [
            {"event": event, "rows": counts[event], "bytes": size}
            for event, size in event_bytes.most_common(15)
        ],
        "periodicEvidence": {event: counts[event] for event in (
            "browserWebRtcStatsObserved", "renderTelemetryObserved", "browserPlaybackHealthObserved",
        )},
        "controlCompactionProposal": control_replay(rows, 15_000),
        "proposedBudgets": {
            "measurementBoundary": "Design targets; producer and writer implementation validation is pending.",
            "steadySummaryIntervalMs": 1000,
            "steadySummaryMaxRowBytes": 4096,
            "steadySummaryMaxBytesPerMinute": 60 * 4096,
            "normalSteadyTotalTargetBytesPerMinute": 320 * 1024,
            "inputAdditionalMaxBytesPerMinute": 64 * 1024,
            "anomalyAdditionalMaxBytesPerMinute": 128 * 1024,
            "anomalyRingSamples": 60,
            "anomalyRingMaxPayloadBytes": 60 * 1024,
            "anomalySampleMaxRowBytes": 2048,
            "anomalyWindowBeforeMs": 5000,
            "anomalyWindowAfterMs": 10000,
            "anomalyMinEpisodeIntervalMs": 60000,
            "productionFileBytes": 16 * 1024 * 1024,
            "productionFileCount": 5,
        },
        "sourceSha256": {source: hashlib.sha256((root / source).read_bytes()).hexdigest() for source in sources},
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("trace", type=Path)
    parser.add_argument("--root", type=Path, default=Path.cwd())
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    report = audit(args.trace, args.root)
    text = json.dumps(report, ensure_ascii=False, indent=2) + "\n"
    if args.output:
        args.output.write_text(text)
    print(text, end="")


if __name__ == "__main__":
    main()
