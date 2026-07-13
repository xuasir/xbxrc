#!/usr/bin/env python3
"""Validate Rust H264 profile negotiation against the browser fallback path."""

from __future__ import annotations

import argparse
import json
import sys
import time
from pathlib import Path
from typing import Any


DEFAULT_RUNTIME_LOG_DIR = Path("runtime-logs")
HIGH_PROFILE_PREFIXES = ("4d", "64")
BROWSER_PROFILE_PREFIXES = ("42e",)


def find_latest_trace(runtime_log_dir: Path) -> Path | None:
    traces = sorted(
        runtime_log_dir.glob("runtime-trace-*.jsonl"),
        key=lambda path: path.stat().st_mtime,
        reverse=True,
    )
    return traces[0] if traces else None


def load_events(trace: Path) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    with trace.open(encoding="utf-8") as handle:
        for line in handle:
            line = line.strip()
            if not line:
                continue
            try:
                row = json.loads(line)
            except json.JSONDecodeError:
                continue
            if isinstance(row, dict):
                rows.append(row)
    return rows


def payload_for(row: dict[str, Any]) -> dict[str, Any]:
    payload = row.get("payload")
    return payload if isinstance(payload, dict) else row


def text_field(payload: dict[str, Any], *names: str) -> str | None:
    for name in names:
        value = payload.get(name)
        if isinstance(value, str) and value:
            return value
    return None


def number_field(payload: dict[str, Any], *names: str) -> float | None:
    for name in names:
        value = payload.get(name)
        if isinstance(value, (int, float)):
            return float(value)
    return None


def profile_family(profile: str | None) -> str | None:
    if not profile:
        return None
    normalized = profile.lower().strip()
    if normalized.startswith(HIGH_PROFILE_PREFIXES):
        return "high"
    if normalized.startswith(BROWSER_PROFILE_PREFIXES):
        return "browser"
    if normalized.startswith("420"):
        return "baseline"
    return "other"


def nested_remote_answer_profile(payload: dict[str, Any]) -> str | None:
    observation = payload.get("latest_remote_answer_observation")
    if not isinstance(observation, dict):
        observation = payload.get("latestRemoteAnswerObservation")
    if not isinstance(observation, dict):
        return None
    return text_field(
        observation,
        "selected_video_profile_level_id",
        "selectedVideoProfileLevelId",
    )


def remote_answer_profiles(rows: list[dict[str, Any]]) -> list[dict[str, Any]]:
    observations: list[dict[str, Any]] = []
    seen: set[tuple[int | None, str | None, int | None]] = set()
    for row in rows:
        payload = payload_for(row)
        name = row.get("event") or row.get("name")
        profile: str | None = None
        payload_type: int | None = None
        if name == "remoteAnswerAccepted":
            profile = text_field(
                payload,
                "selectedVideoProfileLevelId",
                "selected_video_profile_level_id",
            )
            pt = payload.get("selectedVideoPayloadType")
            if isinstance(pt, int):
                payload_type = pt
        elif name == "statsSnapshot":
            profile = nested_remote_answer_profile(payload)
            observation = payload.get("latest_remote_answer_observation")
            if not isinstance(observation, dict):
                observation = payload.get("latestRemoteAnswerObservation")
            if isinstance(observation, dict):
                pt = observation.get("selected_video_payload_type")
                if not isinstance(pt, int):
                    pt = observation.get("selectedVideoPayloadType")
                if isinstance(pt, int):
                    payload_type = pt
        if not profile:
            continue
        key = (row.get("seq") if isinstance(row.get("seq"), int) else None, profile, payload_type)
        if key in seen:
            continue
        seen.add(key)
        observations.append(
            {
                "seq": row.get("seq"),
                "tsMs": row.get("tsMs"),
                "profile": profile,
                "family": profile_family(profile),
                "payloadType": payload_type,
                "event": name,
            }
        )
    return observations


def fallback_observations(rows: list[dict[str, Any]]) -> list[dict[str, Any]]:
    observations: list[dict[str, Any]] = []
    for row in rows:
        if (row.get("event") or row.get("name")) != "statsSnapshot":
            continue
        payload = payload_for(row)
        label = text_field(payload, "latest_observation_label", "latestObservationLabel")
        summary = text_field(payload, "latest_observation_summary", "latestObservationSummary")
        if label != "startupH264ProfileFallback" and not (
            summary and "startupH264ProfileFallback" in summary
        ):
            continue
        observations.append(
            {
                "seq": row.get("seq"),
                "tsMs": row.get("tsMs"),
                "label": label,
                "summary": summary,
            }
        )
    return observations


def first_playout_success_after(
    rows: list[dict[str, Any]], start_ts_ms: int | float | None
) -> dict[str, Any] | None:
    for row in rows:
        if (row.get("event") or row.get("name")) != "statsSnapshot":
            continue
        ts_ms = row.get("tsMs")
        if isinstance(start_ts_ms, (int, float)) and isinstance(ts_ms, (int, float)):
            if ts_ms < start_ts_ms:
                continue
        payload = payload_for(row)
        decode_at = number_field(
            payload, "latest_video_decode_ok_time_ms", "latestVideoDecodeOkTimeMs"
        )
        clean_anchor_at = number_field(
            payload,
            "video_anchor_clean_observed_at_ms",
            "videoAnchorCleanObservedAtMs",
            "recovery_fresh_anchor_recovered_at_ms",
            "recoveryFreshAnchorRecoveredAtMs",
        )
        host_present_at = number_field(
            payload,
            "latest_video_host_present_time_ms",
            "latestVideoHostPresentTimeMs",
        )
        display_state = text_field(payload, "receive_display_state", "receiveDisplayState")
        if host_present_at is None and display_state != "display-stable":
            continue
        return {
            "seq": row.get("seq"),
            "tsMs": row.get("tsMs"),
            "decodeAtMs": decode_at,
            "cleanAnchorAtMs": clean_anchor_at,
            "hostPresentAtMs": host_present_at,
            "receiveDisplayState": display_state,
        }
    return None


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


def build_report(
    trace: Path,
    *,
    require_fallback: bool,
    max_age_seconds: float | None,
) -> dict[str, Any]:
    rows = load_events(trace)
    answers = remote_answer_profiles(rows)
    fallbacks = fallback_observations(rows)
    first_fallback = fallbacks[0] if fallbacks else None
    fallback_ts = first_fallback.get("tsMs") if first_fallback else None
    answers_after_fallback = [
        answer
        for answer in answers
        if isinstance(fallback_ts, (int, float))
        and isinstance(answer.get("tsMs"), (int, float))
        and answer["tsMs"] >= fallback_ts
    ]
    high_answers = [answer for answer in answers if answer.get("family") == "high"]
    browser_answers_after_fallback = [
        answer for answer in answers_after_fallback if answer.get("family") == "browser"
    ]
    playout_success = first_playout_success_after(rows, fallback_ts if first_fallback else None)

    failures: list[str] = []
    freshness = trace_freshness_report(trace, max_age_seconds)
    if freshness["freshnessGate"] != "PASS":
        failures.append("trace is stale")
    if not answers:
        failures.append("missing remote answer profile observation")
    if require_fallback and not first_fallback:
        failures.append("missing startupH264ProfileFallback observation")
    if require_fallback and not high_answers:
        failures.append("missing high-profile answer before fallback")
    if first_fallback and not browser_answers_after_fallback:
        failures.append("missing 42e answer after fallback")
    if first_fallback and playout_success is None:
        failures.append("missing host present/display stable success after fallback")
    if not first_fallback and playout_success is None:
        playout_success = first_playout_success_after(rows, None)
        if playout_success is None:
            failures.append("missing host present/display stable success")

    return {
        "trace": str(trace),
        "h264ProfileFallbackGate": "FAIL" if failures else "PASS",
        "failures": failures,
        "freshness": freshness,
        "requireFallback": require_fallback,
        "remoteAnswerProfiles": answers[:12],
        "highAnswerCount": len(high_answers),
        "fallbackObservations": fallbacks[:5],
        "browserAnswersAfterFallback": browser_answers_after_fallback[:5],
        "mediaSuccess": playout_success,
        "playoutSuccess": playout_success,
    }


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Validate H264 4d-first negotiation and browser 42e fallback in a runtime trace."
    )
    parser.add_argument("trace", nargs="?", type=Path)
    parser.add_argument(
        "--latest",
        action="store_true",
        help="use newest runtime-logs/runtime-trace-*.jsonl when trace is omitted",
    )
    parser.add_argument(
        "--runtime-log-dir",
        type=Path,
        default=DEFAULT_RUNTIME_LOG_DIR,
        help="runtime log directory for --latest",
    )
    parser.add_argument(
        "--max-age-seconds",
        type=float,
        default=None,
        help="fail when selected trace mtime is older than this many seconds",
    )
    parser.add_argument(
        "--require-fallback",
        action="store_true",
        help="require the 4d/64 -> 42e fallback path instead of accepting high-profile success",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    trace = args.trace
    if args.latest:
        latest = find_latest_trace(args.runtime_log_dir)
        if latest is None:
            print(
                json.dumps(
                    {
                        "h264ProfileFallbackGate": "FAIL",
                        "failures": ["no runtime trace found"],
                    },
                    ensure_ascii=False,
                    indent=2,
                )
            )
            return 2
        trace = latest
    if trace is None:
        print("trace path is required unless --latest is used", file=sys.stderr)
        return 2
    report = build_report(
        trace,
        require_fallback=args.require_fallback,
        max_age_seconds=args.max_age_seconds,
    )
    print(json.dumps(report, ensure_ascii=False, indent=2))
    return 0 if report["h264ProfileFallbackGate"] == "PASS" else 2


if __name__ == "__main__":
    raise SystemExit(main())
