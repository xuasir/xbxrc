#!/usr/bin/env python3
"""Validate the iOS streaming core path in runtime trace JSONL files."""

from __future__ import annotations

import argparse
import importlib.util
import json
import sys
from collections import defaultdict
from pathlib import Path
from types import ModuleType
from typing import Any


REPO_ROOT = Path(__file__).resolve().parents[2]
ANALYZER_PATH = (
    REPO_ROOT
    / ".agents"
    / "skills"
    / "analyze-ios-runtime-trace"
    / "scripts"
    / "analyze_ios_runtime_trace.py"
)
TRACE_REORDER_WINDOW_ROWS = 2
EXPECTED_CHANNEL_PROFILES = {
    "input": {"protocol": "1.0", "ordered": True},
    "control": {"protocol": "controlV1", "ordered": True},
    "chat": {"protocol": "chatV1", "ordered": True},
    "message": {"protocol": "messageV1", "ordered": True},
}
CORE_CLEANUP_EVENTS = (
    "iceTasksCancelled",
    "peerClosed",
    "remoteSessionClosed",
    "accessReleased",
)
PEER_CONTEXT_EVENTS = {
    "answerApplied",
    "localIceStarted",
    "localIceCompleted",
    "remoteIceBatchReceived",
    "remoteIceBatchApplied",
    "remoteIceCompleted",
    "peerConnected",
    "dataChannelProfilesCreated",
    "messageHandshakeSent",
    "messageHandshakeAcked",
    "messagePostHandshakeCompleted",
    "controlBootstrapPreHandshakeCompleted",
    "controlBootstrapCompleted",
    "controlReady",
    "firstVideoFrame",
    "steadyMediaObserved",
    "rtcHealthSnapshot",
    "videoSurfaceAttached",
    "videoSurfaceSized",
    "videoSurfaceRendererReady",
}
CONTEXT_EVENTS = PEER_CONTEXT_EVENTS | {
    "sessionReady",
    "terminalSelected",
} | set(CORE_CLEANUP_EVENTS)


def load_base_analyzer() -> ModuleType:
    spec = importlib.util.spec_from_file_location("ios_runtime_trace_analyzer", ANALYZER_PATH)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"unable to load analyzer: {ANALYZER_PATH}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def payload_for(row: dict[str, Any]) -> dict[str, Any]:
    payload = row.get("payload")
    return payload if isinstance(payload, dict) else {}


def is_trace_integer(value: Any) -> bool:
    return type(value) is int


def row_seq(row: dict[str, Any]) -> int:
    value = row.get("seq")
    return value if is_trace_integer(value) else sys.maxsize


def row_reference(row: dict[str, Any]) -> dict[str, Any]:
    return {
        "event": row.get("event"),
        "seq": row.get("seq"),
        "tsMs": row.get("tsMs"),
        "file": row.get("_file"),
        "line": row.get("_line"),
    }


def event_rows(
    rows: list[dict[str, Any]],
    events: str | set[str],
) -> list[dict[str, Any]]:
    expected = {events} if isinstance(events, str) else events
    return [row for row in rows if row.get("event") in expected]


def selected_control_ready_rows(
    rows: list[dict[str, Any]],
) -> tuple[list[dict[str, Any]], bool, int]:
    canonical = event_rows(rows, "controlReady")
    legacy = event_rows(rows, "controlBootstrapCompleted")
    if canonical:
        return canonical, False, len(legacy)
    return legacy, bool(legacy), 0


def make_check(
    observed: list[dict[str, Any]],
    *,
    passed: bool | None = None,
    details: dict[str, Any] | None = None,
) -> dict[str, Any]:
    check = {
        "passed": bool(observed) if passed is None else passed,
        "observed": [row_reference(row) for row in observed],
    }
    if details is not None:
        check["details"] = details
    return check


def record_causal_order(
    before: dict[str, Any],
    after: dict[str, Any],
    expected: str,
    violations: list[dict[str, Any]],
    tolerated_reorders: list[dict[str, Any]],
) -> None:
    before_seq = row_seq(before)
    after_seq = row_seq(after)
    if before_seq < after_seq:
        return
    reorder_rows = before_seq - after_seq
    item = {
        "type": "streaming-order",
        "expected": expected,
        "reorderRows": reorder_rows,
        "observed": [row_reference(before), row_reference(after)],
    }
    if reorder_rows <= TRACE_REORDER_WINDOW_ROWS:
        tolerated_reorders.append(item)
    else:
        violations.append(item)


def validate_channel_profiles(row: dict[str, Any]) -> dict[str, Any]:
    payload = payload_for(row)
    profiles = payload.get("profiles")
    observed: dict[str, dict[str, Any]] = {}
    errors: list[dict[str, Any]] = []
    if not isinstance(profiles, list):
        errors.append({"type": "profiles-type", "expected": "array"})
    else:
        for index, profile in enumerate(profiles):
            if not isinstance(profile, dict):
                errors.append({"type": "profile-type", "index": index})
                continue
            label = profile.get("label")
            if not isinstance(label, str) or not label:
                errors.append({"type": "profile-label", "index": index})
                continue
            if label in observed:
                errors.append({"type": "duplicate-profile", "label": label})
                continue
            observed[label] = {
                "protocol": profile.get("protocol"),
                "ordered": profile.get("ordered"),
            }

    expected_labels = set(EXPECTED_CHANNEL_PROFILES)
    observed_labels = set(observed)
    if observed_labels != expected_labels:
        errors.append(
            {
                "type": "profile-labels",
                "missing": sorted(expected_labels - observed_labels),
                "unexpected": sorted(observed_labels - expected_labels),
            }
        )
    for label, expected in EXPECTED_CHANNEL_PROFILES.items():
        actual = observed.get(label)
        if actual is not None and actual != expected:
            errors.append(
                {
                    "type": "profile-contract",
                    "label": label,
                    "expected": expected,
                    "observed": actual,
                }
            )
    if payload.get("channelCount") != len(EXPECTED_CHANNEL_PROFILES):
        errors.append(
            {
                "type": "channel-count",
                "expected": len(EXPECTED_CHANNEL_PROFILES),
                "observed": payload.get("channelCount"),
            }
        )
    return {
        "reference": row_reference(row),
        "valid": not errors,
        "profiles": observed,
        "errors": errors,
    }


def has_selected_remote_candidate(row: dict[str, Any]) -> bool:
    value = payload_for(row).get("selectedRemoteCandidateType")
    if not isinstance(value, str):
        return False
    return value.strip().lower() not in {"", "none", "null", "unknown", "unsupported"}


def remote_ice_application_rows(rows: list[dict[str, Any]]) -> list[dict[str, Any]]:
    evidence: list[dict[str, Any]] = []
    for row in rows:
        event = row.get("event")
        payload = payload_for(row)
        if event == "remoteIceBatchApplied":
            candidate_count = payload.get("candidateCount")
            if candidate_count is None or (
                is_trace_integer(candidate_count) and candidate_count > 0
            ):
                evidence.append(row)
        elif event == "rtcHealthSnapshot" and has_selected_remote_candidate(row):
            evidence.append(row)
    return evidence


def has_positive_dimensions(row: dict[str, Any], width_key: str, height_key: str) -> bool:
    payload = payload_for(row)
    width = payload.get(width_key)
    height = payload.get(height_key)
    numeric = (int, float)
    return (
        isinstance(width, numeric)
        and not isinstance(width, bool)
        and isinstance(height, numeric)
        and not isinstance(height, bool)
        and width > 0
        and height > 0
    )


def has_positive_surface_size(row: dict[str, Any]) -> bool:
    return has_positive_dimensions(row, "width", "height")


def has_positive_renderer_size(row: dict[str, Any]) -> bool:
    return has_positive_dimensions(row, "frameWidth", "frameHeight")


def is_playing_state(row: dict[str, Any]) -> bool:
    return (
        row.get("event") == "streamingStateChanged"
        and payload_for(row).get("state") == "playing"
    )


def peer_epoch_report(
    peer_epoch: int,
    rows: list[dict[str, Any]],
) -> dict[str, Any]:
    ordered_rows = sorted(rows, key=row_seq)
    answers = event_rows(ordered_rows, "answerApplied")
    local_started = event_rows(ordered_rows, "localIceStarted")
    local_completed = event_rows(ordered_rows, "localIceCompleted")
    remote_received = event_rows(ordered_rows, "remoteIceBatchReceived")
    remote_applied = remote_ice_application_rows(ordered_rows)
    remote_completed = event_rows(ordered_rows, "remoteIceCompleted")
    connected = event_rows(ordered_rows, "peerConnected")
    profile_rows = event_rows(ordered_rows, "dataChannelProfilesCreated")
    profile_snapshots = [validate_channel_profiles(row) for row in profile_rows]
    profile_valid = (
        len(profile_snapshots) == 1
        and profile_snapshots[0]["valid"]
    )
    handshake_sent = event_rows(ordered_rows, "messageHandshakeSent")
    handshake_acked = event_rows(ordered_rows, "messageHandshakeAcked")
    post_handshake_completed = event_rows(ordered_rows, "messagePostHandshakeCompleted")
    pre_handshake_control = event_rows(
        ordered_rows,
        "controlBootstrapPreHandshakeCompleted",
    )
    control_bootstrap_completed = event_rows(ordered_rows, "controlBootstrapCompleted")
    control_ready, legacy_control_fallback, ignored_legacy_control = (
        selected_control_ready_rows(ordered_rows)
    )
    first_frame = event_rows(ordered_rows, "firstVideoFrame")
    steady_media = event_rows(ordered_rows, "steadyMediaObserved")
    surface_attached = event_rows(ordered_rows, "videoSurfaceAttached")
    surface_sizes = event_rows(ordered_rows, "videoSurfaceSized")
    positive_surface_sizes = [row for row in surface_sizes if has_positive_surface_size(row)]
    renderer_ready = event_rows(ordered_rows, "videoSurfaceRendererReady")
    positive_renderer_ready = [row for row in renderer_ready if has_positive_renderer_size(row)]

    checks = {
        "answerApplied": make_check(answers),
        "localIceStarted": make_check(local_started),
        "localIceCompleted": make_check(local_completed),
        "localIcePaired": make_check(
            local_started + local_completed,
            passed=bool(local_started) and len(local_started) == len(local_completed),
            details={"started": len(local_started), "completed": len(local_completed)},
        ),
        "negotiationPaired": make_check(
            answers + local_started + local_completed,
            passed=bool(answers)
            and len(answers) == len(local_started) == len(local_completed),
            details={
                "answers": len(answers),
                "localIceStarted": len(local_started),
                "localIceCompleted": len(local_completed),
            },
        ),
        "remoteIceBatchReceived": make_check(remote_received),
        "remoteIceApplied": make_check(
            remote_applied,
            details={
                "sources": sorted(
                    {
                        "selectedRemoteCandidate"
                        if row.get("event") == "rtcHealthSnapshot"
                        else "remoteIceBatchApplied"
                        for row in remote_applied
                    }
                )
            },
        ),
        "remoteIceCompleted": make_check(remote_completed),
        "peerConnected": make_check(connected),
        "fourChannelProfile": make_check(
            profile_rows,
            passed=profile_valid,
            details={"snapshots": profile_snapshots},
        ),
        "messageHandshakeSent": make_check(
            handshake_sent,
            passed=len(handshake_sent) == 1,
            details={"count": len(handshake_sent)},
        ),
        "messageHandshakeAcked": make_check(
            handshake_acked,
            passed=len(handshake_acked) == 1,
            details={"count": len(handshake_acked)},
        ),
        "messagePostHandshakeCompleted": make_check(
            post_handshake_completed,
            passed=len(post_handshake_completed) == 1,
            details={"count": len(post_handshake_completed)},
        ),
        "controlBootstrapCompleted": make_check(
            control_bootstrap_completed,
            passed=len(control_bootstrap_completed) == 1,
            details={
                "count": len(control_bootstrap_completed),
                "preHandshakeObserved": [
                    row_reference(row) for row in pre_handshake_control
                ],
            },
        ),
        "controlReady": make_check(
            control_ready,
            passed=len(control_ready) == 1,
            details={
                "count": len(control_ready),
                "events": sorted({str(row.get("event")) for row in control_ready}),
                "legacyFallback": legacy_control_fallback,
                "ignoredLegacyCount": ignored_legacy_control,
            },
        ),
        "firstVideoFrame": make_check(
            first_frame,
            passed=len(first_frame) == 1,
            details={"count": len(first_frame)},
        ),
        "steadyMediaObserved": make_check(steady_media),
        "videoSurfaceAttached": make_check(surface_attached),
        "videoSurfaceSized": make_check(
            surface_sizes,
            passed=bool(positive_surface_sizes),
            details={
                "observedCount": len(surface_sizes),
                "positiveSizes": [row_reference(row) for row in positive_surface_sizes],
            },
        ),
        "videoSurfaceRendererReady": make_check(
            renderer_ready,
            passed=bool(positive_renderer_ready),
            details={
                "observedCount": len(renderer_ready),
                "positiveFrames": [row_reference(row) for row in positive_renderer_ready],
            },
        ),
    }

    ordering_violations: list[dict[str, Any]] = []
    tolerated_reorders: list[dict[str, Any]] = []
    if answers and local_started:
        record_causal_order(
            answers[0],
            local_started[0],
            "answerApplied < localIceStarted",
            ordering_violations,
            tolerated_reorders,
        )
    if local_started and local_completed:
        record_causal_order(
            local_started[0],
            local_completed[-1],
            "localIceStarted < localIceCompleted",
            ordering_violations,
            tolerated_reorders,
        )
    if remote_received and remote_completed:
        record_causal_order(
            remote_received[0],
            remote_completed[-1],
            "remoteIceBatchReceived < remoteIceCompleted",
            ordering_violations,
            tolerated_reorders,
        )
    applied_batches = event_rows(ordered_rows, "remoteIceBatchApplied")
    if applied_batches and remote_completed:
        record_causal_order(
            applied_batches[0],
            remote_completed[-1],
            "remoteIceBatchApplied < remoteIceCompleted",
            ordering_violations,
            tolerated_reorders,
        )
    if handshake_sent and handshake_acked:
        record_causal_order(
            handshake_sent[0],
            handshake_acked[0],
            "messageHandshakeSent < messageHandshakeAcked",
            ordering_violations,
            tolerated_reorders,
        )
    if handshake_acked and post_handshake_completed:
        record_causal_order(
            handshake_acked[0],
            post_handshake_completed[0],
            "messageHandshakeAcked < messagePostHandshakeCompleted",
            ordering_violations,
            tolerated_reorders,
        )
    if post_handshake_completed and control_bootstrap_completed:
        record_causal_order(
            post_handshake_completed[0],
            control_bootstrap_completed[0],
            "messagePostHandshakeCompleted < controlBootstrapCompleted",
            ordering_violations,
            tolerated_reorders,
        )
    if control_bootstrap_completed and control_ready:
        record_causal_order(
            control_bootstrap_completed[0],
            control_ready[0],
            "controlBootstrapCompleted < controlReady",
            ordering_violations,
            tolerated_reorders,
        )
    if first_frame and steady_media:
        record_causal_order(
            first_frame[0],
            steady_media[0],
            "firstVideoFrame < steadyMediaObserved",
            ordering_violations,
            tolerated_reorders,
        )

    failed_checks = sorted(name for name, check in checks.items() if not check["passed"])
    return {
        "peerEpoch": peer_epoch,
        "rows": len(rows),
        "firstSeq": row_seq(ordered_rows[0]) if ordered_rows else None,
        "lastSeq": row_seq(ordered_rows[-1]) if ordered_rows else None,
        "checks": checks,
        "failedChecks": failed_checks,
        "orderingViolations": ordering_violations,
        "toleratedReorders": tolerated_reorders,
        "passed": not failed_checks and not ordering_violations,
    }


def playing_state_report(
    playing_rows: list[dict[str, Any]],
    rows_by_peer: dict[int, list[dict[str, Any]]],
) -> tuple[list[dict[str, Any]], list[dict[str, Any]]]:
    mappings: list[dict[str, Any]] = []
    violations: list[dict[str, Any]] = []
    for playing in playing_rows:
        playing_seq = row_seq(playing)
        candidates: list[dict[str, Any]] = []
        peer_diagnostics: list[dict[str, Any]] = []
        for peer_epoch, peer_rows in sorted(rows_by_peer.items()):
            prerequisites = {
                "peerConnected": event_rows(peer_rows, "peerConnected"),
                "firstVideoFrame": event_rows(peer_rows, "firstVideoFrame"),
                "controlReady": selected_control_ready_rows(peer_rows)[0],
            }
            missing = sorted(name for name, observed in prerequisites.items() if not observed)
            if missing:
                peer_diagnostics.append({"peerEpoch": peer_epoch, "missing": missing})
                continue
            evidence = {
                name: sorted(observed, key=row_seq)[0]
                for name, observed in prerequisites.items()
            }
            last_prerequisite_seq = max(row_seq(row) for row in evidence.values())
            reorder_rows = max(0, last_prerequisite_seq - playing_seq)
            peer_diagnostics.append(
                {
                    "peerEpoch": peer_epoch,
                    "lastPrerequisiteSeq": last_prerequisite_seq,
                    "reorderRows": reorder_rows,
                }
            )
            if reorder_rows <= TRACE_REORDER_WINDOW_ROWS:
                candidates.append(
                    {
                        "peerEpoch": peer_epoch,
                        "lastPrerequisiteSeq": last_prerequisite_seq,
                        "reorderRows": reorder_rows,
                        "evidence": {
                            name: row_reference(row) for name, row in evidence.items()
                        },
                    }
                )

        if not candidates:
            violations.append(
                {
                    "type": "playing-prerequisites",
                    "playing": row_reference(playing),
                    "reorderWindowRows": TRACE_REORDER_WINDOW_ROWS,
                    "peerEpochs": peer_diagnostics,
                }
            )
            continue
        selected = max(candidates, key=lambda item: item["lastPrerequisiteSeq"])
        mappings.append(
            {
                "playing": row_reference(playing),
                "peerEpoch": selected["peerEpoch"],
                "reordered": selected["reorderRows"] > 0,
                "reorderRows": selected["reorderRows"],
                "evidence": selected["evidence"],
            }
        )
    return mappings, violations


def attempt_report(
    identity: tuple[str, str, int],
    rows: list[dict[str, Any]],
    peer_context_violations: list[dict[str, Any]],
) -> dict[str, Any]:
    session_id, attempt_id, generation = identity
    ordered_rows = sorted(rows, key=row_seq)
    rows_by_peer: dict[int, list[dict[str, Any]]] = defaultdict(list)
    for row in ordered_rows:
        peer_epoch = payload_for(row).get("peerEpoch")
        if is_trace_integer(peer_epoch) and peer_epoch > 0:
            rows_by_peer[peer_epoch].append(row)

    peers = [
        peer_epoch_report(peer_epoch, peer_rows)
        for peer_epoch, peer_rows in sorted(rows_by_peer.items())
    ]
    complete_peers = [peer for peer in peers if peer["passed"]]
    session_ready = event_rows(ordered_rows, "sessionReady")
    playing_rows = [row for row in ordered_rows if is_playing_state(row)]
    playing_mappings, playing_violations = playing_state_report(playing_rows, rows_by_peer)
    terminal = event_rows(ordered_rows, "terminalSelected")
    cleanup = {event: event_rows(ordered_rows, event) for event in CORE_CLEANUP_EVENTS}
    targets = sorted(
        {
            target
            for row in ordered_rows
            if row.get("event") == "streamLaunchStarted"
            for target in [payload_for(row).get("target")]
            if isinstance(target, str) and target
        }
    )
    violations: list[dict[str, Any]] = []
    tolerated_reorders: list[dict[str, Any]] = []

    if len(session_ready) != 1:
        violations.append(
            {
                "type": "streaming-anchor-count",
                "anchor": "sessionReady",
                "expected": 1,
                "observed": len(session_ready),
            }
        )
    if not complete_peers:
        violations.append(
            {
                "type": "no-complete-peer-epoch",
                "peerEpochs": [
                    {
                        "peerEpoch": peer["peerEpoch"],
                        "failedChecks": peer["failedChecks"],
                        "orderingViolations": len(peer["orderingViolations"]),
                    }
                    for peer in peers
                ],
            }
        )
    if not playing_rows:
        violations.append(
            {
                "type": "streaming-anchor-count",
                "anchor": "streamingStateChanged:playing",
                "expected": "at least 1",
                "observed": 0,
            }
        )
    violations.extend(playing_violations)
    if len(terminal) != 1:
        violations.append(
            {
                "type": "streaming-anchor-count",
                "anchor": "terminalSelected",
                "expected": 1,
                "observed": len(terminal),
            }
        )
    for event, observed in cleanup.items():
        if len(observed) != 1:
            violations.append(
                {
                    "type": "streaming-cleanup-count",
                    "anchor": event,
                    "expected": 1,
                    "observed": len(observed),
                }
            )

    if session_ready:
        for peer in complete_peers:
            answers = event_rows(rows_by_peer[peer["peerEpoch"]], "answerApplied")
            if answers:
                before_count = len(violations)
                reorder_count = len(tolerated_reorders)
                record_causal_order(
                    session_ready[0],
                    answers[0],
                    "sessionReady < answerApplied",
                    violations,
                    tolerated_reorders,
                )
                if len(violations) > before_count:
                    violations[-1]["peerEpoch"] = peer["peerEpoch"]
                if len(tolerated_reorders) > reorder_count:
                    tolerated_reorders[-1]["peerEpoch"] = peer["peerEpoch"]

    if len(terminal) == 1 and complete_peers:
        for peer in complete_peers:
            peer_rows = rows_by_peer[peer["peerEpoch"]]
            last_core_row = max(
                (row for row in peer_rows if row.get("event") in PEER_CONTEXT_EVENTS),
                key=row_seq,
            )
            before_count = len(violations)
            reorder_count = len(tolerated_reorders)
            record_causal_order(
                last_core_row,
                terminal[0],
                "core evidence < terminalSelected",
                violations,
                tolerated_reorders,
            )
            if len(violations) > before_count:
                violations[-1]["peerEpoch"] = peer["peerEpoch"]
            if len(tolerated_reorders) > reorder_count:
                tolerated_reorders[-1]["peerEpoch"] = peer["peerEpoch"]
        for playing in playing_rows:
            record_causal_order(
                playing,
                terminal[0],
                "playing < terminalSelected",
                violations,
                tolerated_reorders,
            )

    cleanup_sequence: list[dict[str, Any]] = []
    if len(terminal) == 1:
        cleanup_sequence.append(terminal[0])
    if all(len(cleanup[event]) == 1 for event in CORE_CLEANUP_EVENTS):
        cleanup_sequence.extend(cleanup[event][0] for event in CORE_CLEANUP_EVENTS)
    if len(cleanup_sequence) == len(CORE_CLEANUP_EVENTS) + 1:
        sequences = [row_seq(row) for row in cleanup_sequence]
        if any(current <= previous for previous, current in zip(sequences, sequences[1:])):
            violations.append(
                {
                    "type": "streaming-cleanup-order",
                    "expected": ["terminalSelected", *CORE_CLEANUP_EVENTS],
                    "observed": [row_reference(row) for row in cleanup_sequence],
                }
            )

    return {
        "sessionId": session_id,
        "attemptId": attempt_id,
        "generation": generation,
        "targets": targets,
        "rows": len(rows),
        "sessionReady": [row_reference(row) for row in session_ready],
        "peerEpochs": peers,
        "completePeerEpochs": [peer["peerEpoch"] for peer in complete_peers],
        "playingStates": playing_mappings,
        "terminal": [row_reference(row) for row in terminal],
        "cleanup": {
            event: [row_reference(row) for row in observed]
            for event, observed in cleanup.items()
        },
        "contextViolations": peer_context_violations,
        "toleratedReorders": tolerated_reorders,
        "violations": violations,
        "passed": not violations and not peer_context_violations,
    }


def streaming_core_report(rows: list[dict[str, Any]]) -> dict[str, Any]:
    streaming_rows = [row for row in rows if row.get("domain") == "ios-streaming"]
    grouped: dict[tuple[str, str, int], list[dict[str, Any]]] = defaultdict(list)
    peer_context_by_attempt: dict[tuple[str, str, int], list[dict[str, Any]]] = defaultdict(list)
    context_violations: list[dict[str, Any]] = []
    generations_by_attempt: dict[tuple[str, str], set[int]] = defaultdict(set)

    for row in streaming_rows:
        event = str(row.get("event"))
        payload = payload_for(row)
        attempt_id = payload.get("attemptId")
        generation = payload.get("generation")
        missing: list[str] = []
        if not isinstance(attempt_id, str) or not attempt_id or attempt_id == "none":
            missing.append("attemptId")
        if not is_trace_integer(generation):
            missing.append("generation")
        if missing:
            if event in CONTEXT_EVENTS or is_playing_state(row):
                context_violations.append(
                    {
                        "type": "missing-streaming-context",
                        "missingFields": missing,
                        **row_reference(row),
                    }
                )
            continue

        session_id = str(row.get("sessionId"))
        identity = (session_id, attempt_id, generation)
        grouped[identity].append(row)
        generations_by_attempt[(session_id, attempt_id)].add(generation)
        if event in PEER_CONTEXT_EVENTS:
            peer_epoch = payload.get("peerEpoch")
            if not is_trace_integer(peer_epoch) or peer_epoch <= 0:
                violation = {
                    "type": "missing-peer-context",
                    "sessionId": session_id,
                    "attemptId": attempt_id,
                    "generation": generation,
                    "missingFields": ["peerEpoch"],
                    **row_reference(row),
                }
                context_violations.append(violation)
                peer_context_by_attempt[identity].append(violation)

    generation_violations: list[dict[str, Any]] = []
    for (session_id, attempt_id), generations in generations_by_attempt.items():
        if len(generations) > 1:
            generation_violations.append(
                {
                    "type": "attempt-generation-conflict",
                    "sessionId": session_id,
                    "attemptId": attempt_id,
                    "generations": sorted(generations),
                }
            )

    attempts = [
        attempt_report(
            identity,
            attempt_rows,
            peer_context_by_attempt.get(identity, []),
        )
        for identity, attempt_rows in sorted(grouped.items())
    ]
    violations = [*context_violations, *generation_violations]
    for attempt in attempts:
        violations.extend(
            {
                "sessionId": attempt["sessionId"],
                "attemptId": attempt["attemptId"],
                "generation": attempt["generation"],
                **violation,
            }
            for violation in attempt["violations"]
        )
    if not attempts:
        violations.append({"type": "missing-streaming-attempt"})

    return {
        "gate": "PASS" if attempts and not violations else "FAIL",
        "rows": len(streaming_rows),
        "attempts": attempts,
        "contextViolations": context_violations,
        "generationViolations": generation_violations,
        "violations": violations,
    }


def build_report(
    analyzer: ModuleType,
    files: list[Path],
    rows: list[dict[str, Any]],
    parsing: dict[str, Any],
) -> dict[str, Any]:
    sequence = analyzer.sequence_report(rows)
    budget = analyzer.budget_report(files, rows)
    streaming_core = streaming_core_report(rows)
    failures = {
        "noRows": int(not rows),
        "invalidRows": len(parsing["invalidRows"]),
        "schemaViolations": len(parsing["schemaViolations"]),
        "sequenceViolations": len(sequence["violations"]),
        "privacyViolations": len(parsing["privacyViolations"]),
        "budgetViolations": len(budget["violations"]),
        "streamingCoreViolations": len(streaming_core["violations"]),
    }
    timestamps = [row["tsMs"] for row in rows if is_trace_integer(row.get("tsMs"))]
    return {
        "gate": "PASS" if not any(failures.values()) else "FAIL",
        "scope": {
            "files": [str(path) for path in files],
            "rows": len(rows),
            "sessions": sorted({str(row.get("sessionId")) for row in rows}),
            "firstTsMs": min(timestamps) if timestamps else None,
            "lastTsMs": max(timestamps) if timestamps else None,
            "profiles": analyzer.count_values(rows, "traceProfile"),
        },
        "failures": failures,
        "schema": {
            "invalidRows": parsing["invalidRows"],
            "truncatedTails": parsing["truncatedTails"],
            "violations": parsing["schemaViolations"],
        },
        "sequence": sequence,
        "privacy": {"violations": parsing["privacyViolations"]},
        "budget": budget,
        "streamingCore": streaming_core,
    }


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("inputs", nargs="+", help="JSONL files or runtime trace directories")
    parser.add_argument("--session-id", help="only analyze one launch session")
    parser.add_argument("--strict", action="store_true", help="exit 2 when the gate fails")
    parser.add_argument("--pretty", action="store_true", help="pretty-print JSON")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    analyzer = load_base_analyzer()
    files = analyzer.discover_files(args.inputs)
    if not files:
        print(json.dumps({"gate": "FAIL", "error": "no trace files found"}, ensure_ascii=False))
        return 2
    rows, parsing = analyzer.parse_rows(files, args.session_id)
    report = build_report(analyzer, files, rows, parsing)
    print(
        json.dumps(
            report,
            ensure_ascii=False,
            indent=2 if args.pretty else None,
            sort_keys=True,
        )
    )
    return 2 if args.strict and report["gate"] != "PASS" else 0


if __name__ == "__main__":
    sys.exit(main())
