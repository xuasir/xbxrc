#!/usr/bin/env python3
"""Lifecycle reconnect gate for healthy-network rebuildPeerConnection regressions."""

from __future__ import annotations

import argparse
import json
import sys
import time
from pathlib import Path
from typing import Any


DEFAULT_RUNTIME_LOG_DIR = Path("runtime-logs")
LOCAL_RECOVERY_RECONNECT_REASONS = {
    "receiverWaitingKeyframe",
    "livenessNoProgressTimeout",
    "rtcConnectionDisconnected",
}
ACCEPTED_LIFECYCLE_BLOCK_REASONS = {
    "lifecycleGate:connectedHealthyNoProgress",
    "lifecycleGate:displaySupplyNoPendingLocal",
    "lifecycleGate:transientDisconnectedWithFreshMedia",
    "lifecycleGate:transportRebuildInFlightNoProgress",
}


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
    value = event.get("tsMs")
    if isinstance(value, (int, float)):
        return float(value)
    payload = event.get("payload")
    if isinstance(payload, dict):
        value = payload.get("tsMs")
        if isinstance(value, (int, float)):
            return float(value)
    return None


def origin_ms(events: list[dict[str, Any]]) -> float:
    timestamps = [ts for event in events if (ts := event_ts_ms(event)) is not None]
    return min(timestamps) if timestamps else 0.0


def rel_s(event: dict[str, Any], origin: float) -> float | None:
    ts = event_ts_ms(event)
    if ts is None:
        return None
    return (ts - origin) / 1000.0


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


def stats_snapshot_payload(event: dict[str, Any]) -> dict[str, Any] | None:
    if (event.get("event") or event.get("name") or "") != "statsSnapshot":
        return None
    payload = event.get("payload") or event
    stats = payload.get("stats") if isinstance(payload.get("stats"), dict) else payload
    return stats if isinstance(stats, dict) else None


def as_float(value: Any) -> float | None:
    if isinstance(value, (int, float)):
        return float(value)
    return None


def first_value(obj: dict[str, Any], *keys: str) -> Any:
    for key in keys:
        if key in obj:
            return obj[key]
    return None


def first_not_none(*values: Any) -> Any:
    for value in values:
        if value is not None:
            return value
    return None


def nested_get(obj: dict[str, Any], *keys: str) -> Any:
    cur: Any = obj
    for key in keys:
        if not isinstance(cur, dict):
            return None
        cur = cur.get(key)
    return cur


def stats_twcc_healthy(stats: dict[str, Any]) -> bool:
    delivery = as_float(
        first_value(
            stats,
            "video_twcc_delivery_ratio",
            "videoTwccDeliveryRatio",
            "deliveryRatio",
        )
    )
    loss = as_float(
        first_value(stats, "video_twcc_loss_ratio", "videoTwccLossRatio", "lossRatio")
    )
    nested_delivery = as_float(nested_get(stats, "twcc", "deliveryRatio"))
    nested_loss = as_float(nested_get(stats, "twcc", "lossRatio"))
    delivery = delivery if delivery is not None else nested_delivery
    loss = loss if loss is not None else nested_loss
    return (
        delivery is not None
        and loss is not None
        and delivery >= 0.95
        and loss <= 0.05
    )


def stats_output_serviceable(stats: dict[str, Any]) -> bool:
    present_fps = as_float(first_value(stats, "present_fps", "presentFps", "fps"))
    decode_fps = as_float(first_value(stats, "decode_fps", "decodeFps"))
    if present_fps is not None and decode_fps is not None:
        if present_fps >= 45.0 and decode_fps >= 45.0:
            return True
    chain_health = first_value(stats, "chain_health", "chainHealth")
    receive_display_state = first_value(
        stats, "receive_display_state", "receiveDisplayState"
    )
    reference_chain_state = first_value(
        stats, "reference_chain_state", "referenceChainState"
    )
    nested_display_state = nested_get(stats, "recovery", "receiveDisplayState")
    return (
        chain_health == "healthy"
        or receive_display_state == "display-stable"
        or nested_display_state == "display-stable"
        or reference_chain_state in {"continuous", "repairing"}
    )


def stats_transport_connected(stats: dict[str, Any]) -> bool:
    transport = first_value(stats, "transport_state", "transportState")
    nested_transport = nested_get(stats, "transport", "state")
    return transport == "Connected" or nested_transport == "Connected"


def healthy_stats_samples(events: list[dict[str, Any]], origin: float) -> list[dict[str, Any]]:
    samples: list[dict[str, Any]] = []
    for event in events:
        stats = stats_snapshot_payload(event)
        if stats is None:
            continue
        if not stats_transport_connected(stats):
            continue
        if not stats_twcc_healthy(stats):
            continue
        if not stats_output_serviceable(stats):
            continue
        rel = rel_s(event, origin)
        if rel is None:
            continue
        samples.append(
            {
                "seq": event.get("seq"),
                "t": round(rel, 3),
                "presentFps": as_float(first_value(stats, "present_fps", "presentFps", "fps")),
                "decodeFps": as_float(first_value(stats, "decode_fps", "decodeFps")),
                "twccDelivery": first_not_none(
                    as_float(
                        first_value(
                            stats, "video_twcc_delivery_ratio", "videoTwccDeliveryRatio"
                        )
                    ),
                    as_float(nested_get(stats, "twcc", "deliveryRatio")),
                ),
                "twccLoss": first_not_none(
                    as_float(
                        first_value(stats, "video_twcc_loss_ratio", "videoTwccLossRatio")
                    ),
                    as_float(nested_get(stats, "twcc", "lossRatio")),
                ),
            }
        )
    return samples


def ice_healthy_samples(events: list[dict[str, Any]], origin: float) -> list[dict[str, Any]]:
    samples: list[dict[str, Any]] = []
    for event in events:
        if (event.get("event") or "") != "iceConnectivityProbe":
            continue
        payload = event.get("payload") or {}
        if not isinstance(payload, dict):
            continue
        if payload.get("hasSelectedOrNominatedPair") is not True:
            continue
        if payload.get("directChecksWithoutResponse") is True:
            continue
        failed = payload.get("failedPairCount")
        if isinstance(failed, (int, float)) and failed > 0:
            continue
        rel = rel_s(event, origin)
        if rel is None:
            continue
        samples.append(
            {
                "seq": event.get("seq"),
                "t": round(rel, 3),
                "failedPairCount": failed,
                "responsesReceivedTotal": payload.get("responsesReceivedTotal"),
            }
        )
    return samples


def is_after_healthy(event: dict[str, Any], origin: float, healthy_start_s: float | None) -> bool:
    if healthy_start_s is None:
        return False
    rel = rel_s(event, origin)
    return rel is not None and rel >= healthy_start_s


def collect_reconnect_events(
    events: list[dict[str, Any]], origin: float, healthy_start_s: float | None
) -> dict[str, Any]:
    runtime_consumed: list[dict[str, Any]] = []
    media_reconnects: list[dict[str, Any]] = []
    rebuilds: list[dict[str, Any]] = []
    lifecycle_blocks: list[dict[str, Any]] = []
    accepted_lifecycle_blocks: list[dict[str, Any]] = []
    connected_healthy_blocks: list[dict[str, Any]] = []

    for event in events:
        if not is_after_healthy(event, origin, healthy_start_s):
            continue
        name = event.get("event") or ""
        payload = event.get("payload") or {}
        payload = payload if isinstance(payload, dict) else {}
        rel = rel_s(event, origin)
        row = {"seq": event.get("seq"), "t": round(rel, 3) if rel is not None else None}

        if name == "runtimeReconnectConsumed":
            reason = payload.get("reason")
            domain = payload.get("reasonDomain")
            if reason in LOCAL_RECOVERY_RECONNECT_REASONS:
                runtime_consumed.append({**row, "reason": reason, "reasonDomain": domain})

        if name == "mediaTransportReconnect":
            reason = str(payload.get("reason") or "")
            if any(local_reason in reason for local_reason in LOCAL_RECOVERY_RECONNECT_REASONS):
                media_reconnects.append({**row, "reason": reason, "count": payload.get("count")})

        if name == "videoIngressTermination":
            cause = payload.get("cause")
            upstream = payload.get("upstreamCause")
            if cause == "rebuildPeerConnection" or upstream == "rebuildPeerConnection":
                rebuilds.append({**row, "source": "videoIngressTermination"})

        if name == "runtimeLog":
            message = str(payload.get("message") or "")
            if "rx closed cause=rebuildPeerConnection" in message:
                rebuilds.append({**row, "source": "runtimeLog"})

        if name == "runtimeReconnectBlocked":
            block_reason = str(payload.get("blockReason") or "")
            if block_reason.startswith("lifecycleGate:"):
                block = {
                    **row,
                    "blockReason": block_reason,
                    "reason": payload.get("reason"),
                    "reasonDomain": payload.get("reasonDomain"),
                }
                lifecycle_blocks.append(block)
                if block_reason in ACCEPTED_LIFECYCLE_BLOCK_REASONS:
                    accepted_lifecycle_blocks.append(block)
                if block_reason == "lifecycleGate:connectedHealthyNoProgress":
                    connected_healthy_blocks.append(block)

        if name == "recoveryDecisionLedger":
            gate = str(payload.get("gateResult") or "")
            if "reconnectBlocked:lifecycleGate:" in gate:
                block = {
                    **row,
                    "gateResult": gate,
                    "actionSelected": payload.get("actionSelected"),
                    "proposalReasonLabel": payload.get("proposalReasonLabel"),
                }
                lifecycle_blocks.append(block)
                if any(
                    block_reason in gate
                    for block_reason in ACCEPTED_LIFECYCLE_BLOCK_REASONS
                ):
                    accepted_lifecycle_blocks.append(block)
                if "lifecycleGate:connectedHealthyNoProgress" in gate:
                    connected_healthy_blocks.append(block)

    return {
        "runtimeLocalReconnectConsumed": runtime_consumed,
        "mediaLocalReconnectCandidates": media_reconnects,
        "rebuildPeerConnectionClosures": rebuilds,
        "lifecycleReconnectBlocks": lifecycle_blocks,
        "acceptedLifecycleBlocks": accepted_lifecycle_blocks,
        "connectedHealthyNoProgressBlocks": connected_healthy_blocks,
    }


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
        "--require-lifecycle-block",
        action="store_true",
        help="require at least one lifecycleGate block after the healthy window",
    )
    parser.add_argument(
        "--allow-rebuilds-after-healthy",
        type=int,
        default=0,
        help="allowed rebuildPeerConnection closures after the healthy window",
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

    events = load_events(trace)
    origin = origin_ms(events)
    freshness = trace_freshness_report(trace, args.max_age_seconds)
    healthy_stats = healthy_stats_samples(events, origin)
    healthy_ice = ice_healthy_samples(events, origin)
    healthy_start_s = healthy_stats[0]["t"] if healthy_stats else None
    reconnect = collect_reconnect_events(events, origin, healthy_start_s)

    failures: list[str] = []
    if freshness["freshnessGate"] != "PASS":
        failures.append("traceFreshness")
    if healthy_start_s is None:
        failures.append("missingHealthyTwccOutputWindow")
    if reconnect["runtimeLocalReconnectConsumed"]:
        failures.append("localRecoveryReconnectConsumedAfterHealthy")
    if len(reconnect["rebuildPeerConnectionClosures"]) > args.allow_rebuilds_after_healthy:
        failures.append("rebuildPeerConnectionAfterHealthy")
    reconnect_attempted_after_healthy = bool(
        reconnect["runtimeLocalReconnectConsumed"]
        or reconnect["mediaLocalReconnectCandidates"]
        or reconnect["rebuildPeerConnectionClosures"]
    )
    if (
        args.require_lifecycle_block
        and reconnect_attempted_after_healthy
        and not reconnect["acceptedLifecycleBlocks"]
    ):
        failures.append("missingLifecycleAcceptedBlock")

    report = {
        "trace": str(trace),
        "lifecycleReconnectGate": "PASS" if not failures else "FAIL",
        "failures": failures,
        "traceFreshness": freshness,
        "healthyWindow": {
            "startS": healthy_start_s,
            "healthyStatsSamples": healthy_stats[:5],
            "healthyStatsCount": len(healthy_stats),
            "healthyIceSamples": healthy_ice[:5],
            "healthyIceCount": len(healthy_ice),
        },
        "afterHealthy": {
            "runtimeLocalReconnectConsumed": reconnect["runtimeLocalReconnectConsumed"][:10],
            "runtimeLocalReconnectConsumedCount": len(
                reconnect["runtimeLocalReconnectConsumed"]
            ),
            "mediaLocalReconnectCandidates": reconnect["mediaLocalReconnectCandidates"][:10],
            "mediaLocalReconnectCandidateCount": len(
                reconnect["mediaLocalReconnectCandidates"]
            ),
            "rebuildPeerConnectionClosures": reconnect["rebuildPeerConnectionClosures"][:10],
            "rebuildPeerConnectionClosureCount": len(
                reconnect["rebuildPeerConnectionClosures"]
            ),
            "lifecycleReconnectBlocks": reconnect["lifecycleReconnectBlocks"][:10],
            "lifecycleReconnectBlockCount": len(reconnect["lifecycleReconnectBlocks"]),
            "acceptedLifecycleBlocks": reconnect["acceptedLifecycleBlocks"][:10],
            "acceptedLifecycleBlockCount": len(reconnect["acceptedLifecycleBlocks"]),
            "connectedHealthyNoProgressBlockCount": len(
                reconnect["connectedHealthyNoProgressBlocks"]
            ),
        },
    }
    print(json.dumps(report, indent=2, ensure_ascii=False))
    return 0 if not failures else 2


if __name__ == "__main__":
    raise SystemExit(main())
