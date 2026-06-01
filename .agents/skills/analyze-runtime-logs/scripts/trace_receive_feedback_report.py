#!/usr/bin/env python3
"""Receive feedback arbiter trace report: keyframe / NACK / reference-chain 验收。

用于 RFC receive-feedback-arbiter 合入后的 trace 回放：统计 receiveFeedbackDecision、
keyframeRequestOutcome、referenceChainStateChanged 与 Insert/Decode 纪律违规。
"""

from __future__ import annotations

import argparse
import json
import sys
from collections import Counter
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


def event_name(event: dict[str, Any]) -> str:
    return str(event.get("event") or event.get("name") or "")


def event_payload(event: dict[str, Any]) -> dict[str, Any]:
    payload = event.get("payload")
    if isinstance(payload, dict):
        return payload
    return event


def picture_recovery_phase(payload: dict[str, Any]) -> str:
    return str(
        payload.get("toPhase")
        or payload.get("to_phase")
        or payload.get("phase")
        or ""
    )


def chain_key(
    payload: dict[str, Any], current_ledger_generation: Any = None
) -> tuple[str, Any]:
    ledger_generation = (
        payload.get("ledgerGeneration")
        or payload.get("ledger_generation")
        or current_ledger_generation
    )
    if ledger_generation is not None:
        return ("ledgerGeneration", ledger_generation)
    episode = payload.get("episodeId") or payload.get("episode_id")
    epoch = payload.get("recoveryEpoch") or payload.get("recovery_epoch")
    rtp = payload.get("rtpTimestamp") or payload.get("rtp_timestamp")
    observation = payload.get("observationId") or payload.get("observation_id")
    if episode is not None:
        return ("episode", episode)
    if epoch is not None:
        return ("epoch", epoch)
    if rtp is not None:
        return ("rtp", rtp)
    return ("observation", observation)


def p95(values: list[float]) -> float | None:
    if not values:
        return None
    ordered = sorted(values)
    idx = int(round(0.95 * (len(ordered) - 1)))
    return ordered[idx]


SURFACE_PHASE_ACTION_STAGES = frozenset(
    {"priming", "steady", "repairing", "await-idr", "supply-break", "must-idr"}
)


def absorb_control_facts(payload: dict[str, Any], facts: dict[str, Any]) -> None:
    for json_key, snake in (
        ("keyframeRequired", "keyframe_required"),
        ("responseState", "response_state"),
        ("receiveDisplayState", "receive_display_state"),
    ):
        if json_key in payload:
            facts[json_key] = payload.get(json_key)
        elif snake in payload:
            facts[json_key] = payload.get(snake)


def ledger_display_closure_ok(facts: dict[str, Any]) -> bool:
    if facts.get("keyframeRequired") is True:
        return False
    response = facts.get("responseState")
    if response is not None and response != "usable-idr":
        return False
    if facts.get("receiveDisplayState") != "display-stable":
        return False
    return True


def is_display_stable_event(name: str, payload: dict[str, Any]) -> bool:
    if name == "stableServingSettled":
        return True
    if name == "pictureRecoveryTransition":
        return picture_recovery_phase(payload) == "DisplayStable"
    return False


def insert_action_stage(payload: dict[str, Any]) -> str | None:
    packet_stage = payload.get("packetRecoveryActionStage") or payload.get(
        "packet_recovery_action_stage"
    )
    if packet_stage is not None:
        return str(packet_stage)
    legacy_stage = payload.get("actionStage") or payload.get("action_stage")
    if legacy_stage is None:
        return None
    return str(legacy_stage)


def analyze(events: list[dict[str, Any]]) -> dict[str, Any]:
    feedback = Counter()
    keyframe_outcomes = Counter()
    reference_states = Counter()
    sparse_dwell_ms: list[float] = []
    sparse_active_start: float | None = None
    mismatch_total = 0
    sparse_mismatch_total = 0
    reference_stats_fallback_total = 0
    need_keyframe_non_idr_feed = 0
    session_violations = 0
    decoder_reset_violations = 0

    terminal_remote_no_usable_idr = 0
    terminal_remote_continuation_only = 0
    terminal_decoder_rejected_idr = 0
    terminal_no_clean_anchor_after_decode = 0
    terminal_any = 0
    usable_idr_observations = 0

    chain_sent = 0
    chain_response_keys: set[tuple[str, Any]] = set()
    chain_decoded_keys: set[tuple[str, Any]] = set()
    chain_anchor_keys: set[tuple[str, Any]] = set()
    chain_display_keys: set[tuple[str, Any]] = set()
    current_ledger_generation: Any = None
    sent_by_ledger_generation: Counter[Any] = Counter()
    response_by_ledger_generation: Counter[Any] = Counter()
    decoded_by_ledger_generation: Counter[Any] = Counter()
    anchor_by_ledger_generation: Counter[Any] = Counter()
    display_by_ledger_generation: Counter[Any] = Counter()
    terminal_by_ledger_generation: Counter[Any] = Counter()
    current_recovery_epoch: Any = None
    max_ledger_generation_by_epoch: dict[Any, Any] = {}
    epoch_isolation_violations = 0
    latest_control_facts: dict[str, Any] = {
        "keyframeRequired": None,
        "responseState": None,
        "receiveDisplayState": None,
    }
    display_stable_without_ledger_closure = 0
    insert_surface_phase_action_stage = 0
    insert_control_projection_mismatch = 0
    episode_projection_states = Counter()
    display_supply_starved_blockers = Counter()

    for event in events:
        name = event_name(event)
        payload = event_payload(event)
        gen = payload.get("ledgerGeneration") or payload.get("ledger_generation")
        if gen is not None:
            current_ledger_generation = gen
        recovery_epoch = payload.get("recoveryEpoch") or payload.get("recovery_epoch")
        if recovery_epoch is not None:
            if (
                current_recovery_epoch is not None
                and recovery_epoch != current_recovery_epoch
                and current_ledger_generation is not None
            ):
                previous_max = max_ledger_generation_by_epoch.get(current_recovery_epoch)
                if (
                    previous_max is not None
                    and current_ledger_generation == previous_max
                    and name
                    in (
                        "pictureRecoveryTransition",
                        "cleanAnchorCommitted",
                        "h264InspectionObserved",
                        "h264InspectionObservation",
                        "decodeOutputPathObserved",
                        "decodeOutputPathObservation",
                    )
                ):
                    epoch_isolation_violations += 1
            current_recovery_epoch = recovery_epoch
            if current_ledger_generation is not None:
                previous = max_ledger_generation_by_epoch.get(recovery_epoch)
                if previous is None or current_ledger_generation > previous:
                    max_ledger_generation_by_epoch[recovery_epoch] = current_ledger_generation

        if name == "receiveFeedbackDecision":
            absorb_control_facts(payload, latest_control_facts)
            episode_state = payload.get("episodeProjectionState") or payload.get(
                "episode_projection_state"
            )
            if episode_state:
                episode_projection_states[str(episode_state)] += 1
            supply_blocker = payload.get("displaySupplyStarvedBlocker") or payload.get(
                "display_supply_starved_blocker"
            )
            if supply_blocker:
                display_supply_starved_blockers[str(supply_blocker)] += 1
            feedback[payload.get("action") or "unknown"] += 1
            feedback[f"coalescing:{payload.get('coalescing')}"] += 1
            mismatch_total = max(
                mismatch_total, int(payload.get("arbiterMismatchTotal") or 0)
            )
            if payload.get("sparseActive") is True:
                ts = event.get("tsMs") or event.get("ts_ms")
                if isinstance(ts, (int, float)) and sparse_active_start is None:
                    sparse_active_start = float(ts)
            elif sparse_active_start is not None:
                ts = event.get("tsMs") or event.get("ts_ms")
                if isinstance(ts, (int, float)):
                    sparse_dwell_ms.append(float(ts) - sparse_active_start)
                sparse_active_start = None

        if name == "keyframeRequestOutcome":
            keyframe_outcomes[payload.get("outcome") or "unknown"] += 1

        if name == "referenceChainStateChanged":
            absorb_control_facts(payload, latest_control_facts)
            reference_states[payload.get("state") or "unknown"] += 1
            sparse_mismatch_total = max(
                sparse_mismatch_total,
                int(payload.get("sparseMustIdrMismatchTotal") or 0),
            )
            reference_stats_fallback_total = max(
                reference_stats_fallback_total,
                int(payload.get("referenceStatsFallbackTotal") or 0),
            )

        if name == "receivePictureRecoveryTerminal":
            absorb_control_facts(payload, latest_control_facts)
            reason = str(payload.get("reason") or "")
            terminal_any += 1
            if current_ledger_generation is not None:
                terminal_by_ledger_generation[current_ledger_generation] += 1
            if reason == "remote-no-usable-idr":
                terminal_remote_no_usable_idr += 1
            elif reason == "remote-continuation-only":
                terminal_remote_continuation_only += 1
            elif reason == "decoder-rejected-idr":
                terminal_decoder_rejected_idr += 1
            elif reason == "no-clean-anchor-after-decode":
                terminal_no_clean_anchor_after_decode += 1

        if name == "pictureRecoveryAuthority":
            if payload.get("authority") not in (None, "receive", "delegatedToReceive"):
                session_violations += 1
            if payload.get("sessionKeyframeInFlight"):
                session_violations += 1

        if name == "insertGateDecision":
            absorb_control_facts(payload, latest_control_facts)
            packet_stage = payload.get("packetRecoveryActionStage") or payload.get(
                "packet_recovery_action_stage"
            )
            if packet_stage is not None and str(packet_stage) in SURFACE_PHASE_ACTION_STAGES:
                insert_surface_phase_action_stage += 1
            stage = insert_action_stage(payload)
            if payload.get("decision") == "emit" and payload.get("keyframeRequired") is True:
                packet_stage = (
                    payload.get("packetRecoveryActionStage")
                    or payload.get("packet_recovery_action_stage")
                )
                reason = str(payload.get("reason") or "").lower()
                if packet_stage in ("wait_keyframe", "request_idr") and "idr" not in reason:
                    insert_control_projection_mismatch += 1
            if (
                payload.get("referenceState") == "need-keyframe"
                and payload.get("decision") == "emit"
                and "idr" not in str(payload.get("reason") or "").lower()
            ):
                need_keyframe_non_idr_feed += 1

        if is_display_stable_event(name, payload):
            if not ledger_display_closure_ok(latest_control_facts):
                display_stable_without_ledger_closure += 1

        if name == "cleanAnchorCommitted":
            chain_anchor_keys.add(chain_key(payload, current_ledger_generation))
            if current_ledger_generation is not None:
                anchor_by_ledger_generation[current_ledger_generation] += 1
        if name == "pictureRecoveryTransition":
            phase = picture_recovery_phase(payload)
            key = chain_key(payload, current_ledger_generation)
            if phase == "ResponseObserved":
                chain_response_keys.add(key)
                if current_ledger_generation is not None:
                    response_by_ledger_generation[current_ledger_generation] += 1
            elif phase == "AnchorSeen":
                chain_response_keys.add(key)
                if current_ledger_generation is not None:
                    response_by_ledger_generation[current_ledger_generation] += 1
            elif phase in ("Decoded", "PlaybackRecovered"):
                chain_decoded_keys.add(key)
                if current_ledger_generation is not None:
                    decoded_by_ledger_generation[current_ledger_generation] += 1
            elif phase in ("CleanAnchorCommitted", "FreshAnchorRecovered"):
                chain_anchor_keys.add(key)
                if current_ledger_generation is not None:
                    anchor_by_ledger_generation[current_ledger_generation] += 1
            elif phase == "DisplayStable":
                chain_display_keys.add(key)
                if current_ledger_generation is not None:
                    display_by_ledger_generation[current_ledger_generation] += 1
        if name == "stableServingSettled":
            chain_display_keys.add(chain_key(payload, current_ledger_generation))
            if current_ledger_generation is not None:
                display_by_ledger_generation[current_ledger_generation] += 1
        if name == "firstFrameLatencyObserved":
            key = chain_key(payload, current_ledger_generation)
            terminal = str(payload.get("terminalPhase") or payload.get("terminal_phase") or "")
            if payload.get("pliSentToFirstIdrPacketMs") is not None:
                chain_response_keys.add(key)
            if payload.get("firstIdrPacketToFirstDecodeMs") is not None:
                chain_decoded_keys.add(key)
            if payload.get("firstDecodeToCleanAnchorCommittedMs") is not None:
                chain_anchor_keys.add(key)
            if (
                payload.get("cleanAnchorCommittedToDisplayStableMs") is not None
                or terminal == "DisplayStable"
            ):
                chain_display_keys.add(key)
        if name == "keyframeRequestOutcome" and payload.get("outcome") == "sent":
            chain_sent += 1
            if current_ledger_generation is not None:
                sent_by_ledger_generation[current_ledger_generation] += 1
        if name in ("h264InspectionObserved", "h264InspectionObservation"):
            if payload.get("isIdr") or payload.get("is_idr"):
                chain_response_keys.add(chain_key(payload, current_ledger_generation))
                if current_ledger_generation is not None:
                    response_by_ledger_generation[current_ledger_generation] += 1
                if payload.get("bootstrapReady") or payload.get("bootstrap_ready"):
                    usable_idr_observations += 1
        if name in ("decodeOutputPathObserved", "decodeOutputPathObservation"):
            if payload.get("decoded") or payload.get("verdict") == "decoded-frame":
                chain_decoded_keys.add(chain_key(payload, current_ledger_generation))
                if current_ledger_generation is not None:
                    decoded_by_ledger_generation[current_ledger_generation] += 1

    if sparse_active_start is not None:
        sparse_dwell_ms.append(0.0)

    sent = keyframe_outcomes.get("sent", 0)
    coalesced = keyframe_outcomes.get("coalesced", 0)
    throttled = keyframe_outcomes.get("throttled", 0)
    feedback_decisions = sum(
        v for k, v in feedback.items() if not str(k).startswith("coalescing:")
    )

    def rate(numerator: int, denominator: int) -> float | None:
        if denominator <= 0:
            return None
        return round(numerator / denominator, 4)

    response_observed = len(chain_response_keys)
    decoded = len(chain_decoded_keys)
    clean_anchor = len(chain_anchor_keys)
    display_stable = len(chain_display_keys)

    rates = {
        "responseObservedRate": rate(response_observed, sent),
        "decodedRate": rate(decoded, sent),
        "cleanAnchorRate": rate(clean_anchor, sent),
        "displayStableRate": rate(display_stable, sent),
        "usableIdrRate": rate(usable_idr_observations, sent),
        "chainBuildSuccessRate": rate(clean_anchor, decoded) if decoded else None,
        "nackEffectiveRate": None,
    }

    gate_failures: list[str] = []
    if need_keyframe_non_idr_feed > 0:
        gate_failures.append("needKeyframeNonIdrFeedViolations")
    if mismatch_total > 0:
        gate_failures.append("arbiterMismatchTotal")
    if epoch_isolation_violations > 0:
        gate_failures.append("epochIsolationViolations")
    same_ledger_generation_closure_failures = 0
    for generation, sent_count in sent_by_ledger_generation.items():
        if sent_count <= 0:
            continue
        has_terminal = terminal_by_ledger_generation.get(generation, 0) > 0
        has_display = display_by_ledger_generation.get(generation, 0) > 0
        has_anchor = anchor_by_ledger_generation.get(generation, 0) > 0
        has_decoded = decoded_by_ledger_generation.get(generation, 0) > 0
        has_response = response_by_ledger_generation.get(generation, 0) > 0
        closed = has_display or has_anchor or has_terminal
        progressed = has_response or has_decoded or has_anchor or has_display or has_terminal
        if not closed and progressed and sent_count >= 3:
            same_ledger_generation_closure_failures += 1
    if same_ledger_generation_closure_failures > 0:
        gate_failures.append("sameLedgerGenerationClosure")
    if display_stable_without_ledger_closure > 0:
        gate_failures.append("displayStableWithoutLedgerClosure")
    if insert_surface_phase_action_stage > 0:
        gate_failures.append("insertSurfacePhaseActionStage")
    if insert_control_projection_mismatch > 0:
        gate_failures.append("insertControlProjectionMismatch")
    if sent > 0 and rates["responseObservedRate"] is not None:
        if rates["responseObservedRate"] < 0.2 and terminal_any == 0:
            gate_failures.append("lowResponseObservedRate")
        if rates["responseObservedRate"] < 0.2 and sent >= 5 and terminal_any == 0:
            gate_failures.append("silentStuck")
    receive_feedback_gate = "PASS" if not gate_failures else "FAIL"

    return {
        "feedbackActionCounts": dict(feedback),
        "keyframeOutcomeCounts": dict(keyframe_outcomes),
        "referenceStateCounts": dict(reference_states),
        "sparseActiveP95DwellMs": p95(sparse_dwell_ms),
        "arbiterMismatchTotal": mismatch_total,
        "sparseMustIdrMismatchTotal": sparse_mismatch_total,
        "referenceStatsFallbackTotal": reference_stats_fallback_total,
        "needKeyframeNonIdrFeedViolations": need_keyframe_non_idr_feed,
        "epochIsolationViolations": epoch_isolation_violations,
        "sameLedgerGenerationClosureFailures": same_ledger_generation_closure_failures,
        "displayStableWithoutLedgerClosure": display_stable_without_ledger_closure,
        "insertSurfacePhaseActionStage": insert_surface_phase_action_stage,
        "insertControlProjectionMismatch": insert_control_projection_mismatch,
        "sessionPictureRecoveryViolations": session_violations,
        "decoderResetViolations": decoder_reset_violations,
        "rates": rates,
        "receiveFeedbackGate": receive_feedback_gate,
        "receiveFeedbackGateFailures": gate_failures,
        "terminalRemoteNoUsableIdr": terminal_remote_no_usable_idr,
        "terminalRemoteContinuationOnly": terminal_remote_continuation_only,
        "terminalDecoderRejectedIdr": terminal_decoder_rejected_idr,
        "terminalNoCleanAnchorAfterDecode": terminal_no_clean_anchor_after_decode,
        "terminalAny": terminal_any,
        "keyframeChain": {
            "sent": chain_sent,
            "responseObserved": response_observed,
            "decoded": decoded,
            "cleanAnchorCommitted": clean_anchor,
            "displayStable": display_stable,
        },
        "episodeProjectionStateCounts": dict(episode_projection_states),
        "displaySupplyStarvedBlockerCounts": dict(display_supply_starved_blockers),
        "summary": {
            "receiveFeedbackDecisionEvents": feedback_decisions,
            "keyframeSent": sent,
            "keyframeCoalesced": coalesced,
            "keyframeThrottled": throttled,
            "referenceChainTransitions": sum(reference_states.values()),
        },
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "trace",
        type=Path,
        nargs="?",
        default=Path("runtime-logs/runtime-trace-1779953007765-1.jsonl"),
        help="runtime-trace JSONL path",
    )
    args = parser.parse_args()
    if not args.trace.exists():
        print(f"trace not found: {args.trace}", file=sys.stderr)
        return 1
    report = analyze(load_events(args.trace))
    print(json.dumps(report, indent=2, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
