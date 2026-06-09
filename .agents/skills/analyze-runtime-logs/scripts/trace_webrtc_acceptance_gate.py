#!/usr/bin/env python3
"""Combined WebRTC acceptance gate for receive recovery and low-latency steady state."""

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
DEFAULT_RUNTIME_LOG_DIR = Path("runtime-logs")


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


def run_midsegment_gate(trace: Path, start_s: float, end_s: float) -> tuple[int, dict[str, Any]]:
    result = run_command(
        [
            sys.executable,
            "-B",
            str(MIDSEGMENT_REPORT),
            str(trace),
            "--start-s",
            str(start_s),
            "--end-s",
            str(end_s),
        ]
    )
    report = {
        "globalLatencyGate": parse_midsegment_gate(stdout=result.stdout, label="GLOBAL_LATENCY_GATE"),
        "mediaSupplyGate": parse_midsegment_gate(stdout=result.stdout, label="MEDIA_SUPPLY_GATE"),
        "steadySupplyGate": parse_midsegment_gate(stdout=result.stdout, label="STEADY_SUPPLY_GATE"),
        "stderr": result.stderr,
    }
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
    parser.add_argument("--start-s", type=float, default=79.0)
    parser.add_argument("--end-s", type=float, default=150.0)
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
    accepted = freshness_ok and receive_code == 0 and midsegment_code == 0
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
    }
    print(json.dumps(report, indent=2, ensure_ascii=False))
    return 0 if accepted else 2


if __name__ == "__main__":
    raise SystemExit(main())
