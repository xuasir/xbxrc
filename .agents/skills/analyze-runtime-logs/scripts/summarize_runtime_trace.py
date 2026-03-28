#!/usr/bin/env python3

from __future__ import annotations

import argparse
import json
import sys
from collections import Counter
from dataclasses import dataclass
from pathlib import Path
from typing import Any


SUSPICIOUS_TERMS = (
    "error",
    "fail",
    "timeout",
    "unavailable",
    "drop",
    "dropped",
    "panic",
    "stall",
    "reconnect",
    "reset",
    "provision",
    "waiting",
    "keyframe",
    "deadline",
    "backpressure",
    "backlog",
    "overrun",
    "underflow",
    "starv",
    "late",
    "lag",
    "queue full",
    "nopendingframe",
)

PHASE_FIELDS = (
    ("session_phase", "sessionPhase"),
    ("video_health", "videoHealth"),
    ("transport_state", "transportState"),
    ("primary_issue_chain", "primaryIssueChain"),
    ("stall_kind", "stallKind"),
)

PHASE_DETAIL_FIELDS = (
    ("recovery_policy_profile", "recoveryPolicyProfile"),
    ("transport_policy_profile", "transportPolicyProfile"),
    ("recovery_coupling_mode", "recoveryCouplingMode"),
    ("twcc_observation_state", "twccObservationState"),
    ("actual_video_bitrate_source", "actualVideoBitrateSource"),
)

PERF_HINT_FIELDS = (
    ("present_age_ms", "presentAgeMs"),
    ("decode_age_ms", "decodeAgeMs"),
    ("packet_age_ms", "packetAgeMs"),
    ("packet_to_decode_ms", "packetToDecodeMs"),
    ("packet_to_present_ms", "packetToPresentMs"),
    ("present_fps", "presentFps"),
    ("decode_fps", "decodeFps"),
    ("video_decoder_stalled", "videoDecoderStalled"),
    ("video_renderer_stalled", "videoRendererStalled"),
    ("video_decoder_hardware_failure_streak", "videoDecoderHardwareFailureStreak"),
)

INTERESTING_SIGNAL_FIELDS = (
    "video_decoder_stalled",
    "video_renderer_stalled",
    "video_decoder_hardware_failure_streak",
    "present_drop_count_total",
    "present_overwrite_count_total",
    "video_present_drop_count_total",
    "video_renderer_drop_count_total",
    "video_pacer_drop_count_total",
    "video_decode_input_drop_count_total",
    "video_decode_output_drop_count_total",
    "recovery_keyframe_request_count",
    "recovery_decoder_reset_count",
    "recovery_reconnect_count",
)

HEALTHY_SESSION_PHASES = {"unknown", "connecting", "active", "steady", "healthy", "ready", "playing", "running"}
HEALTHY_VIDEO_STATES = {"unknown", "connecting", "healthy", "ready", "active"}
HEALTHY_TRANSPORT_STATES = {"new", "connecting", "checking", "connected", "completed"}
BAD_STATE_HINTS = (
    "error",
    "fail",
    "timeout",
    "stall",
    "stalled",
    "drop",
    "dropped",
    "reconnect",
    "reset",
    "pause",
    "paused",
    "degrad",
    "backpressure",
    "backlog",
    "starv",
    "underflow",
    "overrun",
    "closed",
    "disconnect",
    "inactive",
)


@dataclass
class PhaseSegment:
    signature: str
    detail: str
    start_row: dict[str, Any]
    end_row: dict[str, Any]
    row_count: int = 1


@dataclass
class GapWindow:
    gap_ms: int
    left_row: dict[str, Any]
    right_row: dict[str, Any]


@dataclass
class ClusterWindow:
    start_ts: int
    end_ts: int
    rows: list[dict[str, Any]]


def fmt_ms(value: int | None) -> str:
    if value is None:
        return "n/a"
    return f"{value / 1000:.3f}s"


def row_ts(row: dict[str, Any]) -> int | None:
    value = row.get("tsMs")
    return value if isinstance(value, int) else None


def row_seq(row: dict[str, Any]) -> Any:
    return row.get("seq")


def payload_get(payload: Any, *keys: str) -> Any:
    if not isinstance(payload, dict):
        return None
    for key in keys:
        if key in payload:
            value = payload[key]
            if value not in (None, "", [], {}):
                return value
    return None


def short_payload(payload: Any) -> str:
    if isinstance(payload, dict):
        for key in (
            "message",
            "reason",
            "error",
            "status",
            "phase",
            "action",
            "summary",
            "label",
            "detail",
        ):
            value = payload.get(key)
            if value not in (None, "", [], {}):
                return str(value)

        perf_items: list[str] = []
        for snake_key, camel_key in PERF_HINT_FIELDS:
            value = payload_get(payload, snake_key, camel_key)
            if value not in (None, "", [], {}):
                perf_items.append(f"{camel_key}={value}")
        if perf_items:
            return " ".join(perf_items)

        value_items: list[str] = []
        for snake_key, camel_key in PHASE_FIELDS + PHASE_DETAIL_FIELDS:
            value = payload_get(payload, snake_key, camel_key)
            if value not in (None, "", [], {}):
                value_items.append(f"{camel_key}={value}")
        if value_items:
            return " ".join(value_items)

        text = json.dumps(payload, ensure_ascii=False, sort_keys=True)
        return text[:180]
    return str(payload)[:180]


def extract_payload_strings(payload: Any) -> list[str]:
    if not isinstance(payload, dict):
        return []

    values: list[str] = []
    for key in ("message", "reason", "error", "status", "phase", "action", "summary", "label", "detail"):
        value = payload.get(key)
        if isinstance(value, str) and value:
            values.append(value)
    return values


def contains_bad_hint(value: Any) -> bool:
    if not isinstance(value, str):
        return False
    lower = value.lower()
    return any(hint in lower for hint in BAD_STATE_HINTS)


def is_suspicious(row: dict[str, Any]) -> bool:
    haystacks = [
        str(row.get("event", "")).lower(),
        str(row.get("domain", "")).lower(),
        str(row.get("category", "")).lower(),
        json.dumps(row.get("payload", ""), ensure_ascii=False).lower(),
    ]
    return any(term in haystack for haystack in haystacks for term in SUSPICIOUS_TERMS)


def is_focus_row(row: dict[str, Any], session_id: str | None, domain: str | None) -> bool:
    if session_id is not None and str(row.get("sessionId")) != session_id:
        return False
    if domain is not None and str(row.get("domain")) != domain:
        return False
    return True


def build_signature(row: dict[str, Any]) -> tuple[str, str] | None:
    if row.get("category") not in {"state", "decision", "snapshot"}:
        return None

    payload = row.get("payload")
    if not isinstance(payload, dict):
        return None

    phase_values: list[str] = []
    detail_values: list[str] = []

    for snake_key, camel_key in PHASE_FIELDS:
        value = payload_get(payload, snake_key, camel_key)
        if value is not None:
            phase_values.append(f"{camel_key}={value}")

    for snake_key, camel_key in PHASE_DETAIL_FIELDS:
        value = payload_get(payload, snake_key, camel_key)
        if value is not None:
            detail_values.append(f"{camel_key}={value}")

    if not phase_values and not detail_values:
        return None

    return (" | ".join(phase_values) or "phase=unknown", " | ".join(detail_values))


def classify_signal(row: dict[str, Any]) -> str | None:
    payload = row.get("payload")
    if not isinstance(payload, dict):
        return None

    for text in extract_payload_strings(payload):
        if any(term in text.lower() for term in ("error", "fail", "panic", "timeout")):
            return "error"

    for text in extract_payload_strings(payload):
        if contains_bad_hint(text):
            return "state"

    for field in INTERESTING_SIGNAL_FIELDS:
        value = payload_get(payload, field, field)
        if isinstance(value, (int, float)) and value > 0:
            return field

    for field in ("video_decoder_stalled", "video_renderer_stalled"):
        value = payload_get(payload, field, field)
        if value is True:
            return field

    for field in ("present_age_ms", "decode_age_ms", "packet_age_ms", "packet_to_decode_ms", "packet_to_present_ms"):
        value = payload_get(payload, field, field)
        if isinstance(value, (int, float)) and value >= 1000:
            return field

    for field in ("session_phase", "video_health", "transport_state", "primary_issue_chain", "stall_kind"):
        value = payload_get(payload, field, field)
        if isinstance(value, str):
            lower = value.lower()
            if lower in HEALTHY_SESSION_PHASES or lower in HEALTHY_VIDEO_STATES or lower in HEALTHY_TRANSPORT_STATES:
                continue
            if any(hint in lower for hint in BAD_STATE_HINTS):
                return field

    return None


def cluster_signal_rows(rows: list[dict[str, Any]], cluster_window_ms: int) -> list[ClusterWindow]:
    signal_rows = [row for row in rows if row_ts(row) is not None and classify_signal(row) is not None]
    signal_rows.sort(key=lambda row: (row_ts(row) or 0, row_seq(row) or 0))

    clusters: list[ClusterWindow] = []
    if not signal_rows:
        return clusters

    cluster_rows = [signal_rows[0]]
    cluster_start = row_ts(signal_rows[0]) or 0
    cluster_end = cluster_start

    for row in signal_rows[1:]:
        ts = row_ts(row) or 0
        if ts - cluster_end <= cluster_window_ms:
            cluster_rows.append(row)
            cluster_end = ts
            continue
        clusters.append(ClusterWindow(cluster_start, cluster_end, cluster_rows))
        cluster_rows = [row]
        cluster_start = ts
        cluster_end = ts

    clusters.append(ClusterWindow(cluster_start, cluster_end, cluster_rows))
    return clusters


def find_long_gaps(rows: list[dict[str, Any]], gap_threshold_ms: int) -> list[GapWindow]:
    anchors = [
        row
        for row in rows
        if row.get("category") in {"state", "decision", "snapshot"} or classify_signal(row) is not None
    ]
    anchors.sort(key=lambda row: (row_ts(row) or 0, row_seq(row) or 0))

    gaps: list[GapWindow] = []
    for left_row, right_row in zip(anchors, anchors[1:]):
        left_ts = row_ts(left_row)
        right_ts = row_ts(right_row)
        if left_ts is None or right_ts is None:
            continue
        gap_ms = right_ts - left_ts
        if gap_ms >= gap_threshold_ms:
            gaps.append(GapWindow(gap_ms=gap_ms, left_row=left_row, right_row=right_row))
    gaps.sort(key=lambda window: window.gap_ms, reverse=True)
    return gaps


def build_phase_segments(rows: list[dict[str, Any]]) -> list[PhaseSegment]:
    segments: list[PhaseSegment] = []
    current: PhaseSegment | None = None

    for row in rows:
        if row.get("category") not in {"state", "decision"}:
            continue

        signature = build_signature(row)
        if signature is None:
            continue

        phase_signature, detail_signature = signature
        if current is None or current.signature != phase_signature or current.detail != detail_signature:
            current = PhaseSegment(
                signature=phase_signature,
                detail=detail_signature,
                start_row=row,
                end_row=row,
            )
            segments.append(current)
            continue

        current.row_count += 1
        current.end_row = row

    return segments


def format_row_ref(row: dict[str, Any]) -> str:
    return (
        f"seq={row_seq(row)} tsMs={row_ts(row)} "
        f"{row.get('category')}/{row.get('domain')}/{row.get('event')}"
    )


def print_phase_segments(segments: list[PhaseSegment], limit: int) -> None:
    print("\nphase_windows:")
    if not segments:
        print("  - none")
        return

    for segment in segments[:limit]:
        start_ts = row_ts(segment.start_row)
        end_ts = row_ts(segment.end_row)
        duration_ms = None
        if start_ts is not None and end_ts is not None:
            duration_ms = end_ts - start_ts
        print(
            "  - "
            f"{format_row_ref(segment.start_row)} -> {format_row_ref(segment.end_row)} "
            f"duration={fmt_ms(duration_ms)} rows={segment.row_count} "
            f"phase={segment.signature}"
        )
        if segment.detail:
            print(f"    detail={segment.detail}")

    if len(segments) > limit:
        print(f"  - ... {len(segments) - limit} more phase windows omitted")


def print_gap_windows(gaps: list[GapWindow], limit: int) -> None:
    print("\nlong_gaps:")
    if not gaps:
        print("  - none")
        return

    for gap in gaps[:limit]:
        print(
            "  - "
            f"gap={fmt_ms(gap.gap_ms)} "
            f"left=({format_row_ref(gap.left_row)} summary={short_payload(gap.left_row.get('payload'))}) "
            f"right=({format_row_ref(gap.right_row)} summary={short_payload(gap.right_row.get('payload'))})"
        )

    if len(gaps) > limit:
        print(f"  - ... {len(gaps) - limit} more long gaps omitted")


def print_clusters(clusters: list[ClusterWindow], limit: int, sample_limit: int) -> None:
    print("\nanomaly_windows:")
    if not clusters:
        print("  - none")
        return

    for cluster in clusters[:limit]:
        signal_counts = Counter(classify_signal(row) or "unknown" for row in cluster.rows)
        print(
            "  - "
            f"tsMs={cluster.start_ts} -> {cluster.end_ts} "
            f"duration={fmt_ms(cluster.end_ts - cluster.start_ts)} "
            f"rows={len(cluster.rows)} "
            f"signals={', '.join(f'{name}={count}' for name, count in signal_counts.most_common(5))}"
        )
        for row in cluster.rows[:sample_limit]:
            signal = classify_signal(row)
            print(
                "    - "
                f"{format_row_ref(row)} "
                f"signal={signal or 'anchor'} "
                f"summary={short_payload(row.get('payload'))}"
            )
        if len(cluster.rows) > sample_limit:
            print(f"    - ... {len(cluster.rows) - sample_limit} more rows omitted")

    if len(clusters) > limit:
        print(f"  - ... {len(clusters) - limit} more anomaly windows omitted")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Summarize runtime trace logs with phase windows and anomaly windows."
    )
    parser.add_argument("trace", help="trace jsonl file")
    parser.add_argument("--session-id", help="focus on one sessionId")
    parser.add_argument("--domain", help="focus on one domain")
    parser.add_argument("--gap-threshold-ms", type=int, default=1000, help="minimum gap to report")
    parser.add_argument(
        "--cluster-window-ms",
        type=int,
        default=2000,
        help="merge suspicious rows within this time window",
    )
    parser.add_argument("--top-events", type=int, default=20)
    parser.add_argument("--top-suspicious", type=int, default=30)
    parser.add_argument("--max-phase-windows", type=int, default=12)
    parser.add_argument("--max-gap-windows", type=int, default=12)
    parser.add_argument("--max-anomaly-windows", type=int, default=8)
    parser.add_argument("--sample-rows", type=int, default=5)
    return parser.parse_args()


def main() -> int:
    args = parse_args()

    path = Path(args.trace)
    if not path.is_file():
        print(f"trace file not found: {path}", file=sys.stderr)
        return 2

    rows: list[dict[str, Any]] = []
    with path.open("r", encoding="utf-8") as handle:
        for line_no, line in enumerate(handle, start=1):
            line = line.strip()
            if not line:
                continue
            try:
                row = json.loads(line)
            except json.JSONDecodeError as exc:
                print(f"invalid json at line {line_no}: {exc}", file=sys.stderr)
                return 1
            if not isinstance(row, dict):
                continue
            if not is_focus_row(row, args.session_id, args.domain):
                continue
            rows.append(row)

    if not rows:
        print(f"file: {path}")
        print("rows: 0")
        if args.session_id or args.domain:
            print(
                "filters: "
                + ", ".join(
                    part
                    for part in (
                        f"sessionId={args.session_id}" if args.session_id else None,
                        f"domain={args.domain}" if args.domain else None,
                    )
                    if part
                )
            )
        return 0

    category_counts = Counter(str(row.get("category", "unknown")) for row in rows)
    domain_counts = Counter(str(row.get("domain", "unknown")) for row in rows)
    event_counts = Counter(
        f"{row.get('category', 'unknown')}/{row.get('domain', 'unknown')}/{row.get('event', 'unknown')}"
        for row in rows
    )
    session_counts = Counter(str(row.get("sessionId")) for row in rows if row.get("sessionId"))
    log_levels = Counter(
        str(row.get("payload", {}).get("level", "unknown"))
        for row in rows
        if row.get("category") == "log" and isinstance(row.get("payload"), dict)
    )

    first_ts = next((row_ts(row) for row in rows if row_ts(row) is not None), None)
    last_ts = next((row_ts(row) for row in reversed(rows) if row_ts(row) is not None), None)
    duration_ms = None
    if isinstance(first_ts, int) and isinstance(last_ts, int):
        duration_ms = last_ts - first_ts

    suspicious_rows = [row for row in rows if is_suspicious(row)]
    phase_segments = build_phase_segments(rows)
    long_gaps = find_long_gaps(rows, args.gap_threshold_ms)
    anomaly_windows = cluster_signal_rows(rows, args.cluster_window_ms)

    print(f"file: {path}")
    if args.session_id or args.domain:
        filters = [
            f"sessionId={args.session_id}" if args.session_id else None,
            f"domain={args.domain}" if args.domain else None,
        ]
        print("filters: " + ", ".join(filter(None, filters)))
    print(f"rows: {len(rows)}")
    print(f"time_range_ms: {first_ts} -> {last_ts} (duration={fmt_ms(duration_ms)})")
    print(
        "categories: "
        + ", ".join(f"{name}={count}" for name, count in category_counts.most_common())
    )
    print("domains: " + ", ".join(f"{name}={count}" for name, count in domain_counts.most_common(12)))
    if session_counts:
        print(
            "sessions: "
            + ", ".join(f"{name}={count}" for name, count in session_counts.most_common(8))
        )
    if log_levels:
        print(
            "log_levels: "
            + ", ".join(f"{name}={count}" for name, count in log_levels.most_common())
        )

    print("\nphase_anchors:")
    if phase_segments:
        print(f"  - segments={len(phase_segments)}")
        for segment in phase_segments[: args.max_phase_windows]:
            print(
                "  - "
                f"{format_row_ref(segment.start_row)} -> {format_row_ref(segment.end_row)} "
                f"phase={segment.signature}"
            )
            if segment.detail:
                print(f"    detail={segment.detail}")
    else:
        print("  - none")

    print_phase_segments(phase_segments, args.max_phase_windows)
    print_gap_windows(long_gaps, args.max_gap_windows)
    print_clusters(anomaly_windows, args.max_anomaly_windows, args.sample_rows)

    print("\ntop_events:")
    for name, count in event_counts.most_common(args.top_events):
        print(f"  - {count:5d} {name}")

    print("\nsuspicious_rows:")
    # 这里保留首批异常线索，先把可疑窗口和上下文缩小，再回看原始 trace。
    for row in suspicious_rows[: args.top_suspicious]:
        signal = classify_signal(row)
        print(
            "  - "
            f"{format_row_ref(row)} "
            f"signal={signal or 'suspicious'} "
            f"session={row.get('sessionId')} "
            f"summary={short_payload(row.get('payload'))}"
        )

    if len(suspicious_rows) > args.top_suspicious:
        print(f"  - ... {len(suspicious_rows) - args.top_suspicious} more suspicious rows omitted")

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
