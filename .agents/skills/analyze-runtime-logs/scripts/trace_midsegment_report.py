#!/usr/bin/env python3
"""Mid-segment (default 79–150s) runtime trace report for low-latency display scheduling RFC."""

from __future__ import annotations

import argparse
import json
import statistics
import sys
from pathlib import Path
from typing import Any


def load_events(path: Path) -> list[dict[str, Any]]:
    events: list[dict[str, Any]] = []
    with path.open(encoding="utf-8") as handle:
        for line in handle:
            line = line.strip()
            if not line:
                continue
            try:
                events.append(json.loads(line))
            except json.JSONDecodeError:
                continue
    return events


def event_ts_ms(event: dict[str, Any]) -> float | None:
    for key in ("tsMs", "ts_ms", "timestampMs"):
        value = event.get(key)
        if isinstance(value, (int, float)):
            return float(value)
    payload = event.get("payload")
    if isinstance(payload, dict):
        for key in ("tsMs", "ts_ms"):
            value = payload.get(key)
            if isinstance(value, (int, float)):
                return float(value)
    return None


def origin_ms(events: list[dict[str, Any]]) -> float:
    timestamps = [ts for event in events if (ts := event_ts_ms(event)) is not None]
    return min(timestamps) if timestamps else 0.0


def in_window(event: dict[str, Any], origin: float, start_s: float, end_s: float) -> bool:
    ts = event_ts_ms(event)
    if ts is None:
        return False
    rel = (ts - origin) / 1000.0
    return start_s <= rel <= end_s


def nested_get(obj: dict[str, Any], *keys: str) -> Any:
    cur: Any = obj
    for key in keys:
        if not isinstance(cur, dict):
            return None
        cur = cur.get(key)
    return cur


def collect_stats_snapshot_metrics(
    events: list[dict[str, Any]], origin: float, start_s: float, end_s: float
) -> dict[str, list[float]]:
    submit_ages: list[float] = []
    present_ages: list[float] = []
    decode_fps_vals: list[float] = []
    present_fps_vals: list[float] = []
    session_phases: list[str] = []

    for event in events:
        if not in_window(event, origin, start_s, end_s):
            continue
        name = event.get("event") or event.get("name") or ""
        if name != "statsSnapshot":
            continue
        payload = event.get("payload") or event
        stats = payload.get("stats") if isinstance(payload.get("stats"), dict) else payload
        for field, bucket in (
            ("submitAgeMs", submit_ages),
            ("submit_age_ms", submit_ages),
            ("presentAgeMs", present_ages),
            ("present_age_ms", present_ages),
            ("decodeFps", decode_fps_vals),
            ("decode_fps", decode_fps_vals),
            ("fps", present_fps_vals),
            ("presentFps", present_fps_vals),
            ("present_fps", present_fps_vals),
        ):
            value = stats.get(field) if isinstance(stats, dict) else None
            if isinstance(value, (int, float)):
                bucket.append(float(value))
        phase = stats.get("sessionPhase") or stats.get("session_phase")
        if isinstance(phase, str):
            session_phases.append(phase)

    return {
        "submit_ages": submit_ages,
        "present_ages": present_ages,
        "decode_fps": decode_fps_vals,
        "present_fps": present_fps_vals,
        "session_phases": session_phases,
    }


def p95(values: list[float]) -> float | None:
    if not values:
        return None
    if len(values) == 1:
        return values[0]
    ordered = sorted(values)
    idx = int(round(0.95 * (len(ordered) - 1)))
    return ordered[idx]


def count_recovery_pulses(events: list[dict[str, Any]], origin: float, start_s: float, end_s: float) -> int:
    count = 0
    for event in events:
        if not in_window(event, origin, start_s, end_s):
            continue
        payload = event.get("payload") or event
        phase = payload.get("sessionPhase") or payload.get("session_phase")
        reason = payload.get("videoOwnerReason") or payload.get("video_owner_reason")
        milestone = payload.get("presentationMilestone") or payload.get("presentation_milestone")
        text = json.dumps(payload).lower()
        if phase == "recovering" or reason == "receiverWaitingKeyframe":
            count += 1
        elif milestone == "recovering":
            count += 1
        elif "receiverwaitingkeyframe" in text and "stats" in (event.get("event") or ""):
            count += 1
    return count


def count_mailbox_anomalies(events: list[dict[str, Any]], origin: float, start_s: float, end_s: float) -> int:
    anomalies = 0
    for event in events:
        if not in_window(event, origin, start_s, end_s):
            continue
        details = nested_get(event, "payload", "details") or nested_get(event, "details") or {}
        if not isinstance(details, dict):
            continue
        stage = nested_get(event, "payload", "stage") or event.get("stage")
        if stage == "hostMailboxRetainedDisplayed" and details.get("hasPendingFrame") is True:
            anomalies += 1
    return anomalies




def max_recovering_streak_s(
    events: list[dict[str, Any]], origin: float, start_s: float, end_s: float
) -> float:
    """Longest continuous statsSnapshot window with session_phase=recovering."""
    streak = 0.0
    max_streak = 0.0
    prev_rel: float | None = None
    for event in events:
        if not in_window(event, origin, start_s, end_s):
            continue
        name = event.get("event") or event.get("name") or ""
        if name != "statsSnapshot":
            continue
        ts = event_ts_ms(event)
        if ts is None:
            continue
        rel = (ts - origin) / 1000.0
        payload = event.get("payload") or event
        stats = payload.get("stats") if isinstance(payload.get("stats"), dict) else payload
        phase = stats.get("sessionPhase") or stats.get("session_phase") if isinstance(stats, dict) else None
        dt = (rel - prev_rel) if prev_rel is not None else 0.0
        if phase == "recovering":
            streak += max(dt, 0.0) if prev_rel is not None else 0.0
            max_streak = max(max_streak, streak)
        else:
            streak = 0.0
        prev_rel = rel
    return max_streak


def count_decoder_reset_while_waiting_keyframe(
    events: list[dict[str, Any]], origin: float, start_s: float, end_s: float, window_s: float = 5.0
) -> int:
    """Bursts of requestDecoderReset while stallKind/waiting-keyframe without fresh IDR."""
    resets: list[float] = []
    for event in events:
        if not in_window(event, origin, start_s, end_s):
            continue
        name = (event.get("event") or event.get("name") or "").lower()
        payload = event.get("payload") or event
        details = nested_get(event, "payload", "details") or nested_get(event, "details") or {}
        summary = ""
        if isinstance(payload, dict):
            summary = str(payload.get("latestDecisionSummary") or payload.get("summary") or "")
        if isinstance(details, dict):
            summary = summary or str(details.get("latestDecisionSummary") or "")
        action = ""
        if isinstance(payload, dict):
            action = str(payload.get("action") or payload.get("recoveryAction") or "")
        if "requestdecoderreset" not in (name + summary + action).lower():
            continue
        ts = event_ts_ms(event)
        if ts is None:
            continue
        rel = (ts - origin) / 1000.0
        # Heuristic: only count when trace shows waiting-keyframe context in same event or recent stats
        ctx = json.dumps(event).lower()
        if "waitingkeyframe" in ctx or "waiting-keyframe" in ctx or "receiverwaitingkeyframe" in ctx:
            resets.append(rel)
    if not resets:
        return 0
    max_in_window = 0
    for anchor in resets:
        count = sum(1 for t in resets if anchor <= t < anchor + window_s)
        max_in_window = max(max_in_window, count)
    return max_in_window

def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("trace", type=Path, help="Path to runtime-trace-*.jsonl")
    parser.add_argument("--start-s", type=float, default=79.0)
    parser.add_argument("--end-s", type=float, default=150.0)
    args = parser.parse_args()

    if not args.trace.is_file():
        print(f"trace not found: {args.trace}", file=sys.stderr)
        return 1

    events = load_events(args.trace)
    origin = origin_ms(events)
    metrics = collect_stats_snapshot_metrics(events, origin, args.start_s, args.end_s)
    submit_p95 = p95(metrics["submit_ages"])
    present_p95 = p95(metrics["present_ages"])
    steady_count = sum(1 for p in metrics["session_phases"] if p == "steady")
    phase_total = len(metrics["session_phases"])
    steady_ratio = (steady_count / phase_total * 100.0) if phase_total else 0.0
    decode_fps_avg = (
        statistics.mean(metrics["decode_fps"]) if metrics["decode_fps"] else None
    )
    present_fps_avg = (
        statistics.mean(metrics["present_fps"]) if metrics["present_fps"] else None
    )
    fps_gap = None
    if decode_fps_avg is not None and present_fps_avg is not None:
        fps_gap = decode_fps_avg - present_fps_avg

    recovering_count = count_recovery_pulses(events, origin, args.start_s, args.end_s)
    mailbox_anomalies = count_mailbox_anomalies(events, origin, args.start_s, args.end_s)
    recovering_streak_s = max_recovering_streak_s(events, origin, args.start_s, args.end_s)
    decoder_reset_burst = count_decoder_reset_while_waiting_keyframe(events, origin, args.start_s, args.end_s)

    print(f"trace: {args.trace}")
    print(f"window: +{args.start_s:.0f}s – +{args.end_s:.0f}s (origin_ts_ms={origin:.0f})")
    print(f"statsSnapshots in window: {phase_total}")
    print(f"session_phase steady ratio: {steady_ratio:.1f}%")
    print(f"recovering / receiverWaitingKeyframe signals: {recovering_count}")
    print(f"max continuous recovering streak: {recovering_streak_s:.1f}s")
    print(f"requestDecoderReset while waiting-keyframe (max per 5s): {decoder_reset_burst}")
    print(f"submit_age_ms P95: {submit_p95 if submit_p95 is not None else 'n/a'}")
    print(f"present_age_ms P95: {present_p95 if present_p95 is not None else 'n/a'}")
    if decode_fps_avg is not None and present_fps_avg is not None:
        print(f"decode_fps avg: {decode_fps_avg:.1f}, present_fps avg: {present_fps_avg:.1f}, gap: {fps_gap:.1f}")
    print(f"hostMailboxRetainedDisplayed+hasPendingFrame: {mailbox_anomalies}")

    gates = []
    if phase_total > 0 and steady_ratio < 95.0:
        gates.append(f"steady ratio {steady_ratio:.1f}% < 95%")
    if recovering_count >= 3:
        gates.append(f"recovery pulses {recovering_count} >= 3")
    if recovering_streak_s > 5.0:
        gates.append(f"recovering streak {recovering_streak_s:.1f}s > 5s")
    if decoder_reset_burst > 1:
        gates.append(f"decoder reset burst {decoder_reset_burst} > 1 per 5s while waiting-keyframe")
    if submit_p95 is not None and submit_p95 >= 200.0:
        gates.append(f"submit_age P95 {submit_p95:.0f}ms >= 200ms")
    if mailbox_anomalies > 0:
        gates.append(f"mailbox anomalies {mailbox_anomalies} > 0")

    if gates:
        print("GATE: FAIL")
        for item in gates:
            print(f"  - {item}")
        return 2
    print("GATE: PASS (heuristic; manual subjective check still recommended)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
