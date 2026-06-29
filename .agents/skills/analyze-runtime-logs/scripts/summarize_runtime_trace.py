#!/usr/bin/env python3

from __future__ import annotations

import argparse
import json
import re
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
HEALTHY_CHAIN_STATES = {"healthy", "steady"}
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
    trace_mode_counts: Counter[str]
    trace_profile_counts: Counter[str]
    dimension_counts: Counter[str]
    importance_counts: Counter[str]
    trace_budget_notice_count: int
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
    recovery_audit: dict[str, Any]


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


def normalize_text(value: Any) -> str | None:
    if not isinstance(value, str):
        return None
    stripped = value.strip()
    return stripped if stripped else None


def normalize_state(value: Any) -> str | None:
    text = normalize_text(value)
    if text is None:
        return None
    return text.lower().replace("_", "-")


def is_failed_terminal(value: Any) -> bool:
    state = normalize_state(value)
    return state in {"failed-terminal", "failedterminal", "terminal-failed"}


def extract_terminal_reason(*candidates: Any) -> str | None:
    for candidate in candidates:
        text = normalize_text(candidate)
        if not text:
            continue
        match = re.search(r"terminal:([A-Za-z0-9_-]+)", text)
        if match:
            return match.group(1)
    return None


def extract_transport_state(row: dict[str, Any]) -> str | None:
    payload = row.get("payload")
    if not isinstance(payload, dict):
        return None
    return normalize_state(payload_get(payload, "transportState", "transport_state", "state"))


def extract_recovery_ledger(row: dict[str, Any]) -> dict[str, Any] | None:
    if str(row.get("event", "")) != "recoveryDecisionLedger":
        return None
    payload = row.get("payload")
    if not isinstance(payload, dict):
        return None
    return payload


def extract_successful_action_sample(row: dict[str, Any]) -> tuple[int, int] | None:
    payload = row.get("payload")
    ts = row_ts(row)
    if not isinstance(payload, dict) or ts is None:
        return None
    value = payload_get(payload, "successful_action_count", "successfulActionCount")
    if isinstance(value, int):
        return ts, value
    return None


def event_payload(row: dict[str, Any], expected_event: str) -> dict[str, Any] | None:
    if str(row.get("event", "")) != expected_event:
        return None
    payload = row.get("payload")
    if not isinstance(payload, dict):
        return None
    return payload


def get_int(payload: dict[str, Any], *keys: str) -> int | None:
    value = payload_get(payload, *keys)
    return value if isinstance(value, int) else None


def get_number(payload: dict[str, Any], *keys: str) -> float | int | None:
    value = payload_get(payload, *keys)
    return value if isinstance(value, (int, float)) else None


def safe_ratio(numerator: int, denominator: int) -> float | None:
    if denominator <= 0:
        return None
    return numerator / denominator


def round_score(value: float) -> float:
    return round(value, 4)


def average_number(values: list[float | int]) -> float | None:
    if not values:
        return None
    return round_score(sum(float(value) for value in values) / len(values))


def format_boolish(value: Any) -> str:
    if value is True:
        return "true"
    if value is False:
        return "false"
    if value is None:
        return "null"
    return str(value)


def extract_repairability_value(payload: Any) -> float | int | None:
    if not isinstance(payload, dict):
        return None
    return get_number(
        payload,
        "repairabilityScore",
        "repairability_score",
        "repairability",
        "repairabilityIndex",
        "repairability_index",
    )


def get_text(payload: dict[str, Any], *keys: str) -> str | None:
    value = payload_get(payload, *keys)
    return normalize_text(value)


def keyframe_episode_rows(rows: list[dict[str, Any]]) -> list[dict[str, Any]]:
    return [row for row in rows if event_payload(row, "keyframeRequestEpisode") is not None]


def nack_rows(rows: list[dict[str, Any]]) -> list[dict[str, Any]]:
    return [
        row
        for row in rows
        if str(row.get("event", "")) in {"nackSent", "nackRecovered", "nackSkipped", "nackExpired"}
    ]


def chain_transition_rows(rows: list[dict[str, Any]]) -> list[dict[str, Any]]:
    return [row for row in rows if event_payload(row, "videoChainTransition") is not None]


def rows_for_event(rows: list[dict[str, Any]], event_name: str) -> list[dict[str, Any]]:
    return [row for row in rows if event_payload(row, event_name) is not None]


def is_keyframe_related_ledger(payload: dict[str, Any]) -> bool:
    haystacks = [
        get_text(payload, "inputSignal", "input_signal"),
        get_text(payload, "gateResult", "gate_result"),
        get_text(payload, "actionSelected", "action_selected"),
        get_text(payload, "recoveryPrimaryAction", "recovery_primary_action"),
        get_text(payload, "commandDetail", "command_detail"),
    ]
    return any(
        text and any(needle in text.lower() for needle in ("keyframe", "transportawaitrecoverykeyframe"))
        for text in haystacks
    )


def classify_ledger_suppression(payload: dict[str, Any]) -> tuple[str, str] | None:
    gate_result = get_text(payload, "gateResult", "gate_result")
    action_selected = get_text(payload, "actionSelected", "action_selected")
    for value in (gate_result, action_selected):
        if not value:
            continue
        lowered = value.lower()
        if lowered.startswith("suppressed:"):
            return "suppressed", value.split(":", 1)[1]
        if lowered.startswith("coalesced:"):
            return "coalesced", value.split(":", 1)[1]
        if "cooldownsuppressed" in lowered:
            return "suppressed", "cooldownSuppressed"
    return None


def is_effective_chain_state(payload: dict[str, Any]) -> bool:
    chain = payload.get("chain")
    if isinstance(chain, dict):
        state = normalize_state(chain.get("state"))
        if state in HEALTHY_CHAIN_STATES:
            return True
    state = normalize_state(payload_get(payload, "state"))
    return state in HEALTHY_CHAIN_STATES


def collect_connecting_windows(rows: list[dict[str, Any]]) -> list[tuple[int, int]]:
    sorted_rows = sorted(rows, key=lambda item: (row_ts(item) or 0, row_seq(item) or 0))
    windows: list[tuple[int, int]] = []
    active_start: int | None = None
    last_state: str | None = None
    last_ts = next((row_ts(row) for row in reversed(sorted_rows) if row_ts(row) is not None), None)
    for row in sorted_rows:
        ts = row_ts(row)
        if ts is None:
            continue
        state = extract_transport_state(row)
        if state is None:
            continue
        if state == "connecting" and active_start is None:
            active_start = ts
        elif state != "connecting" and last_state == "connecting" and active_start is not None:
            windows.append((active_start, ts))
            active_start = None
        last_state = state
    if active_start is not None and last_ts is not None and last_ts >= active_start:
        windows.append((active_start, last_ts))
    return windows


def analyze_recovery_audit(rows: list[dict[str, Any]], silence_threshold_ms: int) -> dict[str, Any]:
    sorted_rows = sorted(rows, key=lambda item: (row_ts(item) or 0, row_seq(item) or 0))
    picture_recovery_transitions = rows_for_event(sorted_rows, "pictureRecoveryTransition")
    picture_recovery_blockers = rows_for_event(sorted_rows, "pictureRecoveryBlockerObserved")
    video_ingress_terminations = rows_for_event(sorted_rows, "videoIngressTermination")
    first_frame_latencies = rows_for_event(sorted_rows, "firstFrameLatencyObserved")
    h264_inspection_rows = [
        row
        for row in sorted_rows
        if str(row.get("event", "")) in {"h264InspectionObserved", "h264InspectionRejected"}
    ]
    ledger_rows: list[dict[str, Any]] = []
    ledger_times: list[int] = []
    failed_terminal_entries: list[dict[str, Any]] = []
    failed_terminal_reasons: Counter[str] = Counter()
    keyframe_suppression_counts: Counter[str] = Counter()
    keyframe_suppression_samples: list[dict[str, Any]] = []

    for row in sorted_rows:
        payload = extract_recovery_ledger(row)
        if payload is None:
            continue
        ts = row_ts(row)
        if ts is None:
            continue
        ledger_rows.append(row)
        ledger_times.append(ts)
        if is_keyframe_related_ledger(payload):
            suppression = classify_ledger_suppression(payload)
            if suppression is not None:
                family, detail = suppression
                keyframe_suppression_counts[f"{family}:{detail}"] += 1
                keyframe_suppression_samples.append(
                    {
                        "seq": row_seq(row),
                        "tsMs": ts,
                        "decisionId": payload_get(payload, "decisionId", "decision_id"),
                        "inputSignal": payload_get(payload, "inputSignal", "input_signal"),
                        "gateResult": payload_get(payload, "gateResult", "gate_result"),
                        "actionSelected": payload_get(payload, "actionSelected", "action_selected"),
                        "suppressionType": family,
                        "suppressionDetail": detail,
                    }
                )
        state_after = payload_get(payload, "stateAfter", "state_after")
        if not is_failed_terminal(state_after):
            continue
        reason = extract_terminal_reason(
            payload_get(payload, "gateResult", "gate_result"),
            payload_get(payload, "inputSignal", "input_signal"),
            payload_get(payload, "commandDetail", "command_detail"),
            payload_get(payload, "actionSelected", "action_selected"),
        ) or "unknown"
        failed_terminal_reasons[reason] += 1
        failed_terminal_entries.append(
            {
                "seq": row_seq(row),
                "tsMs": ts,
                "decisionId": payload_get(payload, "decisionId", "decision_id"),
                "reason": reason,
                "stateBefore": payload_get(payload, "stateBefore", "state_before"),
                "stateAfter": state_after,
                "inputSignal": payload_get(payload, "inputSignal", "input_signal"),
                "gateResult": payload_get(payload, "gateResult", "gate_result"),
                "actionSelected": payload_get(payload, "actionSelected", "action_selected"),
            }
        )

    connecting_windows = collect_connecting_windows(sorted_rows)
    silence_breaches: list[dict[str, Any]] = []
    for start_ts, end_ts in connecting_windows:
        if end_ts <= start_ts:
            continue
        in_window = [ts for ts in ledger_times if start_ts <= ts <= end_ts]
        checkpoints = [start_ts, *in_window, end_ts]
        max_gap_ms = 0
        for left, right in zip(checkpoints, checkpoints[1:]):
            max_gap_ms = max(max_gap_ms, right - left)
        if max_gap_ms >= silence_threshold_ms:
            silence_breaches.append(
                {
                    "windowStartTsMs": start_ts,
                    "windowEndTsMs": end_ts,
                    "windowDurationMs": end_ts - start_ts,
                    "maxLedgerSilenceMs": max_gap_ms,
                    "ledgerEntries": len(in_window),
                }
            )

    successful_samples = [sample for sample in (extract_successful_action_sample(row) for row in sorted_rows) if sample]
    successful_samples.sort(key=lambda item: item[0])
    successful_action_increments = 0
    for (_, left), (_, right) in zip(successful_samples, successful_samples[1:]):
        if right > left:
            successful_action_increments += 1

    unlock_evidence: list[dict[str, Any]] = []
    for failed in failed_terminal_entries:
        failed_ts = int(failed["tsMs"])
        unlocked = False
        unlock_ts_ms: int | None = None
        unlock_kind: str | None = None
        detail: str | None = None

        for row in sorted_rows:
            ts = row_ts(row)
            if ts is None or ts <= failed_ts:
                continue
            payload = extract_recovery_ledger(row)
            if payload is None:
                continue
            state_after = payload_get(payload, "stateAfter", "state_after")
            if state_after is not None and not is_failed_terminal(state_after):
                unlocked = True
                unlock_ts_ms = ts
                unlock_kind = "state-exit"
                detail = f"stateAfter={state_after}"
                break

        if not unlocked:
            baseline: int | None = None
            for ts, count in successful_samples:
                if ts <= failed_ts:
                    baseline = count
                    continue
                if baseline is not None and count > baseline:
                    unlocked = True
                    unlock_ts_ms = ts
                    unlock_kind = "successful-action-increase"
                    detail = f"{baseline}->{count}"
                    break

        unlock_evidence.append(
            {
                "failedTerminalTsMs": failed_ts,
                "unlocked": unlocked,
                "unlockTsMs": unlock_ts_ms,
                "unlockKind": unlock_kind,
                "detail": detail,
            }
        )

    transition_phase_counts: Counter[str] = Counter()
    transition_to_phase_counts: Counter[str] = Counter()
    transition_cause_counts: Counter[str] = Counter()
    transition_samples: list[dict[str, Any]] = []
    for row in picture_recovery_transitions:
        payload = event_payload(row, "pictureRecoveryTransition")
        if payload is None:
            continue
        phase = get_text(payload, "phase") or "unknown"
        to_phase = get_text(payload, "toPhase", "to_phase") or "unknown"
        cause = get_text(payload, "cause") or "unknown"
        transition_phase_counts[phase] += 1
        transition_to_phase_counts[to_phase] += 1
        transition_cause_counts[cause] += 1
        transition_samples.append(
            {
                "seq": row_seq(row),
                "tsMs": row_ts(row),
                "episodeId": get_int(payload, "episodeId", "episode_id"),
                "recoveryEpoch": get_int(payload, "recoveryEpoch", "recovery_epoch"),
                "phase": phase,
                "fromPhase": get_text(payload, "fromPhase", "from_phase"),
                "toPhase": to_phase,
                "cause": cause,
                "detail": get_text(payload, "detail"),
                "ownerState": get_text(payload, "ownerState", "owner_state"),
                "transportState": get_text(payload, "transportState", "transport_state"),
            }
        )

    blocker_gate_counts: Counter[str] = Counter()
    blocker_kind_counts: Counter[str] = Counter()
    blocker_severity_counts: Counter[str] = Counter()
    blocker_samples: list[dict[str, Any]] = []
    for row in picture_recovery_blockers:
        payload = event_payload(row, "pictureRecoveryBlockerObserved")
        if payload is None:
            continue
        gate = get_text(payload, "gate") or "unknown"
        blocker_kind = get_text(payload, "blockerKind", "blocker_kind") or "unknown"
        severity = get_text(payload, "severity") or "unknown"
        blocker_gate_counts[gate] += 1
        blocker_kind_counts[blocker_kind] += 1
        blocker_severity_counts[severity] += 1
        blocker_samples.append(
            {
                "seq": row_seq(row),
                "tsMs": row_ts(row),
                "episodeId": get_int(payload, "episodeId", "episode_id"),
                "recoveryEpoch": get_int(payload, "recoveryEpoch", "recovery_epoch"),
                "gate": gate,
                "blockerKind": blocker_kind,
                "severity": severity,
                "firstObservedAtMs": get_int(payload, "firstObservedAtMs", "first_observed_at_ms"),
                "count": get_int(payload, "count"),
                "ownerState": get_text(payload, "ownerState", "owner_state"),
                "transportState": get_text(payload, "transportState", "transport_state"),
            }
        )

    ingress_kind_counts: Counter[str] = Counter()
    ingress_cause_counts: Counter[str] = Counter()
    ingress_upstream_cause_counts: Counter[str] = Counter()
    ingress_samples: list[dict[str, Any]] = []
    for row in video_ingress_terminations:
        payload = event_payload(row, "videoIngressTermination")
        if payload is None:
            continue
        kind = get_text(payload, "kind") or "unknown"
        cause = get_text(payload, "cause") or "unknown"
        upstream_cause = get_text(payload, "upstreamCause", "upstream_cause") or "unknown"
        ingress_kind_counts[kind] += 1
        ingress_cause_counts[cause] += 1
        ingress_upstream_cause_counts[upstream_cause] += 1
        ingress_samples.append(
            {
                "seq": row_seq(row),
                "tsMs": row_ts(row),
                "terminationId": get_int(payload, "terminationId", "termination_id"),
                "derivedFromTerminationId": get_int(
                    payload, "derivedFromTerminationId", "derived_from_termination_id"
                ),
                "kind": kind,
                "cause": cause,
                "upstreamCause": upstream_cause,
                "sourceSubsystem": get_text(payload, "sourceSubsystem", "source_subsystem"),
                "linkedRecoveryEpoch": get_int(payload, "linkedRecoveryEpoch", "linked_recovery_epoch"),
                "linkedEpisodeId": get_int(payload, "linkedEpisodeId", "linked_episode_id"),
                "ownerState": get_text(payload, "ownerState", "owner_state"),
                "transportState": get_text(payload, "transportState", "transport_state"),
            }
        )

    first_frame_terminal_phase_counts: Counter[str] = Counter()
    first_frame_incomplete_reason_counts: Counter[str] = Counter()
    first_frame_samples: list[dict[str, Any]] = []
    first_frame_control_ready_to_pli: list[float | int] = []
    first_frame_pli_to_idr: list[float | int] = []
    first_frame_idr_to_decode: list[float | int] = []
    first_frame_decode_to_clean_anchor: list[float | int] = []
    first_frame_clean_anchor_to_display: list[float | int] = []
    for row in first_frame_latencies:
        payload = event_payload(row, "firstFrameLatencyObserved")
        if payload is None:
            continue
        terminal_phase = get_text(payload, "terminalPhase", "terminal_phase") or "unknown"
        incomplete_reason = get_text(payload, "incompleteReason", "incomplete_reason")
        control_ready_to_pli = get_number(
            payload, "controlReadyToPliSentMs", "control_ready_to_pli_sent_ms"
        )
        pli_to_idr = get_number(payload, "pliSentToFirstIdrPacketMs", "pli_sent_to_first_idr_packet_ms")
        idr_to_decode = get_number(
            payload, "firstIdrPacketToFirstDecodeMs", "first_idr_packet_to_first_decode_ms"
        )
        decode_to_clean_anchor = get_number(
            payload,
            "firstDecodeToCleanAnchorCommittedMs",
            "first_decode_to_clean_anchor_committed_ms",
        )
        clean_anchor_to_display = get_number(
            payload,
            "cleanAnchorCommittedToDisplayStableMs",
            "clean_anchor_committed_to_display_stable_ms",
        )
        first_frame_terminal_phase_counts[terminal_phase] += 1
        if incomplete_reason:
            first_frame_incomplete_reason_counts[incomplete_reason] += 1
        if control_ready_to_pli is not None:
            first_frame_control_ready_to_pli.append(control_ready_to_pli)
        if pli_to_idr is not None:
            first_frame_pli_to_idr.append(pli_to_idr)
        if idr_to_decode is not None:
            first_frame_idr_to_decode.append(idr_to_decode)
        if decode_to_clean_anchor is not None:
            first_frame_decode_to_clean_anchor.append(decode_to_clean_anchor)
        if clean_anchor_to_display is not None:
            first_frame_clean_anchor_to_display.append(clean_anchor_to_display)
        first_frame_samples.append(
            {
                "seq": row_seq(row),
                "tsMs": row_ts(row),
                "episodeId": get_int(payload, "episodeId", "episode_id"),
                "recoveryEpoch": get_int(payload, "recoveryEpoch", "recovery_epoch"),
                "controlReadyToPliSentMs": control_ready_to_pli,
                "pliSentToFirstIdrPacketMs": pli_to_idr,
                "firstIdrPacketToFirstDecodeMs": idr_to_decode,
                "firstDecodeToCleanAnchorCommittedMs": decode_to_clean_anchor,
                "cleanAnchorCommittedToDisplayStableMs": clean_anchor_to_display,
                "terminalPhase": terminal_phase,
                "incompleteReason": incomplete_reason,
            }
        )

    h264_reject_classification_counts: Counter[str] = Counter()
    h264_bootstrap_reject_reason_counts: Counter[str] = Counter()
    h264_continuation_profile_counts: Counter[str] = Counter()
    h264_accepted_count = 0
    h264_post_recovery_degradation_count = 0
    h264_samples: list[dict[str, Any]] = []
    for row in h264_inspection_rows:
        payload = row.get("payload")
        if not isinstance(payload, dict):
            continue
        reject_classification = get_text(payload, "rejectClassification", "reject_classification")
        bootstrap_reject_reason = get_text(payload, "bootstrapRejectReason", "bootstrap_reject_reason")
        if reject_classification:
            h264_reject_classification_counts[reject_classification] += 1
        if bootstrap_reject_reason:
            h264_bootstrap_reject_reason_counts[bootstrap_reject_reason] += 1
        else:
            h264_accepted_count += 1
            h264_bootstrap_reject_reason_counts["accepted"] += 1
        if reject_classification and "continuation" in reject_classification.lower():
            continuation_profile = ",".join(
                [
                    f"admissionAccepted={format_boolish(payload_get(payload, 'admissionAccepted', 'admission_accepted'))}",
                    f"isIdr={format_boolish(payload_get(payload, 'isIdr', 'is_idr'))}",
                    f"deltaContinuationReady={format_boolish(payload_get(payload, 'deltaContinuationReady', 'delta_continuation_ready'))}",
                    f"committedSpsPresent={format_boolish(payload_get(payload, 'committedSpsPresent', 'committed_sps_present'))}",
                    f"committedPpsPresent={format_boolish(payload_get(payload, 'committedPpsPresent', 'committed_pps_present'))}",
                ]
            )
            h264_continuation_profile_counts[continuation_profile] += 1
        if payload_get(payload, "isPostRecoveryDegradation", "is_post_recovery_degradation") is True:
            h264_post_recovery_degradation_count += 1
        h264_samples.append(
            {
                "seq": row_seq(row),
                "tsMs": row_ts(row),
                "event": row.get("event"),
                "boundEpisodeId": get_int(payload, "boundEpisodeId", "bound_episode_id"),
                "boundRecoveryEpoch": get_int(payload, "boundRecoveryEpoch", "bound_recovery_epoch"),
                "episodePhaseAtObservation": get_text(
                    payload, "episodePhaseAtObservation", "episode_phase_at_observation"
                ),
                "admissionAccepted": payload_get(payload, "admissionAccepted", "admission_accepted"),
                "isIdr": payload_get(payload, "isIdr", "is_idr"),
                "bootstrapRejectReason": bootstrap_reject_reason,
                "rejectClassification": reject_classification,
                "isPostRecoveryDegradation": payload_get(
                    payload, "isPostRecoveryDegradation", "is_post_recovery_degradation"
                ),
            }
        )

    twcc_mapping_missing_rows = rows_for_event(sorted_rows, "twccReceiverMappingMissing")
    twcc_inbound_seen_rows = rows_for_event(sorted_rows, "twccInboundExtensionSeen")
    feedback_target_availability_rows = rows_for_event(sorted_rows, "feedbackTargetAvailabilityChanged")
    host_mailbox_state_rows = rows_for_event(sorted_rows, "hostMailboxState")
    frame_drop_event_rows = rows_for_event(sorted_rows, "frameDropped") + rows_for_event(
        sorted_rows, "frameDeadlineMissed"
    )
    feedback_target_state_counts: Counter[str] = Counter()
    feedback_target_reason_counts: Counter[str] = Counter()
    for row in feedback_target_availability_rows:
        payload = event_payload(row, "feedbackTargetAvailabilityChanged")
        if payload is None:
            continue
        state = get_text(payload, "state") or "unknown"
        reason = get_text(payload, "reason") or "unknown"
        feedback_target_state_counts[state] += 1
        feedback_target_reason_counts[reason] += 1

    host_present_cadence_phase_counts: Counter[str] = Counter()
    host_present_no_pending_pressure_level_counts: Counter[str] = Counter()
    displayed_frame_stale_count = 0
    retained_old_frame_risk_count = 0
    host_present_samples: list[dict[str, Any]] = []
    for row in host_mailbox_state_rows:
        payload = event_payload(row, "hostMailboxState")
        if payload is None:
            continue
        cadence_phase = get_text(payload, "cadencePhase", "cadence_phase") or "unknown"
        no_pending_pressure_level = (
            get_text(payload, "noPendingPressureLevel", "no_pending_pressure_level") or "unknown"
        )
        displayed_frame_stale = payload_get(payload, "displayedFrameStale", "displayed_frame_stale") is True
        retained_old_frame_risk = payload_get(payload, "retainedOldFrameRisk", "retained_old_frame_risk") is True
        host_present_cadence_phase_counts[cadence_phase] += 1
        host_present_no_pending_pressure_level_counts[no_pending_pressure_level] += 1
        if displayed_frame_stale:
            displayed_frame_stale_count += 1
        if retained_old_frame_risk:
            retained_old_frame_risk_count += 1
        host_present_samples.append(
            {
                "seq": row_seq(row),
                "tsMs": row_ts(row),
                "cadencePhase": cadence_phase,
                "noPendingPressureLevel": no_pending_pressure_level,
                "displayedFrameStale": displayed_frame_stale,
                "retainedOldFrameRisk": retained_old_frame_risk,
                "presentAgeMs": get_number(payload, "presentAgeMs", "present_age_ms"),
                "lastDisplayedFrameSeq": get_int(
                    payload, "lastDisplayedFrameSeq", "last_displayed_frame_seq"
                ),
            }
        )

    frame_drop_reason_counts: Counter[str] = Counter()
    frame_drop_stage_counts: Counter[str] = Counter()
    frame_drop_detail_counts: Counter[str] = Counter()
    scheduled_frame_stale_count = 0
    submitted_frame_stale_count = 0
    recovery_valued_frame_drop_count = 0
    frame_drop_samples: list[dict[str, Any]] = []
    for row in frame_drop_event_rows:
        payload = row.get("payload")
        if not isinstance(payload, dict):
            continue
        reason = get_text(payload, "reason") or "unknown"
        stage = get_text(payload, "stage") or "unknown"
        detail = get_text(payload, "detail") or "unknown"
        recovery_disposition = get_text(
            payload, "frameRecoveryDisposition", "frame_recovery_disposition"
        )
        is_keyframe = payload_get(payload, "isKeyframe", "is_keyframe") is True
        frame_drop_reason_counts[reason] += 1
        frame_drop_stage_counts[stage] += 1
        frame_drop_detail_counts[detail] += 1
        if detail == "scheduledFrameStale":
            scheduled_frame_stale_count += 1
        if detail == "submittedFrameStale":
            submitted_frame_stale_count += 1
        if is_keyframe or recovery_disposition in {"repairing", "rebuilding", "rebuilding-supply"}:
            recovery_valued_frame_drop_count += 1
        frame_drop_samples.append(
            {
                "seq": row_seq(row),
                "tsMs": row_ts(row),
                "event": row.get("event"),
                "reason": reason,
                "stage": stage,
                "detail": detail,
                "frameRecoveryDisposition": recovery_disposition,
                "frameSeq": get_int(payload, "frameSeq", "frame_seq"),
                "frameRtpTimestamp": get_int(payload, "frameRtpTimestamp", "frame_rtp_timestamp"),
                "isKeyframe": is_keyframe,
            }
        )

    keyframe_episodes: dict[int, dict[str, Any]] = {}
    for row in keyframe_episode_rows(sorted_rows):
        payload = event_payload(row, "keyframeRequestEpisode")
        if payload is None:
            continue
        episode_id = get_int(payload, "episodeId", "episode_id")
        if episode_id is None:
            continue
        episode = keyframe_episodes.setdefault(
            episode_id,
            {
                "episodeId": episode_id,
                "seq": row_seq(row),
                "rowTsMs": row_ts(row),
                "requestReason": get_text(payload, "requestReason", "request_reason"),
                "requestKind": get_text(payload, "requestKind", "request_kind"),
                "lifecyclePhase": None,
                "statuses": [],
                "finalStatus": None,
                "statusDetail": None,
                "responseVerdict": None,
                "timedOut": False,
                "requestedAtMs": None,
                "sentAtMs": None,
                "deadlineAtMs": None,
                "retiredAtMs": None,
                "firstVideoPacketAtMs": None,
                "firstKeyframePacketAtMs": None,
                "firstKeyframeDecodedAtMs": None,
                "requestToFirstPacketMs": None,
                "requestToFirstDecodeMs": None,
                "linkedH264AdmissionAccepted": None,
                "linkedH264BootstrapRejectReason": None,
                "diagnosticTimelineChainState": None,
                "diagnosticTimelineChainReason": None,
                "chainRecovered": False,
                "chainRecoveredAtMs": None,
                "chainRecoveryReason": None,
                "chainFailureAfterSuccess": False,
                "chainFailureAtMs": None,
                "chainFailureReason": None,
                "effective": False,
            },
        )
        status = get_text(payload, "status")
        if status:
            episode["statuses"].append(status)
            episode["finalStatus"] = status
        lifecycle_phase = get_text(payload, "lifecyclePhase", "lifecycle_phase")
        if lifecycle_phase:
            episode["lifecyclePhase"] = lifecycle_phase
        episode["statusDetail"] = get_text(payload, "statusDetail", "status_detail")
        episode["responseVerdict"] = get_text(payload, "responseVerdict", "response_verdict")
        episode["timedOut"] = bool(payload_get(payload, "timedOut", "timed_out")) or episode["timedOut"]
        for field in (
            "requestedAtMs",
            "sentAtMs",
            "deadlineAtMs",
            "retiredAtMs",
            "firstVideoPacketAtMs",
            "firstKeyframePacketAtMs",
            "firstKeyframeDecodedAtMs",
            "requestToFirstPacketMs",
            "requestToFirstDecodeMs",
        ):
            value = get_number(payload, field)
            if value is not None:
                episode[field] = value
        if payload_get(payload, "linkedH264AdmissionAccepted") is not None:
            episode["linkedH264AdmissionAccepted"] = bool(payload_get(payload, "linkedH264AdmissionAccepted"))
        reject_reason = get_text(payload, "linkedH264BootstrapRejectReason")
        if reject_reason:
            episode["linkedH264BootstrapRejectReason"] = reject_reason
        timeline_chain_state = get_text(payload, "diagnosticTimelineChainState", "diagnostic_timeline_chain_state")
        if timeline_chain_state:
            episode["diagnosticTimelineChainState"] = timeline_chain_state
        timeline_chain_reason = get_text(payload, "diagnosticTimelineChainReason", "diagnostic_timeline_chain_reason")
        if timeline_chain_reason:
            episode["diagnosticTimelineChainReason"] = timeline_chain_reason

    def keyframe_episode_has_success_signal(episode: dict[str, Any]) -> bool:
        final_status = normalize_state(episode.get("finalStatus"))
        lifecycle_phase = normalize_state(episode.get("lifecyclePhase"))
        response_verdict = normalize_state(episode.get("responseVerdict"))
        timeline_chain_state = normalize_state(episode.get("diagnosticTimelineChainState"))
        return (
            final_status == "succeeded"
            or lifecycle_phase == "success"
            or response_verdict == "cleananchorcommitted"
            or timeline_chain_state in HEALTHY_CHAIN_STATES
        )

    # 修复建链成功率计算：按episode计数，避免重复计数同一链恢复事件
    chain_rows = chain_transition_rows(sorted_rows)
    chain_recovered_episodes = set()  # 使用set记录已恢复且已完成 decode 的 episode ID

    for episode in keyframe_episodes.values():
        success_ts = episode["firstKeyframeDecodedAtMs"] or episode["firstKeyframePacketAtMs"] or episode["firstVideoPacketAtMs"]
        success_signal = keyframe_episode_has_success_signal(episode)
        if success_ts is None and not success_signal:
            continue
        window_end = episode["retiredAtMs"] or episode["deadlineAtMs"]
        if window_end is None:
            base_ts = success_ts or episode["rowTsMs"] or 0
            window_end = base_ts + 5000

        # 在窗口内搜索链恢复事件，找到第一个就停止
        for row in chain_rows:
            payload = event_payload(row, "videoChainTransition")
            ts = row_ts(row)
            if payload is None or ts is None:
                continue
            if success_ts is not None and ts < success_ts:
                continue
            if ts > window_end:
                continue
            chain = payload.get("chain")
            chain_state = None
            chain_reason = None
            if isinstance(chain, dict):
                chain_state = normalize_state(chain.get("state"))
                chain_reason = get_text(chain, "reason")
            if chain_state in HEALTHY_CHAIN_STATES:
                episode["chainRecovered"] = True
                episode["chainRecoveredAtMs"] = ts
                episode["chainRecoveryReason"] = chain_reason
                episode["effective"] = episode["firstKeyframeDecodedAtMs"] is not None
                if episode["firstKeyframeDecodedAtMs"] is not None:
                    chain_recovered_episodes.add(episode["episodeId"])
                break  # 找到第一个就停止，避免重复计数
            if episode["firstKeyframeDecodedAtMs"] is not None and chain_state in {"broken", "recovering", "repairing", "stalled", "waiting-keyframe"}:
                episode["chainFailureAfterSuccess"] = True
                episode["chainFailureAtMs"] = ts
                episode["chainFailureReason"] = chain_reason or chain_state

        if not episode["chainRecovered"] and success_signal:
            episode["chainRecovered"] = True
            episode["chainRecoveredAtMs"] = episode["retiredAtMs"] or success_ts or episode["rowTsMs"]
            episode["chainRecoveryReason"] = (
                episode["responseVerdict"]
                or episode["diagnosticTimelineChainReason"]
                or episode["diagnosticTimelineChainState"]
            )
            if episode["firstKeyframeDecodedAtMs"] is not None:
                chain_recovered_episodes.add(episode["episodeId"])

        if success_signal:
            episode["effective"] = True

    keyframe_status_counts = Counter()
    keyframe_reason_counts = Counter()
    keyframe_request_kind_counts = Counter()
    keyframe_response_verdict_counts = Counter()
    keyframe_failure_reasons = Counter()
    keyframe_effective_failures = Counter()
    keyframe_samples: list[dict[str, Any]] = []
    keyframe_sent_count = 0
    keyframe_response_observed_count = 0
    keyframe_packet_seen_count = 0
    keyframe_decoded_count = 0
    keyframe_missed_count = 0
    keyframe_invalid_response_count = 0
    # 使用set的大小作为链恢复计数，避免重复计数
    keyframe_chain_recovered_count = len(chain_recovered_episodes)
    keyframe_chain_failed_after_success_count = 0
    for episode in sorted(keyframe_episodes.values(), key=lambda item: (item["requestedAtMs"] or 0, item["episodeId"])):
        final_status = episode["finalStatus"] or "unknown"
        success_signal = keyframe_episode_has_success_signal(episode)
        keyframe_status_counts[final_status] += 1
        keyframe_reason_counts[episode["requestReason"] or "unknown"] += 1
        keyframe_request_kind_counts[episode["requestKind"] or "unknown"] += 1
        if episode["responseVerdict"]:
            keyframe_response_verdict_counts[str(episode["responseVerdict"])] += 1
        if episode["sentAtMs"] is not None or "sent" in episode["statuses"]:
            keyframe_sent_count += 1
        if (
            episode["firstVideoPacketAtMs"] is not None
            or episode["firstKeyframePacketAtMs"] is not None
            or final_status in {"response-observed", "packet-seen", "decoded"}
        ):
            keyframe_response_observed_count += 1
        if episode["firstKeyframePacketAtMs"] is not None or final_status in {"packet-seen", "decoded"}:
            keyframe_packet_seen_count += 1
        if episode["firstKeyframeDecodedAtMs"] is not None or final_status in {"decoded", "succeeded"} or success_signal:
            keyframe_decoded_count += 1
        if final_status == "missed" or episode["timedOut"]:
            keyframe_missed_count += 1
        invalid_reason = episode["linkedH264BootstrapRejectReason"]
        if success_signal:
            pass
        elif invalid_reason:
            keyframe_invalid_response_count += 1
            keyframe_failure_reasons[invalid_reason] += 1
        elif episode["linkedH264AdmissionAccepted"] is False:
            keyframe_invalid_response_count += 1
            keyframe_failure_reasons["h264-admission-rejected"] += 1
        elif final_status == "missed" or episode["timedOut"]:
            keyframe_failure_reasons["missed-or-timeout"] += 1
        elif episode["responseVerdict"] and str(episode["responseVerdict"]).lower() not in {"decoded", "accepted", "packet-seen", "response-observed"}:
            keyframe_failure_reasons[str(episode["responseVerdict"])] += 1
        # 链恢复计数已在上面通过set统计，这里不再累加
        if not episode["chainRecovered"] and episode["firstKeyframeDecodedAtMs"] is not None:
            keyframe_effective_failures["decoded-without-healthy-chain"] += 1
        if episode["chainFailureAfterSuccess"]:
            keyframe_chain_failed_after_success_count += 1
            keyframe_effective_failures[episode["chainFailureReason"] or "chain-not-recovered"] += 1
        keyframe_samples.append(
            {
                "episodeId": episode["episodeId"],
                "requestReason": episode["requestReason"],
                "requestKind": episode["requestKind"],
                "finalStatus": final_status,
                "lifecyclePhase": episode["lifecyclePhase"],
                "responseVerdict": episode["responseVerdict"],
                "timedOut": episode["timedOut"],
                "firstKeyframeDecodedAtMs": episode["firstKeyframeDecodedAtMs"],
                "chainRecovered": episode["chainRecovered"],
                "chainRecoveredAtMs": episode["chainRecoveredAtMs"],
                "linkedH264AdmissionAccepted": episode["linkedH264AdmissionAccepted"],
                "linkedH264BootstrapRejectReason": episode["linkedH264BootstrapRejectReason"],
                "diagnosticTimelineChainState": episode["diagnosticTimelineChainState"],
                "effective": episode["effective"],
            }
        )

    nack_action_counts = Counter()
    nack_source_counts = Counter()
    nack_disposition_counts = Counter()
    nack_unrecoverable_reason_counts = Counter()
    nack_samples: list[dict[str, Any]] = []
    nack_sent_count = 0
    nack_recovered_count = 0
    nack_recovered_late_count = 0
    nack_skipped_count = 0
    nack_expired_count = 0
    nack_effective_count = 0
    nack_ineffective_count = 0
    for row in nack_rows(sorted_rows):
        payload = row.get("payload")
        if not isinstance(payload, dict):
            continue
        action = get_text(payload, "action") or str(row.get("event", "unknown"))
        source = get_text(payload, "source") or "unknown"
        disposition = get_text(payload, "nackDisposition", "nack_disposition")
        unrecoverable_reason = get_text(payload, "frameUnrecoverableReason", "frame_unrecoverable_reason")
        nack_action_counts[action] += 1
        nack_source_counts[source] += 1
        if disposition:
            nack_disposition_counts[disposition] += 1
        if unrecoverable_reason:
            nack_unrecoverable_reason_counts[unrecoverable_reason] += 1
        if str(row.get("event", "")) == "nackSent":
            nack_sent_count += 1
        elif str(row.get("event", "")) == "nackRecovered":
            nack_recovered_count += 1
            if action.lower() == "recovered":
                nack_effective_count += 1
            else:
                nack_recovered_late_count += 1
                nack_ineffective_count += 1
        elif str(row.get("event", "")) == "nackSkipped":
            nack_skipped_count += 1
            nack_ineffective_count += 1
        elif str(row.get("event", "")) == "nackExpired":
            nack_expired_count += 1
            nack_ineffective_count += 1
        nack_samples.append(
            {
                "seq": row_seq(row),
                "tsMs": row_ts(row),
                "event": row.get("event"),
                "action": action,
                "source": source,
                "retryCount": get_int(payload, "retryCount", "retry_count"),
                "frameIsKeyframe": payload_get(payload, "frameIsKeyframe", "frame_is_keyframe"),
                "frameImportance": get_text(payload, "frameImportance", "frame_importance"),
                "nackDisposition": disposition,
                "frameUnrecoverableReason": unrecoverable_reason,
            }
        )

    repairability_samples: list[dict[str, Any]] = []
    repairability_missing_streak = 0
    repairability_max_missing_streak = 0
    repairability_last_ts: int | None = None
    repairability_longest_missing_gap_ms = 0
    for row in sorted_rows:
        ts = row_ts(row)
        if ts is None:
            continue
        payload = row.get("payload")
        score = extract_repairability_value(payload)
        if score is None:
            repairability_missing_streak += 1
            repairability_max_missing_streak = max(
                repairability_max_missing_streak,
                repairability_missing_streak,
            )
            continue
        score_value = float(score)
        if repairability_last_ts is not None:
            repairability_longest_missing_gap_ms = max(
                repairability_longest_missing_gap_ms,
                ts - repairability_last_ts,
            )
        repairability_last_ts = ts
        repairability_missing_streak = 0
        repairability_samples.append(
            {
                "seq": row_seq(row),
                "tsMs": ts,
                "event": row.get("event"),
                "domain": row.get("domain"),
                "score": score_value,
            }
        )

    repairability_score_values = [sample["score"] for sample in repairability_samples]
    repairability_stats: dict[str, Any] = {
        "sampleCount": len(repairability_samples),
        "samples": repairability_samples,
        "persistence": {
            "presentRowRatio": round_score(
                safe_ratio(len(repairability_samples), len(sorted_rows)) or 0.0
            ),
            "longestMissingGapMs": repairability_longest_missing_gap_ms,
            "maxMissingStreakRows": repairability_max_missing_streak,
        },
    }
    if repairability_score_values:
        repairability_stats.update(
            {
                "first": repairability_score_values[0],
                "last": repairability_score_values[-1],
                "min": min(repairability_score_values),
                "max": max(repairability_score_values),
                "avg": round_score(sum(repairability_score_values) / len(repairability_score_values)),
            }
        )

    keyframe_chain_build_success_rate = safe_ratio(
        keyframe_chain_recovered_count,
        keyframe_decoded_count,
    )
    nack_effective_denominator = nack_sent_count
    if nack_effective_denominator <= 0:
        nack_effective_denominator = (
            nack_recovered_count + nack_recovered_late_count + nack_skipped_count + nack_expired_count
        )
    nack_effectiveness_rate = safe_ratio(nack_effective_count, nack_effective_denominator)
    keyframe_effective_rate = safe_ratio(
        sum(1 for episode in keyframe_episodes.values() if episode["effective"]),
        len(keyframe_episodes),
    )
    repairability_persistence_rate = repairability_stats["persistence"]["presentRowRatio"]
    recovery_score_components = {
        "keyframeEffectiveRate": keyframe_effective_rate or 0.0,
        "chainBuildSuccessRate": keyframe_chain_build_success_rate or 0.0,
        "nackEffectiveRate": nack_effectiveness_rate or 0.0,
        "repairabilityPersistenceRate": repairability_persistence_rate,
    }
    recovery_effectiveness_score = round_score(
        recovery_score_components["keyframeEffectiveRate"] * 0.3
        + recovery_score_components["chainBuildSuccessRate"] * 0.35
        + recovery_score_components["nackEffectiveRate"] * 0.25
        + recovery_score_components["repairabilityPersistenceRate"] * 0.1
    )
    twcc_observation_total = len(twcc_mapping_missing_rows) + len(twcc_inbound_seen_rows)
    twcc_mapping_missing_rate = safe_ratio(len(twcc_mapping_missing_rows), twcc_observation_total)
    keyframe_deferred_count = keyframe_status_counts["deferred"]
    keyframe_sent_rate = safe_ratio(keyframe_sent_count, len(keyframe_episodes))
    ingress_unknown_upstream_count = ingress_upstream_cause_counts["unknown"]
    ingress_unknown_upstream_rate = safe_ratio(ingress_unknown_upstream_count, len(ingress_samples))

    return {
        "structuredRecoveryTrace": {
            "pictureRecoveryTransition": {
                "count": len(transition_samples),
                "phaseCounts": dict(transition_phase_counts),
                "toPhaseCounts": dict(transition_to_phase_counts),
                "causeCounts": dict(transition_cause_counts),
                "events": transition_samples,
            },
            "pictureRecoveryBlockerObserved": {
                "count": len(blocker_samples),
                "gateCounts": dict(blocker_gate_counts),
                "blockerKindCounts": dict(blocker_kind_counts),
                "severityCounts": dict(blocker_severity_counts),
                "events": blocker_samples,
            },
            "videoIngressTermination": {
                "count": len(ingress_samples),
                "kindCounts": dict(ingress_kind_counts),
                "causeCounts": dict(ingress_cause_counts),
                "upstreamCauseCounts": dict(ingress_upstream_cause_counts),
                "events": ingress_samples,
            },
            "firstFrameLatencyObserved": {
                "count": len(first_frame_samples),
                "terminalPhaseCounts": dict(first_frame_terminal_phase_counts),
                "incompleteReasonCounts": dict(first_frame_incomplete_reason_counts),
                "events": first_frame_samples,
            },
            "h264InspectionObserved": {
                "count": len(h264_samples),
                "rejectClassificationCounts": dict(h264_reject_classification_counts),
                "postRecoveryDegradationCount": h264_post_recovery_degradation_count,
                "events": h264_samples,
            },
        },
        "recoveryLedgerRows": len(ledger_rows),
        "connectingWindows": len(connecting_windows),
        "silenceThresholdMs": silence_threshold_ms,
        "silenceBreachCount": len(silence_breaches),
        "silenceBreaches": silence_breaches,
        "failedTerminalCount": len(failed_terminal_entries),
        "failedTerminalReasons": dict(failed_terminal_reasons),
        "failedTerminalEntries": failed_terminal_entries,
        "unlockEvidence": unlock_evidence,
        "successfulActionSamples": len(successful_samples),
        "successfulActionIncrements": successful_action_increments,
        "controlPlaneHealth": {
            "keyframeEpisodeCount": len(keyframe_episodes),
            "keyframeSentCount": keyframe_sent_count,
            "keyframeDeferredCount": keyframe_deferred_count,
            "keyframeSentRate": round_score(keyframe_sent_rate or 0.0),
            "twccReceiverMappingMissingCount": len(twcc_mapping_missing_rows),
            "twccInboundExtensionSeenCount": len(twcc_inbound_seen_rows),
            "twccReceiverMappingMissingRate": round_score(twcc_mapping_missing_rate or 0.0),
            "feedbackTargetAvailabilityChangedCount": len(feedback_target_availability_rows),
            "feedbackTargetStateCounts": dict(feedback_target_state_counts),
            "feedbackTargetReasonCounts": dict(feedback_target_reason_counts),
        },
        "ingressHealth": {
            "terminationCount": len(ingress_samples),
            "rxClosedCount": ingress_kind_counts["rxClosed"],
            "upstreamSenderDroppedCount": ingress_cause_counts["upstreamSenderDropped"],
            "unknownUpstreamCauseCount": ingress_unknown_upstream_count,
            "unknownUpstreamCauseRate": round_score(ingress_unknown_upstream_rate or 0.0),
            "causeCounts": dict(ingress_cause_counts),
            "upstreamCauseCounts": dict(ingress_upstream_cause_counts),
        },
        "firstFrameHealth": {
            "observationCount": len(first_frame_samples),
            "controlReadyToPliSentMsCount": len(first_frame_control_ready_to_pli),
            "pliSentToFirstIdrPacketMsCount": len(first_frame_pli_to_idr),
            "firstIdrPacketToFirstDecodeMsCount": len(first_frame_idr_to_decode),
            "firstDecodeToCleanAnchorCommittedMsCount": len(first_frame_decode_to_clean_anchor),
            "cleanAnchorCommittedToDisplayStableMsCount": len(first_frame_clean_anchor_to_display),
            "avgControlReadyToPliSentMs": average_number(first_frame_control_ready_to_pli),
            "avgPliSentToFirstIdrPacketMs": average_number(first_frame_pli_to_idr),
            "avgFirstIdrPacketToFirstDecodeMs": average_number(first_frame_idr_to_decode),
            "avgFirstDecodeToCleanAnchorCommittedMs": average_number(first_frame_decode_to_clean_anchor),
            "avgCleanAnchorCommittedToDisplayStableMs": average_number(first_frame_clean_anchor_to_display),
            "terminalPhaseCounts": dict(first_frame_terminal_phase_counts),
            "incompleteReasonCounts": dict(first_frame_incomplete_reason_counts),
        },
        "bootstrapHealth": {
            "observationCount": len(h264_samples),
            "acceptedCount": h264_accepted_count,
            "bootstrapMissingIdrCount": h264_bootstrap_reject_reason_counts["bootstrapMissingIdr"],
            "bootstrapRejectReasonCounts": dict(h264_bootstrap_reject_reason_counts),
            "rejectClassificationCounts": dict(h264_reject_classification_counts),
            "continuationProfileCounts": dict(h264_continuation_profile_counts),
            "postRecoveryDegradationCount": h264_post_recovery_degradation_count,
        },
        "presentationHealth": {
            "hostMailboxStateCount": len(host_mailbox_state_rows),
            "displayedFrameStaleCount": displayed_frame_stale_count,
            "retainedOldFrameRiskCount": retained_old_frame_risk_count,
            "cadencePhaseCounts": dict(host_present_cadence_phase_counts),
            "noPendingPressureLevelCounts": dict(
                host_present_no_pending_pressure_level_counts
            ),
            "frameDropEventCount": len(frame_drop_samples),
            "frameDropReasonCounts": dict(frame_drop_reason_counts),
            "frameDropStageCounts": dict(frame_drop_stage_counts),
            "frameDropDetailCounts": dict(frame_drop_detail_counts),
            "scheduledFrameStaleCount": scheduled_frame_stale_count,
            "submittedFrameStaleCount": submitted_frame_stale_count,
            "recoveryValuedFrameDropCount": recovery_valued_frame_drop_count,
            "hostPresentSamples": host_present_samples,
            "frameDropSamples": frame_drop_samples,
        },
        "keyframeEffectiveness": {
            "episodeCount": len(keyframe_episodes),
            "statusCounts": dict(keyframe_status_counts),
            "requestReasonCounts": dict(keyframe_reason_counts),
            "requestKindCounts": dict(keyframe_request_kind_counts),
            "responseVerdictCounts": dict(keyframe_response_verdict_counts),
            "sentCount": keyframe_sent_count,
            "responseObservedCount": keyframe_response_observed_count,
            "packetSeenCount": keyframe_packet_seen_count,
            "decodedCount": keyframe_decoded_count,
            "missedCount": keyframe_missed_count,
            "invalidResponseCount": keyframe_invalid_response_count,
            "chainRecoveredCount": keyframe_chain_recovered_count,
            "chainBuildSuccessRate": round_score(keyframe_chain_build_success_rate or 0.0),
            "chainFailedAfterSuccessCount": keyframe_chain_failed_after_success_count,
            "effectiveCount": sum(1 for episode in keyframe_episodes.values() if episode["effective"]),
            "effectiveRate": round_score(keyframe_effective_rate or 0.0),
            "suppressionCounts": dict(keyframe_suppression_counts),
            "failureReasons": dict(keyframe_failure_reasons),
            "effectiveFailureReasons": dict(keyframe_effective_failures),
            "episodes": keyframe_samples,
            "suppressionSamples": keyframe_suppression_samples,
        },
        "nackEffectiveness": {
            "eventCount": len(nack_samples),
            "actionCounts": dict(nack_action_counts),
            "sourceCounts": dict(nack_source_counts),
            "dispositionCounts": dict(nack_disposition_counts),
            "unrecoverableReasonCounts": dict(nack_unrecoverable_reason_counts),
            "sentCount": nack_sent_count,
            "recoveredCount": nack_recovered_count,
            "recoveredLateCount": nack_recovered_late_count,
            "skippedCount": nack_skipped_count,
            "expiredCount": nack_expired_count,
            "effectiveCount": nack_effective_count,
            "ineffectiveCount": nack_ineffective_count,
            "effectiveRate": round_score(nack_effectiveness_rate or 0.0),
            "events": nack_samples,
        },
        "repairabilityPersistence": repairability_stats,
        "recoveryEffectiveness": {
            "score": recovery_effectiveness_score,
            "components": {
                key: round_score(value) for key, value in recovery_score_components.items()
            },
            "weights": {
                "keyframeEffectiveRate": 0.3,
                "chainBuildSuccessRate": 0.35,
                "nackEffectiveRate": 0.25,
                "repairabilityPersistenceRate": 0.1,
            },
        },
    }


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
    recovery_silence_threshold_ms: int,
    metric_name: str | None = None,
) -> TraceProfile:
    trace_mode_counts = Counter(str(row.get("traceMode", "legacy")) for row in rows)
    trace_profile_counts = Counter(
        str(row.get("traceProfile", row.get("traceMode", "legacy"))) for row in rows
    )
    dimension_counts = Counter(str(row.get("dimension", "legacy")) for row in rows)
    importance_counts = Counter(str(row.get("importance", "legacy")) for row in rows)
    trace_budget_notice_count = sum(1 for row in rows if row.get("event") == "traceBudgetNotice")
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
    recovery_audit = analyze_recovery_audit(rows, recovery_silence_threshold_ms)

    return TraceProfile(
        path=path,
        rows=rows,
        first_ts=first_ts,
        last_ts=last_ts,
        duration_ms=duration_ms,
        trace_mode_counts=trace_mode_counts,
        trace_profile_counts=trace_profile_counts,
        dimension_counts=dimension_counts,
        importance_counts=importance_counts,
        trace_budget_notice_count=trace_budget_notice_count,
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
        recovery_audit=recovery_audit,
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


def print_recovery_audit(profile: TraceProfile, sample_limit: int) -> None:
    audit = profile.recovery_audit
    print("\nrecovery_audit:")
    structured = audit["structuredRecoveryTrace"]
    transitions = structured["pictureRecoveryTransition"]
    transition_to_phase_text = ", ".join(
        f"{name}={count}" for name, count in sorted(transitions["toPhaseCounts"].items())
    ) or "none"
    transition_cause_text = ", ".join(
        f"{name}={count}" for name, count in sorted(transitions["causeCounts"].items())
    ) or "none"
    print("  - picture_recovery_transition:")
    print(
        "    - "
        f"events={transitions['count']} to_phases={transition_to_phase_text} "
        f"causes={transition_cause_text}"
    )
    for item in transitions["events"][:sample_limit]:
        print(
            "    - "
            f"seq={item['seq']} tsMs={item['tsMs']} episode={item['episodeId']} "
            f"epoch={item['recoveryEpoch']} phase={item['fromPhase']}->{item['toPhase']} "
            f"cause={item['cause']} detail={item['detail']}"
        )
    blockers = structured["pictureRecoveryBlockerObserved"]
    blocker_gate_text = ", ".join(
        f"{name}={count}" for name, count in sorted(blockers["gateCounts"].items())
    ) or "none"
    blocker_kind_text = ", ".join(
        f"{name}={count}" for name, count in sorted(blockers["blockerKindCounts"].items())
    ) or "none"
    print("  - picture_recovery_blocker_observed:")
    print(
        "    - "
        f"events={blockers['count']} gates={blocker_gate_text} blocker_kinds={blocker_kind_text}"
    )
    for item in blockers["events"][:sample_limit]:
        print(
            "    - "
            f"seq={item['seq']} tsMs={item['tsMs']} episode={item['episodeId']} "
            f"epoch={item['recoveryEpoch']} gate={item['gate']} blocker={item['blockerKind']} "
            f"severity={item['severity']} first_seen={item['firstObservedAtMs']} count={item['count']}"
        )
    ingress = structured["videoIngressTermination"]
    ingress_cause_text = ", ".join(
        f"{name}={count}" for name, count in sorted(ingress["causeCounts"].items())
    ) or "none"
    ingress_upstream_text = ", ".join(
        f"{name}={count}" for name, count in sorted(ingress["upstreamCauseCounts"].items())
    ) or "none"
    print("  - video_ingress_termination:")
    print(
        "    - "
        f"events={ingress['count']} causes={ingress_cause_text} upstream_causes={ingress_upstream_text}"
    )
    for item in ingress["events"][:sample_limit]:
        print(
            "    - "
            f"seq={item['seq']} tsMs={item['tsMs']} termination={item['terminationId']} "
            f"derived_from={item['derivedFromTerminationId']} kind={item['kind']} "
            f"cause={item['cause']} upstream={item['upstreamCause']} "
            f"epoch={item['linkedRecoveryEpoch']} episode={item['linkedEpisodeId']}"
        )
    first_frame = structured["firstFrameLatencyObserved"]
    first_frame_terminal_text = ", ".join(
        f"{name}={count}" for name, count in sorted(first_frame["terminalPhaseCounts"].items())
    ) or "none"
    first_frame_incomplete_text = ", ".join(
        f"{name}={count}" for name, count in sorted(first_frame["incompleteReasonCounts"].items())
    ) or "none"
    print("  - first_frame_latency_observed:")
    print(
        "    - "
        f"events={first_frame['count']} terminal_phases={first_frame_terminal_text} "
        f"incomplete_reasons={first_frame_incomplete_text}"
    )
    for item in first_frame["events"][:sample_limit]:
        print(
            "    - "
            f"seq={item['seq']} tsMs={item['tsMs']} episode={item['episodeId']} "
            f"epoch={item['recoveryEpoch']} control_to_pli={item['controlReadyToPliSentMs']} "
            f"pli_to_idr={item['pliSentToFirstIdrPacketMs']} "
            f"idr_to_decode={item['firstIdrPacketToFirstDecodeMs']} "
            f"decode_to_clean_anchor={item['firstDecodeToCleanAnchorCommittedMs']} "
            f"clean_anchor_to_display={item['cleanAnchorCommittedToDisplayStableMs']} "
            f"terminal={item['terminalPhase']} incomplete={item['incompleteReason']}"
        )
    h264 = structured["h264InspectionObserved"]
    h264_reject_text = ", ".join(
        f"{name}={count}" for name, count in sorted(h264["rejectClassificationCounts"].items())
    ) or "none"
    print("  - h264_inspection_observed:")
    print(
        "    - "
        f"events={h264['count']} reject_classification={h264_reject_text} "
        f"post_recovery_degradation={h264['postRecoveryDegradationCount']}"
    )
    for item in h264["events"][:sample_limit]:
        print(
            "    - "
            f"seq={item['seq']} tsMs={item['tsMs']} event={item['event']} "
            f"episode={item['boundEpisodeId']} epoch={item['boundRecoveryEpoch']} "
            f"phase={item['episodePhaseAtObservation']} admission={item['admissionAccepted']} "
            f"is_idr={item['isIdr']} reject={item['rejectClassification']} "
            f"bootstrap={item['bootstrapRejectReason']} post_recovery_degradation={item['isPostRecoveryDegradation']}"
        )
    print(
        "  - "
        f"ledger_rows={audit['recoveryLedgerRows']} "
        f"connecting_windows={audit['connectingWindows']} "
        f"silence_threshold_ms={audit['silenceThresholdMs']} "
        f"silence_breaches={audit['silenceBreachCount']}"
    )
    for breach in audit["silenceBreaches"][:sample_limit]:
        print(
            "    - "
            f"window={breach['windowStartTsMs']}->{breach['windowEndTsMs']} "
            f"duration={fmt_ms(breach['windowDurationMs'])} "
            f"max_ledger_silence={fmt_ms(breach['maxLedgerSilenceMs'])} "
            f"ledger_entries={breach['ledgerEntries']}"
        )
    reasons = audit["failedTerminalReasons"]
    reason_text = ", ".join(f"{name}={count}" for name, count in sorted(reasons.items())) or "none"
    print(
        "  - "
        f"failed_terminal_count={audit['failedTerminalCount']} reasons={reason_text}"
    )
    for item in audit["failedTerminalEntries"][:sample_limit]:
        print(
            "    - "
            f"seq={item['seq']} tsMs={item['tsMs']} reason={item['reason']} "
            f"decisionId={item['decisionId']} state={item['stateBefore']}->{item['stateAfter']} "
            f"gate={item['gateResult']} action={item['actionSelected']}"
        )
    unlock_items = audit["unlockEvidence"]
    unlocked_count = sum(1 for item in unlock_items if item["unlocked"])
    print(f"  - failed_terminal_unlock={unlocked_count}/{len(unlock_items)}")
    for item in unlock_items[:sample_limit]:
        status = "unlocked" if item["unlocked"] else "still-locked"
        print(
            "    - "
            f"failedAt={item['failedTerminalTsMs']} status={status} "
            f"unlockAt={item['unlockTsMs']} kind={item['unlockKind']} detail={item['detail']}"
        )
    print(
        "  - "
        f"successful_action_samples={audit['successfulActionSamples']} "
        f"increments={audit['successfulActionIncrements']}"
    )
    presentation = audit["presentationHealth"]
    cadence_text = ", ".join(
        f"{name}={count}" for name, count in sorted(presentation["cadencePhaseCounts"].items())
    ) or "none"
    pressure_text = ", ".join(
        f"{name}={count}"
        for name, count in sorted(presentation["noPendingPressureLevelCounts"].items())
    ) or "none"
    drop_reason_text = ", ".join(
        f"{name}={count}" for name, count in sorted(presentation["frameDropReasonCounts"].items())
    ) or "none"
    drop_detail_text = ", ".join(
        f"{name}={count}" for name, count in sorted(presentation["frameDropDetailCounts"].items())
    ) or "none"
    print("  - presentation_health:")
    print(
        "    - "
        f"host_mailbox_states={presentation['hostMailboxStateCount']} "
        f"displayed_frame_stale={presentation['displayedFrameStaleCount']} "
        f"retained_old_frame_risk={presentation['retainedOldFrameRiskCount']} "
        f"frame_drop_events={presentation['frameDropEventCount']} "
        f"recovery_valued_frame_drops={presentation['recoveryValuedFrameDropCount']}"
    )
    print(f"    - cadence={cadence_text}")
    print(f"    - no_pending_pressure={pressure_text}")
    print(f"    - frame_drop_reasons={drop_reason_text}")
    print(f"    - frame_drop_details={drop_detail_text}")
    for item in presentation["hostPresentSamples"][:sample_limit]:
        print(
            "    - "
            f"host seq={item['seq']} tsMs={item['tsMs']} cadence={item['cadencePhase']} "
            f"pressure={item['noPendingPressureLevel']} displayed_stale={item['displayedFrameStale']} "
            f"retained_old_frame_risk={item['retainedOldFrameRisk']} present_age_ms={item['presentAgeMs']} "
            f"last_displayed_seq={item['lastDisplayedFrameSeq']}"
        )
    for item in presentation["frameDropSamples"][:sample_limit]:
        print(
            "    - "
            f"drop seq={item['seq']} tsMs={item['tsMs']} event={item['event']} "
            f"reason={item['reason']} stage={item['stage']} detail={item['detail']} "
            f"recovery_disposition={item['frameRecoveryDisposition']} "
            f"frame_seq={item['frameSeq']} rtp={item['frameRtpTimestamp']} keyframe={item['isKeyframe']}"
        )
    keyframe = audit["keyframeEffectiveness"]
    suppression_text = ", ".join(
        f"{name}={count}" for name, count in sorted(keyframe["suppressionCounts"].items())
    ) or "none"
    failure_text = ", ".join(
        f"{name}={count}" for name, count in sorted(keyframe["failureReasons"].items())
    ) or "none"
    effective_failure_text = ", ".join(
        f"{name}={count}" for name, count in sorted(keyframe["effectiveFailureReasons"].items())
    ) or "none"
    print("  - keyframe_effectiveness:")
    print(
        "    - "
        f"episodes={keyframe['episodeCount']} sent={keyframe['sentCount']} "
        f"response_observed={keyframe['responseObservedCount']} packet_seen={keyframe['packetSeenCount']} "
        f"decoded={keyframe['decodedCount']} missed={keyframe['missedCount']} "
        f"invalid_response={keyframe['invalidResponseCount']} effective={keyframe['effectiveCount']} "
        f"effective_rate={keyframe['effectiveRate']}"
    )
    print(
        "    - "
        f"chain_recovered={keyframe['chainRecoveredCount']} "
        f"chain_build_success_rate={keyframe['chainBuildSuccessRate']} "
        f"chain_failed_after_success={keyframe['chainFailedAfterSuccessCount']} "
        f"suppression={suppression_text}"
    )
    print(f"    - failure_reasons={failure_text}")
    print(f"    - effective_failure_reasons={effective_failure_text}")
    for item in keyframe["episodes"][:sample_limit]:
        print(
            "    - "
            f"episode={item['episodeId']} reason={item['requestReason']} kind={item['requestKind']} "
            f"status={item['finalStatus']} verdict={item['responseVerdict']} timed_out={item['timedOut']} "
            f"decoded_at={item['firstKeyframeDecodedAtMs']} chain_recovered={item['chainRecovered']} "
            f"effective={item['effective']} h264_ok={item['linkedH264AdmissionAccepted']} "
            f"bootstrap_reject={item['linkedH264BootstrapRejectReason']}"
        )
    nack = audit["nackEffectiveness"]
    action_text = ", ".join(f"{name}={count}" for name, count in sorted(nack["actionCounts"].items())) or "none"
    disposition_text = ", ".join(
        f"{name}={count}" for name, count in sorted(nack["dispositionCounts"].items())
    ) or "none"
    unrecoverable_text = ", ".join(
        f"{name}={count}" for name, count in sorted(nack["unrecoverableReasonCounts"].items())
    ) or "none"
    print("  - nack_effectiveness:")
    print(
        "    - "
        f"events={nack['eventCount']} sent={nack['sentCount']} recovered={nack['recoveredCount']} "
        f"recovered_late={nack['recoveredLateCount']} skipped={nack['skippedCount']} "
        f"expired={nack['expiredCount']} effective={nack['effectiveCount']} "
        f"ineffective={nack['ineffectiveCount']} effective_rate={nack['effectiveRate']}"
    )
    print(f"    - actions={action_text}")
    print(f"    - disposition={disposition_text}")
    print(f"    - unrecoverable_reasons={unrecoverable_text}")
    for item in nack["events"][:sample_limit]:
        print(
            "    - "
            f"seq={item['seq']} tsMs={item['tsMs']} event={item['event']} action={item['action']} "
            f"source={item['source']} retry={item['retryCount']} "
            f"frame_is_keyframe={item['frameIsKeyframe']} importance={item['frameImportance']} "
            f"disposition={item['nackDisposition']} unrecoverable={item['frameUnrecoverableReason']}"
        )
    repairability = audit["repairabilityPersistence"]
    persistence = repairability["persistence"]
    print("  - repairability_persistence:")
    print(
        "    - "
        f"samples={repairability['sampleCount']} present_row_ratio={persistence['presentRowRatio']} "
        f"longest_missing_gap_ms={persistence['longestMissingGapMs']} "
        f"max_missing_streak_rows={persistence['maxMissingStreakRows']}"
    )
    if repairability["sampleCount"] > 0:
        print(
            "    - "
            f"first={repairability['first']} last={repairability['last']} "
            f"min={repairability['min']} max={repairability['max']} avg={repairability['avg']}"
        )
    for item in repairability["samples"][:sample_limit]:
        print(
            "    - "
            f"seq={item['seq']} tsMs={item['tsMs']} domain={item['domain']} "
            f"event={item['event']} score={item['score']}"
        )
    recovery_effectiveness = audit["recoveryEffectiveness"]
    component_text = ", ".join(
        f"{name}={value}" for name, value in sorted(recovery_effectiveness["components"].items())
    )
    print("  - recovery_effectiveness:")
    print(
        "    - "
        f"score={recovery_effectiveness['score']} components={component_text}"
    )


def build_machine_summary(
    profile: TraceProfile,
    args: argparse.Namespace,
    compare_profile: TraceProfile | None = None,
) -> dict[str, Any]:
    result: dict[str, Any] = {
        "file": str(profile.path),
        "rows": len(profile.rows),
        "timeRangeMs": {
            "first": profile.first_ts,
            "last": profile.last_ts,
            "duration": profile.duration_ms,
        },
        "filters": {
            "sessionId": args.session_id,
            "domain": args.domain,
            "phase": args.phase,
            "metric": args.metric,
            "timeWindow": args.time_window,
        },
        "counts": {
            "traceModes": dict(profile.trace_mode_counts),
            "traceProfiles": dict(profile.trace_profile_counts),
            "dimensions": dict(profile.dimension_counts),
            "importance": dict(profile.importance_counts),
            "traceBudgetNotices": profile.trace_budget_notice_count,
            "categories": dict(profile.category_counts),
            "domains": dict(profile.domain_counts),
            "eventsTop": dict(profile.event_counts.most_common(args.top_events)),
            "phaseSegments": len(profile.phase_segments),
            "longGaps": len(profile.long_gaps),
            "anomalyWindows": len(profile.anomaly_windows),
            "suspiciousRows": len(profile.suspicious_rows),
        },
        "recoveryAudit": profile.recovery_audit,
    }
    if compare_profile is not None:
        result["compare"] = {
            "file": str(compare_profile.path),
            "rows": len(compare_profile.rows),
            "durationMs": compare_profile.duration_ms,
            "phaseWindows": sum(compare_profile.phase_signature_counts.values()),
            "anomalySignals": sum(compare_profile.anomaly_signal_counts.values()),
        }
    return result


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
        "trace_profiles: "
        + ", ".join(f"{name}={count}" for name, count in profile.trace_profile_counts.most_common())
    )
    print(
        "trace_modes: "
        + ", ".join(f"{name}={count}" for name, count in profile.trace_mode_counts.most_common())
    )
    print(
        "dimensions: "
        + ", ".join(f"{name}={count}" for name, count in profile.dimension_counts.most_common())
    )
    print(
        "importance: "
        + ", ".join(f"{name}={count}" for name, count in profile.importance_counts.most_common())
    )
    if profile.trace_budget_notice_count:
        print(f"trace_budget_notices: {profile.trace_budget_notice_count}")
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
    print_recovery_audit(profile, args.sample_rows)

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


def resolve_category_filter(
    categories: str | None, exclude_categories: str | None
) -> frozenset[str] | None:
    """Return allowed categories, or None to allow all."""
    if categories and exclude_categories:
        return None  # caller prints error
    if categories:
        return frozenset(x.strip() for x in categories.split(",") if x.strip())
    if exclude_categories:
        all_known = frozenset({"event", "decision", "state", "snapshot", "log"})
        ex = {x.strip() for x in exclude_categories.split(",") if x.strip()}
        return frozenset(all_known - ex)
    return None


def load_trace_profile(
    path: Path,
    *,
    session_id: str | None,
    domain: str | None,
    time_filters: list[tuple[int | None, int | None]],
    phase_filter: str | None,
    gap_threshold_ms: int,
    cluster_window_ms: int,
    recovery_silence_threshold_ms: int,
    metric_name: str | None = None,
    category_allowlist: frozenset[str] | None = None,
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
            if category_allowlist is not None:
                cat = str(row.get("category", ""))
                if cat not in category_allowlist:
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
        recovery_silence_threshold_ms=recovery_silence_threshold_ms,
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
    parser.add_argument(
        "--recovery-silence-threshold-ms",
        type=int,
        default=3000,
        help="max allowed recoveryDecisionLedger silence in connecting windows",
    )
    parser.add_argument(
        "--json",
        action="store_true",
        help="emit machine-readable JSON summary",
    )
    parser.add_argument(
        "--categories",
        help="comma-separated category whitelist (event,decision,state,snapshot,log)",
    )
    parser.add_argument(
        "--exclude-categories",
        help="comma-separated categories to omit from analysis (e.g. log)",
    )
    parser.add_argument(
        "--anchor-seq",
        type=int,
        default=None,
        help="if set, print JSON rows around this seq (drill-down); skips normal summary",
    )
    parser.add_argument("--context-before", type=int, default=25, help="rows before anchor-seq")
    parser.add_argument("--context-after", type=int, default=80, help="rows after anchor-seq")
    return parser.parse_args()


def dump_anchor_context(path: Path, anchor_seq: int, before: int, after: int) -> int:
    rows: list[dict[str, Any]] = []
    with path.open("r", encoding="utf-8") as handle:
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
    indices = [i for i, r in enumerate(rows) if r.get("seq") == anchor_seq]
    if not indices:
        print(f"seq {anchor_seq} not found in {path}", file=sys.stderr)
        return 3
    idx = indices[0]
    lo = max(0, idx - before)
    hi = min(len(rows), idx + after + 1)
    print(
        f"# anchor seq={anchor_seq} line_index={idx} window=[{lo},{hi}) file={path}",
        file=sys.stderr,
    )
    for r in rows[lo:hi]:
        print(json.dumps(r, ensure_ascii=False))
    return 0


def main() -> int:
    args = parse_args()

    path = Path(args.trace)
    if not path.is_file():
        print(f"trace file not found: {path}", file=sys.stderr)
        return 2

    if args.anchor_seq is not None:
        return dump_anchor_context(path, args.anchor_seq, args.context_before, args.context_after)

    time_filters = parse_time_filters(args.time_window)
    if args.time_window and time_filters is None:
        print("invalid time window filter", file=sys.stderr)
        return 2

    if args.metric and not is_metric_field(args.metric):
        print(f"unsupported metric field: {args.metric}", file=sys.stderr)
        return 2

    if args.categories and args.exclude_categories:
        print(
            "error: use only one of --categories and --exclude-categories",
            file=sys.stderr,
        )
        return 2

    category_allowlist = resolve_category_filter(args.categories, args.exclude_categories)

    profile = load_trace_profile(
        path,
        session_id=args.session_id,
        domain=args.domain,
        time_filters=time_filters,
        phase_filter=args.phase,
        gap_threshold_ms=args.gap_threshold_ms,
        cluster_window_ms=args.cluster_window_ms,
        recovery_silence_threshold_ms=args.recovery_silence_threshold_ms,
        metric_name=args.metric,
        category_allowlist=category_allowlist,
    )
    compare_profile: TraceProfile | None = None

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
            recovery_silence_threshold_ms=args.recovery_silence_threshold_ms,
            metric_name=args.metric,
            category_allowlist=category_allowlist,
        )

    if args.json:
        print(
            json.dumps(
                build_machine_summary(profile, args, compare_profile),
                ensure_ascii=False,
                indent=2,
            )
        )
        return 0

    print_profile(profile, args)
    if compare_profile is not None:
        print_trace_comparison(profile, compare_profile)

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
