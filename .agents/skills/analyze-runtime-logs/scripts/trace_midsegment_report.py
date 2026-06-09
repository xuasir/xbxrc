#!/usr/bin/env python3
"""Mid-segment runtime trace report: global latency + SteadySupply present contract."""

from __future__ import annotations

import argparse
import json
import statistics
import sys
from collections import Counter
from pathlib import Path
from typing import Any

LATENCY_SANITY_MAX_MS = 5_000.0
STEADY_DECODE_MIN = 28.0
STEADY_DECODE_MAX = 32.0
STEADY_SUPPLY_GAP_MAX = 6.0
STEADY_PHASE_RATIO_MIN = 95.0
READY_RATIO_MIN = 0.85
SUBMIT_TO_PRESENT_P95_MAX_MS = 80.0


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


def p95(values: list[float]) -> float | None:
    if not values:
        return None
    if len(values) == 1:
        return values[0]
    return statistics.quantiles(sorted(values), n=20, method="inclusive")[18]


def filter_latency(values: list[float]) -> list[float]:
    return [v for v in values if 0.0 <= v <= LATENCY_SANITY_MAX_MS]


def collect_stats_snapshot_metrics(
    events: list[dict[str, Any]], origin: float, start_s: float, end_s: float
) -> dict[str, Any]:
    submit_ages: list[float] = []
    present_ages: list[float] = []
    submit_to_present: list[float] = []
    decode_fps_vals: list[float] = []
    present_fps_vals: list[float] = []
    session_phases: list[str] = []
    steady_supply_rows: list[tuple[float, float]] = []
    stats_snapshots: list[dict[str, Any]] = []

    for event in events:
        if not in_window(event, origin, start_s, end_s):
            continue
        name = event.get("event") or event.get("name") or ""
        if name != "statsSnapshot":
            continue
        payload = event.get("payload") or event
        stats = payload.get("stats") if isinstance(payload.get("stats"), dict) else payload
        if not isinstance(stats, dict):
            continue
        stats_snapshots.append(stats)
        for field, bucket in (
            ("submitAgeMs", submit_ages),
            ("submit_age_ms", submit_ages),
            ("presentAgeMs", present_ages),
            ("present_age_ms", present_ages),
            ("submitToPresentMs", submit_to_present),
            ("submit_to_present_ms", submit_to_present),
            ("decodeFps", decode_fps_vals),
            ("decode_fps", decode_fps_vals),
            ("fps", present_fps_vals),
            ("presentFps", present_fps_vals),
            ("present_fps", present_fps_vals),
        ):
            value = stats.get(field)
            if isinstance(value, (int, float)):
                bucket.append(float(value))
        phase = stats.get("sessionPhase") or stats.get("session_phase")
        if isinstance(phase, str):
            session_phases.append(phase)
        decode_fps = stats.get("decode_fps") or stats.get("decodeFps")
        present_fps = stats.get("fps") or stats.get("presentFps")
        if (
            phase == "steady"
            and isinstance(decode_fps, (int, float))
            and isinstance(present_fps, (int, float))
            and STEADY_DECODE_MIN <= float(decode_fps) <= STEADY_DECODE_MAX
        ):
            steady_supply_rows.append((float(decode_fps), float(present_fps)))

    return {
        "submit_ages": filter_latency(submit_ages),
        "present_ages": filter_latency(present_ages),
        "submit_to_present": filter_latency(submit_to_present),
        "decode_fps": decode_fps_vals,
        "present_fps": present_fps_vals,
        "session_phases": session_phases,
        "steady_supply_rows": steady_supply_rows,
        "frame_supply_deltas": collect_counter_deltas(stats_snapshots),
    }


COUNTER_FIELDS: tuple[tuple[str, tuple[str, ...]], ...] = (
    ("inboundFrames", ("inbound_video_frame_count_total", "inboundFrameCountTotal")),
    ("rtpMarkers", ("inbound_video_rtp_marker_count_total", "inboundRtpMarkerCountTotal")),
    ("accessUnits", ("inbound_video_access_unit_count_total", "inboundAccessUnitCountTotal")),
    (
        "decodeGateEmit",
        ("inbound_video_decode_gate_emit_count_total", "inboundDecodeGateEmitCountTotal"),
    ),
    (
        "decodeGateContinue",
        (
            "inbound_video_decode_gate_continue_count_total",
            "inboundDecodeGateContinueCountTotal",
        ),
    ),
    ("pacerSubmit", ("video_pacer_submit_count_total", "pacerSubmitCountTotal")),
    ("pacerDrop", ("video_pacer_drop_count_total", "pacerDropCountTotal")),
    ("rendererSubmit", ("video_renderer_submit_count_total", "rendererSubmitCountTotal")),
    ("hostEnqueue", ("host_mailbox_enqueue_count_total", "hostMailboxEnqueueCountTotal")),
    ("hostOverwrite", ("host_mailbox_overwrite_count_total", "hostMailboxOverwriteCountTotal")),
    ("hostPresent", ("host_frame_present_epoch", "hostFramePresentEpoch")),
    ("hostNoPendingTake", ("host_no_pending_take_count_total", "hostNoPendingTakeCountTotal")),
)


def first_numeric(stats: dict[str, Any], keys: tuple[str, ...]) -> float | None:
    for key in keys:
        value = stats.get(key)
        if isinstance(value, (int, float)):
            return float(value)
    return None


def collect_counter_deltas(stats_snapshots: list[dict[str, Any]]) -> dict[str, int]:
    if len(stats_snapshots) < 2:
        return {}
    first = stats_snapshots[0]
    last = stats_snapshots[-1]
    deltas: dict[str, int] = {}
    for label, keys in COUNTER_FIELDS:
        start = first_numeric(first, keys)
        end = first_numeric(last, keys)
        if start is None or end is None:
            continue
        deltas[label] = int(end - start)
    return deltas


def collect_host_take_metrics(
    events: list[dict[str, Any]], origin: float, start_s: float, end_s: float
) -> dict[str, int]:
    take = Counter()
    retained_pending = 0
    for event in events:
        if not in_window(event, origin, start_s, end_s):
            continue
        name = event.get("event") or ""
        payload = event.get("payload") or {}
        details = nested_get(payload, "details") or {}
        if not isinstance(details, dict):
            details = {}

        if name == "hostMailboxTakeDecision":
            decision = details.get("decision") or payload.get("decision")
            has_pending = details.get("hasPendingFrame") or payload.get("hasPendingFrame")
        elif name == "hostTiming":
            stage = payload.get("stage") or ""
            if stage not in (
                "hostMailboxTakeDecision",
                "hostMailboxRetainedDisplayed",
            ):
                continue
            decision = details.get("decision")
            has_pending = details.get("hasPendingFrame")
        else:
            continue

        if not isinstance(decision, str):
            continue
        take[decision] += 1
        if decision == "retainedDisplayed" and has_pending is True:
            retained_pending += 1

    return {"take": take, "retained_pending": retained_pending}


def collect_frame_drops(
    events: list[dict[str, Any]], origin: float, start_s: float, end_s: float
) -> Counter[str]:
    drops: Counter[str] = Counter()
    for event in events:
        if not in_window(event, origin, start_s, end_s):
            continue
        if (event.get("event") or "") != "frameDropped":
            continue
        payload = event.get("payload") or {}
        stage = str(payload.get("stage") or "?")
        detail = str(payload.get("detail") or payload.get("reason") or "?")[:60]
        drops[f"{stage}|{detail}"] += 1
    return drops


def count_recovery_pulses(
    events: list[dict[str, Any]], origin: float, start_s: float, end_s: float
) -> int:
    count = 0
    for event in events:
        if not in_window(event, origin, start_s, end_s):
            continue
        payload = event.get("payload") or event
        phase = payload.get("sessionPhase") or payload.get("session_phase")
        reason = payload.get("videoOwnerReason") or payload.get("video_owner_reason")
        milestone = payload.get("presentationMilestone") or payload.get("presentation_milestone")
        if phase == "recovering" or reason == "receiverWaitingKeyframe" or milestone == "recovering":
            count += 1
    return count


def max_recovering_streak_s(
    events: list[dict[str, Any]], origin: float, start_s: float, end_s: float
) -> float:
    streak = 0.0
    max_streak = 0.0
    prev_rel: float | None = None
    for event in events:
        if not in_window(event, origin, start_s, end_s):
            continue
        if (event.get("event") or "") != "statsSnapshot":
            continue
        ts = event_ts_ms(event)
        if ts is None:
            continue
        rel = (ts - origin) / 1000.0
        payload = event.get("payload") or event
        stats = payload.get("stats") if isinstance(payload.get("stats"), dict) else payload
        phase = (
            stats.get("sessionPhase") or stats.get("session_phase")
            if isinstance(stats, dict)
            else None
        )
        dt = (rel - prev_rel) if prev_rel is not None else 0.0
        if phase == "recovering":
            streak += max(dt, 0.0) if prev_rel is not None else 0.0
            max_streak = max(max_streak, streak)
        else:
            streak = 0.0
        prev_rel = rel
    return max_streak


def ready_ratio(take: Counter[str]) -> float | None:
    ready = take.get("ready", 0) + take.get("ReadyDisplayedReplay", 0)
    denom = ready + take.get("retainedDisplayed", 0)
    if denom == 0:
        return None
    return ready / denom


def host_counter_ready_ratio(frame_supply_deltas: dict[str, int]) -> float | None:
    host_present = frame_supply_deltas.get("hostPresent")
    host_enqueue = frame_supply_deltas.get("hostEnqueue")
    if not host_present or not host_enqueue:
        return None
    if host_enqueue <= 0:
        return None
    return min(host_present, host_enqueue) / host_enqueue


def collect_startup_supply_metrics(
    events: list[dict[str, Any]], origin: float, first_present_window_s: float = 5.0
) -> dict[str, Any]:
    """起播门禁：首显后窗口内 supply-starved / keyframeRequestOutcome / waiting-keyframe。"""
    first_present_rel: float | None = None
    for event in events:
        if (event.get("event") or "") != "statsSnapshot":
            continue
        ts = event_ts_ms(event)
        if ts is None:
            continue
        payload = event.get("payload") or event
        stats = payload.get("stats") if isinstance(payload.get("stats"), dict) else payload
        if not isinstance(stats, dict):
            continue
        epoch = stats.get("hostFramePresentEpoch") or stats.get("host_frame_present_epoch")
        if isinstance(epoch, (int, float)) and float(epoch) > 0:
            first_present_rel = (ts - origin) / 1000.0
            break
    if first_present_rel is None:
        return {
            "first_present_rel": None,
            "supply_starved_in_window": 0,
            "keyframe_outcomes": 0,
            "waiting_keyframe_snapshots": 0,
        }
    end_rel = first_present_rel + first_present_window_s
    supply_starved = 0
    keyframe_outcomes = 0
    waiting_kf = 0
    for event in events:
        ts = event_ts_ms(event)
        if ts is None:
            continue
        rel = (ts - origin) / 1000.0
        if rel < first_present_rel or rel > end_rel:
            continue
        name = event.get("event") or ""
        payload = event.get("payload") or event
        if name == "statsSnapshot":
            stats = payload.get("stats") if isinstance(payload.get("stats"), dict) else payload
            if isinstance(stats, dict):
                owner = stats.get("videoOwnerState") or stats.get("recovery_owner_state")
                if owner == "supply-starved":
                    supply_starved += 1
                decoder = stats.get("decoderState") or stats.get("video_decoder_recovery_state")
                if decoder == "waiting-keyframe":
                    waiting_kf += 1
        if (event.get("event") or "") == "keyframeRequestOutcome":
            keyframe_outcomes += 1
        label = payload.get("latestObservationLabel") or payload.get("latest_observation_label")
        if label == "keyframeRequestOutcome":
            keyframe_outcomes += 1
        recovery = payload.get("recovery") if isinstance(payload.get("recovery"), dict) else {}
        if recovery.get("latestObservationLabel") == "keyframeRequestOutcome":
            keyframe_outcomes += 1
    return {
        "first_present_rel": first_present_rel,
        "supply_starved_in_window": supply_starved,
        "keyframe_outcomes": keyframe_outcomes,
        "waiting_keyframe_snapshots": waiting_kf,
    }



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
    trace_mode = next((e.get("traceMode") for e in events[:5] if e.get("traceMode")), "unknown")
    origin = origin_ms(events)
    metrics = collect_stats_snapshot_metrics(events, origin, args.start_s, args.end_s)
    take_metrics = collect_host_take_metrics(events, origin, args.start_s, args.end_s)
    drops = collect_frame_drops(events, origin, args.start_s, args.end_s)

    submit_p95 = p95(metrics["submit_ages"])
    present_p95 = p95(metrics["present_ages"])
    submit_to_present_p95 = p95(metrics["submit_to_present"])
    steady_count = sum(1 for p in metrics["session_phases"] if p == "steady")
    phase_total = len(metrics["session_phases"])
    steady_ratio = (steady_count / phase_total * 100.0) if phase_total else 0.0
    decode_fps_avg = statistics.mean(metrics["decode_fps"]) if metrics["decode_fps"] else None
    present_fps_avg = statistics.mean(metrics["present_fps"]) if metrics["present_fps"] else None
    fps_gap = (
        (decode_fps_avg - present_fps_avg)
        if decode_fps_avg is not None and present_fps_avg is not None
        else None
    )

    steady_supply_gap = None
    if metrics["steady_supply_rows"]:
        gaps = [d - p for d, p in metrics["steady_supply_rows"]]
        steady_supply_gap = statistics.mean(gaps)

    recovering_count = count_recovery_pulses(events, origin, args.start_s, args.end_s)
    recovering_streak_s = max_recovering_streak_s(events, origin, args.start_s, args.end_s)
    startup = collect_startup_supply_metrics(events, origin)
    media_supply_gates: list[str] = []
    media_verdict = "SKIPPED"
    if startup["first_present_rel"] is not None:
        media_verdict = "PASS"
        if startup["supply_starved_in_window"] > 0:
            media_supply_gates.append(
                f"supply-starved snapshots in first-present+5s: {startup['supply_starved_in_window']}"
            )
        if (
            startup["keyframe_outcomes"] < 1
            and (startup["supply_starved_in_window"] > 0 or startup["waiting_keyframe_snapshots"] > 0)
        ):
            media_supply_gates.append("no keyframeRequestOutcome in first-present+5s")
        if startup["waiting_keyframe_snapshots"] >= 3:
            media_supply_gates.append(
                f"waiting-keyframe snapshots {startup['waiting_keyframe_snapshots']} >= 3 in 5s"
            )
        if media_supply_gates:
            media_verdict = "FAIL"
    else:
        media_supply_gates.append("no host present epoch in trace")

    ratio = ready_ratio(take_metrics["take"])
    counter_ratio = host_counter_ready_ratio(metrics["frame_supply_deltas"])

    print(f"trace: {args.trace}")
    print(f"traceMode: {trace_mode}")
    print(f"window: +{args.start_s:.0f}s – +{args.end_s:.0f}s (origin_ts_ms={origin:.0f})")
    print(f"statsSnapshots in window: {phase_total}")
    print(f"session_phase steady ratio: {steady_ratio:.1f}%")
    print(f"recovering / receiverWaitingKeyframe signals: {recovering_count}")
    print(f"max continuous recovering streak: {recovering_streak_s:.1f}s")
    print(f"submit_age_ms P95 (sanitized): {submit_p95 if submit_p95 is not None else 'n/a'}")
    print(f"present_age_ms P95 (sanitized): {present_p95 if present_p95 is not None else 'n/a'}")
    print(f"submit_to_present_ms P95 (sanitized): {submit_to_present_p95 if submit_to_present_p95 is not None else 'n/a'}")
    if decode_fps_avg is not None and present_fps_avg is not None:
        print(
            f"decode_fps avg: {decode_fps_avg:.1f}, present_fps avg: {present_fps_avg:.1f}, gap: {fps_gap:.1f}"
        )
    if steady_supply_gap is not None:
        print(
            f"steady_supply (phase=steady, decode in [{STEADY_DECODE_MIN},{STEADY_DECODE_MAX}]): "
            f"samples={len(metrics['steady_supply_rows'])} gap_avg={steady_supply_gap:.1f}"
        )
    if metrics["frame_supply_deltas"]:
        print(f"frame_supply_deltas: {metrics['frame_supply_deltas']}")
    print(f"host take decisions (mid): {dict(take_metrics['take'])}")
    if ratio is not None:
        print(f"ready/(ready+retainedDisplayed): {ratio:.1%}")
    if counter_ratio is not None:
        print(f"host_present/host_enqueue delta ratio: {counter_ratio:.1%}")
    print(f"retainedDisplayed+hasPendingFrame: {take_metrics['retained_pending']}")
    if drops:
        print(f"frameDropped top: {drops.most_common(5)}")

    global_gates: list[str] = []
    if submit_p95 is not None and submit_p95 >= 200.0:
        global_gates.append(f"submit_age P95 {submit_p95:.0f}ms >= 200ms")
    if recovering_count >= 3:
        global_gates.append(f"recovery pulses {recovering_count} >= 3")
    if recovering_streak_s > 5.0:
        global_gates.append(f"recovering streak {recovering_streak_s:.1f}s > 5s")

    steady_gates: list[str] = []
    steady_verdict = "SKIPPED"
    if phase_total == 0:
        steady_gates.append("no statsSnapshot in window")
    elif steady_ratio < STEADY_PHASE_RATIO_MIN:
        steady_gates.append(f"steady ratio {steady_ratio:.1f}% < {STEADY_PHASE_RATIO_MIN}%")
    else:
        steady_verdict = "PASS"
        if steady_supply_gap is not None and steady_supply_gap > STEADY_SUPPLY_GAP_MAX:
            steady_gates.append(
                f"steady_supply gap {steady_supply_gap:.1f} > {STEADY_SUPPLY_GAP_MAX}"
            )
        if submit_to_present_p95 is not None and submit_to_present_p95 >= SUBMIT_TO_PRESENT_P95_MAX_MS:
            steady_gates.append(
                f"submit_to_present P95 {submit_to_present_p95:.0f}ms >= {SUBMIT_TO_PRESENT_P95_MAX_MS:.0f}ms"
            )
        if take_metrics["retained_pending"] > 0:
            steady_gates.append(
                f"retainedDisplayed+hasPending {take_metrics['retained_pending']} > 0"
            )
        effective_ready_ratio = counter_ratio if counter_ratio is not None else ratio
        if effective_ready_ratio is not None and effective_ready_ratio < READY_RATIO_MIN:
            steady_gates.append(
                f"ready ratio {effective_ready_ratio:.1%} < {READY_RATIO_MIN:.0%}"
            )
        if steady_gates:
            steady_verdict = "FAIL"

    print(f"GLOBAL_LATENCY_GATE: {'FAIL' if global_gates else 'PASS'}")
    for item in global_gates:
        print(f"  - {item}")
    if startup["first_present_rel"] is not None:
        print(
            f"startup window: first_present=+{startup['first_present_rel']:.1f}s "
            f"supply_starved={startup['supply_starved_in_window']} "
            f"keyframe_outcomes={startup['keyframe_outcomes']} "
            f"waiting_kf={startup['waiting_keyframe_snapshots']}"
        )
    print(f"MEDIA_SUPPLY_GATE: {media_verdict}")
    for item in media_supply_gates:
        print(f"  - {item}")

    print(f"STEADY_SUPPLY_GATE: {steady_verdict}")
    for item in steady_gates:
        print(f"  - {item}")

    if media_verdict == "FAIL":
        return 4
    if global_gates:
        return 2
    if steady_verdict == "FAIL":
        return 3
    return 0


if __name__ == "__main__":
    sys.exit(main())
