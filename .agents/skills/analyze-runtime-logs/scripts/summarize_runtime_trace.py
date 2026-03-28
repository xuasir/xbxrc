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

PHASE_ALIASES = {
    "startup": ("connecting", "provisioning", "waiting"),
    "connect": ("connecting", "checking"),
    "playing": ("playing", "steady", "healthy", "active", "running"),
    "stall": ("stall", "stalled", "pipelineStall", "decoderStall", "rendererStall"),
    "recovery": ("recover", "repair", "reset", "keyframe", "nack", "recovery"),
}


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


@dataclass
class TraceProfile:
    path: Path
    rows: list[dict[str, Any]]
    first_ts: int | None
    last_ts: int | None
    duration_ms: int | None
    category_counts: Counter[str]
    domain_counts: Counter[str]
    event_counts: Counter[str]
    session_counts: Counter[str]
    log_levels: Counter[str]
    suspicious_rows: list[dict[str, Any]]
    phase_segments: list[PhaseSegment]
    long_gaps: list[GapWindow]
    anomaly_windows: list[ClusterWindow]
    phase_signature_counts: Counter[str]
    anomaly_signal_counts: Counter[str]
    metric_stats: dict[str, dict[str, Any]]


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


def classify_phase_label(row: dict[str, Any]) -> str | None:
    payload = row.get("payload")
    if not isinstance(payload, dict):
        return None

    phase = payload_get(payload, "session_phase", "sessionPhase")
    if isinstance(phase, str) and phase:
        phase_lower = phase.lower()
        if phase_lower == "connecting":
            return "connect"
        if phase_lower in {"playing", "steady", "healthy", "active", "running"}:
            return "playing"
        if any(hint in phase_lower for hint in ("stall", "stalled")):
            return "stall"
        if any(hint in phase_lower for hint in ("recover", "repair", "reset", "keyframe", "nack")):
            return "recovery"
        return phase_lower

    video_health = payload_get(payload, "video_health", "videoHealth")
    if isinstance(video_health, str) and video_health:
        health_lower = video_health.lower()
        if health_lower == "connecting":
            return "connect"
        if health_lower in {"healthy", "ready", "active"}:
            return "playing"
        if any(hint in health_lower for hint in ("stall", "stalled")):
            return "stall"
        if any(hint in health_lower for hint in ("recover", "repair", "reset")):
            return "recovery"
        return health_lower

    transport_state = payload_get(payload, "transport_state", "transportState")
    if isinstance(transport_state, str) and transport_state:
        transport_lower = transport_state.lower()
        if transport_lower in {"new", "connecting", "checking"}:
            return "connect"
        if transport_lower in {"connected", "completed"}:
            return "playing"
        return transport_lower

    primary_issue_chain = payload_get(payload, "primary_issue_chain", "primaryIssueChain")
    if isinstance(primary_issue_chain, str) and primary_issue_chain:
        chain_lower = primary_issue_chain.lower()
        for alias, needles in PHASE_ALIASES.items():
            if any(needle.lower() in chain_lower for needle in needles):
                return alias
        return chain_lower

    return None


def is_phase_match(row: dict[str, Any], phase_filter: str | None) -> bool:
    if not phase_filter:
        return True
    label = classify_phase_label(row)
    if label is None:
        return False
    if label == phase_filter:
        return True
    needles = PHASE_ALIASES.get(phase_filter, ())
    if any(needle.lower() in label for needle in needles):
        return True
    return any(needle.lower() in label for needle in PHASE_ALIASES.get(label, ()))


def is_metric_field(name: str) -> bool:
    return name in {
        "presentAgeMs",
        "decodeAgeMs",
        "packetAgeMs",
        "packetToDecodeMs",
        "packetToPresentMs",
        "presentFps",
        "decodeFps",
        "videoDecoderHardwareFailureStreak",
        "videoPresentDropCountTotal",
        "videoRendererDropCountTotal",
        "videoPacerDropCountTotal",
        "recoveryKeyframeRequestCount",
        "recoveryDecoderResetCount",
        "recoveryReconnectCount",
    }


def extract_numeric_metric(payload: Any, metric: str) -> float | None:
    if not isinstance(payload, dict):
        return None
    value = payload_get(payload, metric, metric)
    if isinstance(value, (int, float)):
        return float(value)
    return None


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


def parse_time_filters(value: str | None) -> list[tuple[int | None, int | None]] | None:
    if not value:
        return []

    filters: list[tuple[int | None, int | None]] = []
    for item in value.split(","):
        item = item.strip()
        if not item:
            continue
        try:
            if "-" in item:
                start_text, end_text = item.split("-", 1)
                start_ts = int(start_text) if start_text.strip() else None
                end_ts = int(end_text) if end_text.strip() else None
            else:
                start_ts = int(item)
                end_ts = int(item)
        except ValueError:
            return None
        filters.append((start_ts, end_ts))
    return filters


def describe_time_filters(filters: list[tuple[int | None, int | None]]) -> str:
    if not filters:
        return ""
    parts: list[str] = []
    for start_ts, end_ts in filters:
        parts.append(f"{start_ts or ''}-{end_ts or ''}")
    return ",".join(parts)


def is_within_time_filters(ts_ms: int | None, filters: list[tuple[int | None, int | None]]) -> bool:
    if not filters:
        return True
    if ts_ms is None:
        return False
    for start_ts, end_ts in filters:
        if start_ts is not None and ts_ms < start_ts:
            continue
        if end_ts is not None and ts_ms > end_ts:
            continue
        return True
    return False


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


def build_trace_profile(
    path: Path,
    rows: list[dict[str, Any]],
    *,
    gap_threshold_ms: int,
    cluster_window_ms: int,
    metric_name: str | None = None,
) -> TraceProfile:
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
    suspicious_rows = [row for row in rows if is_suspicious(row)]
    phase_segments = build_phase_segments(rows)
    long_gaps = find_long_gaps(rows, gap_threshold_ms)
    anomaly_windows = cluster_signal_rows(rows, cluster_window_ms)

    phase_signature_counts = Counter(segment.signature for segment in phase_segments)
    anomaly_signal_counts = Counter(
        classify_signal(row) or "unknown" for cluster in anomaly_windows for row in cluster.rows
    )
    metric_stats: dict[str, dict[str, Any]] = {}
    if metric_name:
        samples: list[tuple[int, float]] = []
        for row in rows:
            payload = row.get("payload")
            value = extract_numeric_metric(payload, metric_name)
            ts = row_ts(row)
            if value is not None and ts is not None:
                samples.append((ts, value))
        if samples:
            values = [value for _, value in samples]
            metric_stats[metric_name] = {
                "samples": len(samples),
                "min": min(values),
                "max": max(values),
                "avg": sum(values) / len(values),
                "first": samples[0][1],
                "last": samples[-1][1],
                "peaks": [
                    {
                        "ts": ts,
                        "value": value,
                    }
                    for ts, value in sorted(samples, key=lambda item: item[1], reverse=True)[:5]
                ],
            }

    first_ts = next((row_ts(row) for row in rows if row_ts(row) is not None), None)
    last_ts = next((row_ts(row) for row in reversed(rows) if row_ts(row) is not None), None)
    duration_ms = None
    if isinstance(first_ts, int) and isinstance(last_ts, int):
        duration_ms = last_ts - first_ts

    return TraceProfile(
        path=path,
        rows=rows,
        first_ts=first_ts,
        last_ts=last_ts,
        duration_ms=duration_ms,
        category_counts=category_counts,
        domain_counts=domain_counts,
        event_counts=event_counts,
        session_counts=session_counts,
        log_levels=log_levels,
        suspicious_rows=suspicious_rows,
        phase_segments=phase_segments,
        long_gaps=long_gaps,
        anomaly_windows=anomaly_windows,
        phase_signature_counts=phase_signature_counts,
        anomaly_signal_counts=anomaly_signal_counts,
        metric_stats=metric_stats,
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


def print_profile(profile: TraceProfile, args: argparse.Namespace) -> None:
    print(f"file: {profile.path}")
    if args.session_id or args.domain or args.time_window or args.phase or args.metric:
        filters = []
        if args.session_id:
            filters.append(f"sessionId={args.session_id}")
        if args.domain:
            filters.append(f"domain={args.domain}")
        if args.phase:
            filters.append(f"phase={args.phase}")
        if args.metric:
            filters.append(f"metric={args.metric}")
        if args.time_window:
            filters.append(f"time_window={args.time_window}")
        print("filters: " + ", ".join(filters))
    print(f"rows: {len(profile.rows)}")
    if len(profile.rows) == 0:
        print("note: no rows matched the selected filters")
    print(
        f"time_range_ms: {profile.first_ts} -> {profile.last_ts} "
        f"(duration={fmt_ms(profile.duration_ms)})"
    )
    print(
        "categories: "
        + ", ".join(f"{name}={count}" for name, count in profile.category_counts.most_common())
    )
    print("domains: " + ", ".join(f"{name}={count}" for name, count in profile.domain_counts.most_common(12)))
    if profile.session_counts:
        print(
            "sessions: "
            + ", ".join(f"{name}={count}" for name, count in profile.session_counts.most_common(8))
        )
    if profile.log_levels:
        print(
            "log_levels: "
            + ", ".join(f"{name}={count}" for name, count in profile.log_levels.most_common())
        )

    print("\nphase_anchors:")
    if profile.phase_segments:
        print(f"  - segments={len(profile.phase_segments)}")
        for segment in profile.phase_segments[: args.max_phase_windows]:
            print(
                "  - "
                f"{format_row_ref(segment.start_row)} -> {format_row_ref(segment.end_row)} "
                f"phase={segment.signature}"
            )
            if segment.detail:
                print(f"    detail={segment.detail}")
    else:
        print("  - none")

    print_phase_segments(profile.phase_segments, args.max_phase_windows)
    print_gap_windows(profile.long_gaps, args.max_gap_windows)
    print_clusters(profile.anomaly_windows, args.max_anomaly_windows, args.sample_rows)

    print("\ntop_events:")
    for name, count in profile.event_counts.most_common(args.top_events):
        print(f"  - {count:5d} {name}")

    print("\nsuspicious_rows:")
    # 这里保留首批异常线索，先把可疑窗口和上下文缩小，再回看原始 trace。
    for row in profile.suspicious_rows[: args.top_suspicious]:
        signal = classify_signal(row)
        print(
            "  - "
            f"{format_row_ref(row)} "
            f"signal={signal or 'suspicious'} "
            f"session={row.get('sessionId')} "
            f"summary={short_payload(row.get('payload'))}"
        )

    if len(profile.suspicious_rows) > args.top_suspicious:
        print(f"  - ... {len(profile.suspicious_rows) - args.top_suspicious} more suspicious rows omitted")

    if args.metric:
        print("\nmetric_summary:")
        metric = profile.metric_stats.get(args.metric)
        if not metric:
            print("  - no samples")
        else:
            print(
                "  - "
                f"samples={metric['samples']} min={metric['min']:.3f} max={metric['max']:.3f} "
                f"avg={metric['avg']:.3f} first={metric['first']:.3f} last={metric['last']:.3f}"
            )
            for peak in metric["peaks"]:
                print(f"    - peak ts={peak['ts']} value={peak['value']:.3f}")


def print_trace_comparison(base: TraceProfile, compare: TraceProfile) -> None:
    print("\ntrace_comparison:")
    print(
        "  - "
        f"base_rows={len(base.rows)} compare_rows={len(compare.rows)} "
        f"base_duration={fmt_ms(base.duration_ms)} compare_duration={fmt_ms(compare.duration_ms)}"
    )

    base_phase_total = sum(base.phase_signature_counts.values())
    compare_phase_total = sum(compare.phase_signature_counts.values())
    print(
        "  - "
        f"phase_windows base={base_phase_total} compare={compare_phase_total} "
        f"delta={compare_phase_total - base_phase_total:+d}"
    )

    base_anomaly_total = sum(base.anomaly_signal_counts.values())
    compare_anomaly_total = sum(compare.anomaly_signal_counts.values())
    print(
        "  - "
        f"anomaly_signals base={base_anomaly_total} compare={compare_anomaly_total} "
        f"delta={compare_anomaly_total - base_anomaly_total:+d}"
    )

    print("  - top phase signatures delta:")
    all_phase_signatures = set(base.phase_signature_counts) | set(compare.phase_signature_counts)
    for signature in sorted(
        all_phase_signatures,
        key=lambda item: abs(compare.phase_signature_counts.get(item, 0) - base.phase_signature_counts.get(item, 0)),
        reverse=True,
    )[:8]:
        base_count = base.phase_signature_counts.get(signature, 0)
        compare_count = compare.phase_signature_counts.get(signature, 0)
        delta = compare_count - base_count
        if delta == 0:
            continue
        print(f"    - {signature}: base={base_count} compare={compare_count} delta={delta:+d}")

    print("  - top anomaly signal delta:")
    all_signals = set(base.anomaly_signal_counts) | set(compare.anomaly_signal_counts)
    for signal in sorted(
        all_signals,
        key=lambda item: abs(compare.anomaly_signal_counts.get(item, 0) - base.anomaly_signal_counts.get(item, 0)),
        reverse=True,
    )[:8]:
        base_count = base.anomaly_signal_counts.get(signal, 0)
        compare_count = compare.anomaly_signal_counts.get(signal, 0)
        delta = compare_count - base_count
        if delta == 0:
            continue
        print(f"    - {signal}: base={base_count} compare={compare_count} delta={delta:+d}")

    if base.metric_stats or compare.metric_stats:
        print("  - metric delta:")
        metric_names = set(base.metric_stats) | set(compare.metric_stats)
        for metric_name in sorted(metric_names):
            base_metric = base.metric_stats.get(metric_name)
            compare_metric = compare.metric_stats.get(metric_name)
            if not base_metric or not compare_metric:
                continue
            print(
                f"    - {metric_name}: "
                f"base_avg={base_metric['avg']:.3f} compare_avg={compare_metric['avg']:.3f} "
                f"delta={compare_metric['avg'] - base_metric['avg']:+.3f}"
            )


def load_trace_profile(
    path: Path,
    *,
    session_id: str | None,
    domain: str | None,
    time_filters: list[tuple[int | None, int | None]],
    phase_filter: str | None,
    gap_threshold_ms: int,
    cluster_window_ms: int,
    metric_name: str | None = None,
) -> TraceProfile:
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
                raise SystemExit(1)
            if not isinstance(row, dict):
                continue
            if not is_focus_row(row, session_id, domain):
                continue
            if not is_within_time_filters(row_ts(row), time_filters):
                continue
            if not is_phase_match(row, phase_filter):
                continue
            rows.append(row)

    return build_trace_profile(
        path,
        rows,
        gap_threshold_ms=gap_threshold_ms,
        cluster_window_ms=cluster_window_ms,
        metric_name=metric_name,
    )


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Summarize runtime trace logs with phase windows and anomaly windows."
    )
    parser.add_argument("trace", help="trace jsonl file")
    parser.add_argument("--compare", help="another trace jsonl file for comparison")
    parser.add_argument("--session-id", help="focus on one sessionId")
    parser.add_argument("--domain", help="focus on one domain")
    parser.add_argument("--phase", help="focus on a semantic phase such as startup, playing, stall, recovery")
    parser.add_argument("--metric", help="focus on a numeric metric field, e.g. presentAgeMs or packetAgeMs")
    parser.add_argument(
        "--time-window",
        help="time filter, format start-end or single timestamp, comma separated",
    )
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

    time_filters = parse_time_filters(args.time_window)
    if args.time_window and time_filters is None:
        print("invalid time window filter", file=sys.stderr)
        return 2

    if args.metric and not is_metric_field(args.metric):
        print(f"unsupported metric field: {args.metric}", file=sys.stderr)
        return 2

    profile = load_trace_profile(
        path,
        session_id=args.session_id,
        domain=args.domain,
        time_filters=time_filters,
        phase_filter=args.phase,
        gap_threshold_ms=args.gap_threshold_ms,
        cluster_window_ms=args.cluster_window_ms,
        metric_name=args.metric,
    )
    print_profile(profile, args)

    if args.compare:
        compare_path = Path(args.compare)
        if not compare_path.is_file():
            print(f"compare trace file not found: {compare_path}", file=sys.stderr)
            return 2
        compare_profile = load_trace_profile(
            compare_path,
            session_id=args.session_id,
            domain=args.domain,
            time_filters=time_filters,
            phase_filter=args.phase,
            gap_threshold_ms=args.gap_threshold_ms,
            cluster_window_ms=args.cluster_window_ms,
            metric_name=args.metric,
        )
        print_trace_comparison(profile, compare_profile)

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
