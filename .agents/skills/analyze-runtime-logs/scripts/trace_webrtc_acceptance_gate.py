#!/usr/bin/env python3
"""Combined WebRTC acceptance gate for receive, latency, and reconnect regressions."""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
import time
from pathlib import Path
from typing import Any


SCRIPT_DIR = Path(__file__).resolve().parent
RECEIVE_REPORT = SCRIPT_DIR / "trace_receive_feedback_report.py"
MIDSEGMENT_REPORT = SCRIPT_DIR / "trace_midsegment_report.py"
LIFECYCLE_RECONNECT_REPORT = SCRIPT_DIR / "trace_lifecycle_reconnect_gate.py"
DEFAULT_RUNTIME_LOG_DIR = Path("runtime-logs")
INGRESS_QUEUE_EVENTS = {
    "frameDropped",
    "pacerCandidateDecision",
    "renderMailboxDecision",
}


def run_command(args: list[str]) -> subprocess.CompletedProcess[str]:
    return subprocess.run(args, capture_output=True, text=True, check=False)


def find_latest_trace(runtime_log_dir: Path) -> Path | None:
    traces = sorted(
        runtime_log_dir.glob("runtime-trace-*.jsonl"),
        key=lambda path: path.stat().st_mtime,
        reverse=True,
    )
    return traces[0] if traces else None


def trace_freshness_report(trace: Path, max_age_seconds: float | None) -> dict[str, Any]:
    mtime_seconds = trace.stat().st_mtime
    age_seconds = max(0.0, time.time() - mtime_seconds)
    accepted = max_age_seconds is None or age_seconds <= max_age_seconds
    return {
        "mtimeSeconds": round(mtime_seconds, 3),
        "ageSeconds": round(age_seconds, 3),
        "maxAgeSeconds": max_age_seconds,
        "freshnessGate": "PASS" if accepted else "FAIL",
    }


def load_events(trace: Path) -> list[dict[str, Any]]:
    events: list[dict[str, Any]] = []
    with trace.open(encoding="utf-8") as handle:
        for line in handle:
            line = line.strip()
            if not line:
                continue
            try:
                event = json.loads(line)
            except json.JSONDecodeError:
                continue
            if isinstance(event, dict):
                events.append(event)
    return events


def numeric_field(payload: dict[str, Any], *names: str) -> float | None:
    for name in names:
        value = payload.get(name)
        if isinstance(value, (int, float)):
            return float(value)
    return None


def ingress_breakdown_total_depth(payload: dict[str, Any]) -> float:
    total = 0.0
    for camel_name, snake_name in (
        ("senderQueueDepth", "sender_queue_depth"),
        ("pendingPriorityPrimaryLen", "pending_priority_primary_len"),
        ("pendingRepairLen", "pending_repair_len"),
        ("pendingBestEffortLen", "pending_best_effort_len"),
    ):
        total += numeric_field(payload, camel_name, snake_name) or 0.0
    return total


def ingress_queue_gate(
    trace: Path,
    *,
    require_breakdown: bool,
    max_sender_queue_limit: int,
    max_sender_queue_depth: int,
    max_total_queue_depth: int,
    max_best_effort_overflow_streak: int,
) -> dict[str, Any]:
    samples: list[dict[str, Any]] = []
    failures: list[str] = []
    best_effort_streak = 0
    max_best_effort_streak = 0
    max_sender_limit = 0.0
    max_sender_depth = 0.0
    max_total_depth = 0.0
    breakdown_count = 0

    for event in load_events(trace):
        name = event.get("event") or event.get("name") or ""
        if name not in INGRESS_QUEUE_EVENTS:
            continue
        payload = event.get("payload") if isinstance(event.get("payload"), dict) else event
        if not isinstance(payload, dict):
            continue

        reason = str(payload.get("reason") or "")
        if reason == "localBackpressureBestEffortOverflow":
            best_effort_streak += 1
            max_best_effort_streak = max(max_best_effort_streak, best_effort_streak)
        else:
            best_effort_streak = 0

        breakdown = payload.get("ingressQueueDepthBreakdown")
        if not isinstance(breakdown, dict):
            breakdown = payload.get("ingress_queue_depth_breakdown")
        if not isinstance(breakdown, dict):
            continue

        breakdown_count += 1
        sender_limit = numeric_field(
            breakdown, "senderQueueLimit", "sender_queue_limit"
        ) or 0.0
        sender_depth = numeric_field(
            breakdown, "senderQueueDepth", "sender_queue_depth"
        ) or 0.0
        total_depth = max(
            ingress_breakdown_total_depth(breakdown),
            numeric_field(payload, "queueDepth", "queue_depth") or 0.0,
        )
        max_sender_limit = max(max_sender_limit, sender_limit)
        max_sender_depth = max(max_sender_depth, sender_depth)
        max_total_depth = max(max_total_depth, total_depth)
        if sender_limit > max_sender_queue_limit:
            failures.append(
                f"senderQueueLimit {sender_limit:.0f} > {max_sender_queue_limit}"
            )
        if sender_depth > max_sender_queue_depth:
            failures.append(
                f"senderQueueDepth {sender_depth:.0f} > {max_sender_queue_depth}"
            )
        if total_depth > max_total_queue_depth:
            failures.append(f"queueDepth {total_depth:.0f} > {max_total_queue_depth}")
        if len(samples) < 5:
            samples.append(
                {
                    "seq": event.get("seq"),
                    "event": name,
                    "reason": reason or None,
                    "senderQueueLimit": int(sender_limit),
                    "senderQueueDepth": int(sender_depth),
                    "totalQueueDepth": int(total_depth),
                }
            )

    if require_breakdown and breakdown_count == 0:
        failures.append("missing ingressQueueDepthBreakdown")
    if max_best_effort_streak > max_best_effort_overflow_streak:
        failures.append(
            "localBackpressureBestEffortOverflow streak "
            f"{max_best_effort_streak} > {max_best_effort_overflow_streak}"
        )

    return {
        "ingressQueueGate": "FAIL" if failures else "PASS",
        "failures": sorted(set(failures)),
        "breakdownEvents": breakdown_count,
        "maxSenderQueueLimit": int(max_sender_limit),
        "maxSenderQueueDepth": int(max_sender_depth),
        "maxTotalQueueDepth": int(max_total_depth),
        "maxBestEffortOverflowStreak": max_best_effort_streak,
        "samples": samples,
    }


def parse_midsegment_gate(stdout: str, label: str) -> str | None:
    prefix = f"{label}: "
    for line in stdout.splitlines():
        if line.startswith(prefix):
            return line[len(prefix) :].strip()
    return None


def run_receive_gate(trace: Path) -> tuple[int, dict[str, Any]]:
    result = run_command(
        [
            sys.executable,
            "-B",
            str(RECEIVE_REPORT),
            "--fail-on-gate",
            "--require-media-recovered",
            "--require-display-stable",
            str(trace),
        ]
    )
    try:
        report = json.loads(result.stdout)
    except json.JSONDecodeError:
        report = {"rawStdout": result.stdout, "stderr": result.stderr}
    return result.returncode, report


def run_midsegment_gate(
    trace: Path, start_s: float | None, end_s: float | None
) -> tuple[int, dict[str, Any]]:
    command = [
        sys.executable,
        "-B",
        str(MIDSEGMENT_REPORT),
        str(trace),
    ]
    if start_s is not None:
        command.extend(["--start-s", str(start_s)])
    if end_s is not None:
        command.extend(["--end-s", str(end_s)])
    result = run_command(command)
    report = {
        "globalLatencyGate": parse_midsegment_gate(stdout=result.stdout, label="GLOBAL_LATENCY_GATE"),
        "mediaSupplyGate": parse_midsegment_gate(stdout=result.stdout, label="MEDIA_SUPPLY_GATE"),
        "steadySupplyGate": parse_midsegment_gate(stdout=result.stdout, label="STEADY_SUPPLY_GATE"),
        "stderr": result.stderr,
    }
    return result.returncode, report


def run_lifecycle_reconnect_gate(
    trace: Path,
    max_age_seconds: float | None,
    allow_rebuilds_after_healthy: int,
) -> tuple[int, dict[str, Any]]:
    command = [
        sys.executable,
        "-B",
        str(LIFECYCLE_RECONNECT_REPORT),
        "--require-lifecycle-block",
        "--allow-rebuilds-after-healthy",
        str(allow_rebuilds_after_healthy),
    ]
    if max_age_seconds is not None:
        command.extend(["--max-age-seconds", str(max_age_seconds)])
    command.append(str(trace))
    result = run_command(command)
    try:
        report = json.loads(result.stdout)
    except json.JSONDecodeError:
        report = {"rawStdout": result.stdout, "stderr": result.stderr}
    return result.returncode, report


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("trace", nargs="?", type=Path, help="Path to runtime-trace-*.jsonl")
    parser.add_argument(
        "--latest",
        action="store_true",
        help="use the newest runtime-trace-*.jsonl from --runtime-log-dir",
    )
    parser.add_argument(
        "--runtime-log-dir",
        type=Path,
        default=DEFAULT_RUNTIME_LOG_DIR,
        help="directory used by --latest",
    )
    parser.add_argument(
        "--max-age-seconds",
        type=float,
        default=None,
        help="fail unless the selected trace file was modified within this many seconds",
    )
    parser.add_argument(
        "--start-s",
        type=float,
        default=None,
        help="manual midsegment start seconds; default auto-anchors at first steady statsSnapshot after +79s",
    )
    parser.add_argument(
        "--end-s",
        type=float,
        default=None,
        help="manual midsegment end seconds; default keeps the midsegment script's auto window",
    )
    parser.add_argument(
        "--require-lifecycle-reconnect-gate",
        action="store_true",
        help="fail unless the trace passes the healthy-network lifecycle reconnect gate",
    )
    parser.add_argument(
        "--allow-rebuilds-after-healthy",
        type=int,
        default=0,
        help="allowed rebuildPeerConnection closures after healthy playback when lifecycle gate is enabled",
    )
    parser.add_argument(
        "--require-ingress-queue-gate",
        action="store_true",
        help="fail unless ingress queue breakdown evidence stays within low-latency bounds",
    )
    parser.add_argument(
        "--max-sender-queue-limit",
        type=int,
        default=64,
        help="maximum allowed ingress sender queue limit when ingress queue gate is required",
    )
    parser.add_argument(
        "--max-sender-queue-depth",
        type=int,
        default=64,
        help="maximum allowed ingress sender queue depth when ingress queue gate is required",
    )
    parser.add_argument(
        "--max-total-queue-depth",
        type=int,
        default=96,
        help="maximum allowed total ingress queue depth when ingress queue gate is required",
    )
    parser.add_argument(
        "--max-best-effort-overflow-streak",
        type=int,
        default=60,
        help="maximum consecutive localBackpressureBestEffortOverflow rows",
    )
    args = parser.parse_args()

    trace = args.trace
    if args.latest:
        trace = find_latest_trace(args.runtime_log_dir)
        if trace is None:
            print(f"no runtime trace found in: {args.runtime_log_dir}", file=sys.stderr)
            return 1
    if trace is None:
        print("trace path required unless --latest is set", file=sys.stderr)
        return 1
    if not trace.is_file():
        print(f"trace not found: {trace}", file=sys.stderr)
        return 1

    freshness = trace_freshness_report(trace, args.max_age_seconds)
    freshness_ok = freshness["freshnessGate"] == "PASS"
    receive_code, receive_report = run_receive_gate(trace)
    midsegment_code, midsegment_report = run_midsegment_gate(
        trace, args.start_s, args.end_s
    )
    lifecycle_code: int | None = None
    lifecycle_report: dict[str, Any] | None = None
    if args.require_lifecycle_reconnect_gate:
        lifecycle_code, lifecycle_report = run_lifecycle_reconnect_gate(
            trace,
            args.max_age_seconds,
            args.allow_rebuilds_after_healthy,
        )
    lifecycle_ok = lifecycle_code is None or lifecycle_code == 0
    ingress_report = ingress_queue_gate(
        trace,
        require_breakdown=args.require_ingress_queue_gate,
        max_sender_queue_limit=args.max_sender_queue_limit,
        max_sender_queue_depth=args.max_sender_queue_depth,
        max_total_queue_depth=args.max_total_queue_depth,
        max_best_effort_overflow_streak=args.max_best_effort_overflow_streak,
    )
    ingress_ok = (
        not args.require_ingress_queue_gate
        or ingress_report.get("ingressQueueGate") == "PASS"
    )
    accepted = (
        freshness_ok
        and receive_code == 0
        and midsegment_code == 0
        and lifecycle_ok
        and ingress_ok
    )
    report = {
        "trace": str(trace),
        "acceptanceGate": "PASS" if accepted else "FAIL",
        "traceFreshness": freshness,
        "receive": {
            "exitCode": receive_code,
            "receiveFeedbackGate": receive_report.get("receiveFeedbackGate"),
            "receiveFeedbackGateFailures": receive_report.get("receiveFeedbackGateFailures"),
            "controlKeyframeRequests": receive_report.get("controlKeyframeRequests"),
            "keyframeChain": receive_report.get("keyframeChain"),
            "rates": receive_report.get("rates"),
        },
        "midsegment": {
            "exitCode": midsegment_code,
            **midsegment_report,
        },
        "ingressQueue": ingress_report,
    }
    if lifecycle_report is not None:
        report["lifecycleReconnect"] = {
            "exitCode": lifecycle_code,
            "lifecycleReconnectGate": lifecycle_report.get("lifecycleReconnectGate"),
            "failures": lifecycle_report.get("failures"),
            "healthyWindow": lifecycle_report.get("healthyWindow"),
            "afterHealthy": lifecycle_report.get("afterHealthy"),
        }
    print(json.dumps(report, indent=2, ensure_ascii=False))
    return 0 if accepted else 2


if __name__ == "__main__":
    raise SystemExit(main())
