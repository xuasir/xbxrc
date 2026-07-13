#!/usr/bin/env python3
"""Report browser WebRTC negotiation and first-frame behavior from runtime traces."""

from __future__ import annotations

import argparse
import json
import sys
import time
from pathlib import Path
from typing import Any


DEFAULT_RUNTIME_LOG_DIR = Path("runtime-logs")


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


def event_name(row: dict[str, Any]) -> str | None:
    name = row.get("event") or row.get("name")
    return name if isinstance(name, str) else None


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


def int_field(payload: dict[str, Any], *names: str) -> int | None:
    for name in names:
        value = payload.get(name)
        if isinstance(value, bool):
            continue
        if isinstance(value, int):
            return value
        if isinstance(value, float) and value.is_integer():
            return int(value)
    return None


def profile_family(profile: str | None) -> str | None:
    if not profile:
        return None
    normalized = profile.lower().strip()
    if normalized.startswith(("4d", "64")):
        return "high"
    if normalized.startswith("42e"):
        return "constrained-baseline"
    if normalized.startswith("420"):
        return "baseline"
    return "other"


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


def compact_h264_payload(payload: dict[str, Any]) -> dict[str, Any]:
    profile = text_field(payload, "profileLevelId", "profile_level_id")
    return {
        "payloadType": text_field(payload, "payloadType", "payload_type"),
        "profileLevelId": profile,
        "profileFamily": profile_family(profile),
        "packetizationMode": text_field(payload, "packetizationMode", "packetization_mode"),
        "rtpmap": text_field(payload, "rtpmap"),
        "rtcpFeedback": payload.get("rtcpFeedback") or payload.get("rtcp_feedback") or [],
        "spropParameterSetsPresent": bool(
            payload.get("spropParameterSetsPresent")
            or payload.get("sprop_parameter_sets_present")
        ),
    }


def compact_sdp_observation(row: dict[str, Any]) -> dict[str, Any]:
    payload = payload_for(row)
    h264_payloads = payload.get("h264Payloads")
    if not isinstance(h264_payloads, list):
        h264_payloads = payload.get("h264_payloads")
    compact_payloads = [
        compact_h264_payload(item)
        for item in h264_payloads
        if isinstance(item, dict)
    ]
    return {
        "seq": row.get("seq"),
        "tsMs": row.get("tsMs"),
        "stage": text_field(payload, "stage"),
        "length": int_field(payload, "length"),
        "hasAudio": payload.get("hasAudio"),
        "hasVideo": payload.get("hasVideo"),
        "hasApplication": payload.get("hasApplication"),
        "videoHeaderExtensionCount": len(payload.get("videoHeaderExtensions") or []),
        "videoSsrcCount": len(payload.get("videoSsrcs") or []),
        "h264Payloads": compact_payloads,
        "preferredH264Payload": compact_payloads[0] if compact_payloads else None,
    }


def browser_sdp_observations(rows: list[dict[str, Any]]) -> dict[str, Any]:
    by_stage: dict[str, dict[str, Any]] = {}
    all_observations: list[dict[str, Any]] = []
    for row in rows:
        if event_name(row) != "browserWebRtcSdpObserved":
            continue
        observation = compact_sdp_observation(row)
        stage = observation.get("stage")
        if isinstance(stage, str):
            by_stage[stage] = observation
        all_observations.append(observation)
    return {
        "byStage": by_stage,
        "observations": all_observations[:12],
        "observationCount": len(all_observations),
    }


def selected_remote_answer_from_timeline(rows: list[dict[str, Any]]) -> dict[str, Any] | None:
    for row in rows:
        if event_name(row) != "browserWebRtcTimelineObserved":
            continue
        payload = payload_for(row)
        if text_field(payload, "kind") != "remoteAnswerSet":
            continue
        profile = text_field(payload, "selectedProfileLevelId")
        return {
            "seq": row.get("seq"),
            "tsMs": row.get("tsMs"),
            "observedAtMs": number_field(payload, "observedAtMs"),
            "elapsedSinceBindMs": number_field(payload, "elapsedSinceBindMs"),
            "selectedPayloadType": text_field(payload, "selectedPayloadType"),
            "selectedMimeType": text_field(payload, "selectedMimeType"),
            "selectedProfileLevelId": profile,
            "profileFamily": profile_family(profile),
        }
    return None


def selected_remote_answer_from_sdp(sdp: dict[str, Any]) -> dict[str, Any] | None:
    by_stage = sdp.get("byStage")
    if not isinstance(by_stage, dict):
        return None
    remote_answer = by_stage.get("remoteAnswer")
    if not isinstance(remote_answer, dict):
        return None
    preferred_payload = remote_answer.get("preferredH264Payload")
    if not isinstance(preferred_payload, dict):
        return None
    profile = text_field(preferred_payload, "profileLevelId")
    return {
        "seq": remote_answer.get("seq"),
        "tsMs": remote_answer.get("tsMs"),
        "selectedPayloadType": text_field(preferred_payload, "payloadType"),
        "selectedMimeType": "video/H264",
        "selectedProfileLevelId": profile,
        "profileFamily": profile_family(profile),
        "source": "remoteAnswerSdpPreferredPayload",
    }


def compact_timeline_event(row: dict[str, Any]) -> dict[str, Any]:
    payload = payload_for(row)
    inbound = payload.get("inboundVideo")
    if not isinstance(inbound, dict):
        inbound = {}
    profile = text_field(payload, "selectedProfileLevelId")
    return {
        "seq": row.get("seq"),
        "tsMs": row.get("tsMs"),
        "kind": text_field(payload, "kind"),
        "observedAtMs": number_field(payload, "observedAtMs"),
        "elapsedSinceBindMs": number_field(payload, "elapsedSinceBindMs"),
        "elapsedSinceConnectedMs": number_field(payload, "elapsedSinceConnectedMs"),
        "connectionState": text_field(payload, "connectionState"),
        "iceConnectionState": text_field(payload, "iceConnectionState"),
        "iceGatheringState": text_field(payload, "iceGatheringState"),
        "signalingState": text_field(payload, "signalingState"),
        "trackKind": text_field(payload, "trackKind"),
        "selectedProfileLevelId": profile,
        "profileFamily": profile_family(profile),
        "selectedPayloadType": text_field(payload, "selectedPayloadType"),
        "selectedMimeType": text_field(payload, "selectedMimeType"),
        "packetsReceived": int_field(inbound, "packetsReceived"),
        "framesDecoded": int_field(inbound, "framesDecoded"),
        "keyFramesDecoded": int_field(inbound, "keyFramesDecoded"),
        "presentedFrames": int_field(payload, "presentedFrames"),
        "mediaTime": number_field(payload, "mediaTime"),
    }


def browser_timeline(rows: list[dict[str, Any]]) -> dict[str, Any]:
    events = [
        compact_timeline_event(row)
        for row in rows
        if event_name(row) == "browserWebRtcTimelineObserved"
    ]
    first_by_kind: dict[str, dict[str, Any]] = {}
    for item in events:
        kind = item.get("kind")
        if isinstance(kind, str) and kind not in first_by_kind:
            first_by_kind[kind] = item
    connected = next(
        (
            item
            for item in events
            if item.get("kind") == "connectionStateChanged"
            and item.get("connectionState") == "connected"
        ),
        None,
    )
    return {
        "eventCount": len(events),
        "firstByKind": first_by_kind,
        "connected": connected,
        "events": events[:80],
    }


def delta_value(delta: dict[str, Any], name: str) -> float:
    value = delta.get(name)
    return float(value) if isinstance(value, (int, float)) else 0.0


def compact_stats_sample(row: dict[str, Any]) -> dict[str, Any]:
    payload = payload_for(row)
    inbound = payload.get("inboundVideo")
    if not isinstance(inbound, dict):
        inbound = {}
    codec = payload.get("selectedCodec")
    if not isinstance(codec, dict):
        codec = {}
    delta = payload.get("delta")
    if not isinstance(delta, dict):
        delta = {}
    return {
        "seq": row.get("seq"),
        "tsMs": row.get("tsMs"),
        "sampledAtMs": number_field(payload, "sampledAtMs"),
        "connectionState": text_field(payload, "connectionState"),
        "codecMimeType": text_field(codec, "mimeType"),
        "codecPayloadType": codec.get("payloadType"),
        "codecFmtp": text_field(codec, "sdpFmtpLine"),
        "packetsReceived": int_field(inbound, "packetsReceived"),
        "bytesReceived": int_field(inbound, "bytesReceived"),
        "framesDecoded": int_field(inbound, "framesDecoded"),
        "keyFramesDecoded": int_field(inbound, "keyFramesDecoded"),
        "framesDropped": int_field(inbound, "framesDropped"),
        "pliCount": int_field(inbound, "pliCount"),
        "firCount": int_field(inbound, "firCount"),
        "nackCount": int_field(inbound, "nackCount"),
        "framesPerSecond": number_field(inbound, "framesPerSecond"),
        "frameWidth": int_field(inbound, "frameWidth"),
        "frameHeight": int_field(inbound, "frameHeight"),
        "decoderImplementation": text_field(inbound, "decoderImplementation"),
        "delta": delta,
    }


def browser_stats(rows: list[dict[str, Any]]) -> dict[str, Any]:
    samples = [
        compact_stats_sample(row)
        for row in rows
        if event_name(row) == "browserWebRtcStatsObserved"
    ]
    inbound_samples = [sample for sample in samples if sample.get("packetsReceived") is not None]
    first_inbound = next(
        (sample for sample in inbound_samples if (sample.get("packetsReceived") or 0) > 0),
        None,
    )
    first_decoded = next(
        (sample for sample in inbound_samples if (sample.get("framesDecoded") or 0) > 0),
        None,
    )
    first_keyframe = next(
        (sample for sample in inbound_samples if (sample.get("keyFramesDecoded") or 0) > 0),
        None,
    )
    totals = empty_delta_totals()
    for sample in samples:
        delta = sample.get("delta")
        if not isinstance(delta, dict):
            continue
        for key in totals:
            totals[key] += delta_value(delta, key)
    return {
        "sampleCount": len(samples),
        "inboundSampleCount": len(inbound_samples),
        "firstSample": samples[0] if samples else None,
        "firstInboundPacketSample": first_inbound,
        "firstDecodedSample": first_decoded,
        "firstKeyframeDecodedSample": first_keyframe,
        "latestSample": samples[-1] if samples else None,
        "deltaTotals": {key: round(value, 6) for key, value in totals.items()},
        "counterChange": counter_change(inbound_samples),
    }


def empty_delta_totals() -> dict[str, float]:
    return {
        "packetsReceivedDelta": 0.0,
        "bytesReceivedDelta": 0.0,
        "framesDecodedDelta": 0.0,
        "keyFramesDecodedDelta": 0.0,
        "framesDroppedDelta": 0.0,
        "pliCountDelta": 0.0,
        "firCountDelta": 0.0,
        "nackCountDelta": 0.0,
        "jitterBufferDelayDelta": 0.0,
        "jitterBufferEmittedCountDelta": 0.0,
        "totalDecodeTimeDelta": 0.0,
    }


def counter_change(samples: list[dict[str, Any]]) -> dict[str, int | None]:
    if not samples:
        return {}
    first = samples[0]
    latest = samples[-1]
    keys = (
        "packetsReceived",
        "bytesReceived",
        "framesDecoded",
        "keyFramesDecoded",
        "framesDropped",
        "pliCount",
        "firCount",
        "nackCount",
    )
    changes: dict[str, int | None] = {}
    for key in keys:
        start = first.get(key)
        end = latest.get(key)
        changes[f"{key}Change"] = end - start if isinstance(start, int) and isinstance(end, int) else None
    return changes


def value_from_kind(first_by_kind: dict[str, Any], kind: str, field: str) -> float | None:
    item = first_by_kind.get(kind)
    if not isinstance(item, dict):
        return None
    value = item.get(field)
    return float(value) if isinstance(value, (int, float)) else None


def first_frame_latencies(timeline: dict[str, Any]) -> dict[str, Any]:
    first_by_kind = timeline.get("firstByKind")
    if not isinstance(first_by_kind, dict):
        return {}
    return {
        "connectedToFirstInboundPacketMs": value_from_kind(
            first_by_kind, "firstInboundPacket", "elapsedSinceConnectedMs"
        ),
        "connectedToFirstDecodedMs": value_from_kind(
            first_by_kind, "firstDecoded", "elapsedSinceConnectedMs"
        ),
        "connectedToFirstKeyframeDecodedMs": value_from_kind(
            first_by_kind, "firstKeyframeDecoded", "elapsedSinceConnectedMs"
        ),
        "connectedToFirstPresentedMs": value_from_kind(
            first_by_kind, "firstPresented", "elapsedSinceConnectedMs"
        ),
        "bindToRemoteAnswerSetMs": value_from_kind(
            first_by_kind, "remoteAnswerSet", "elapsedSinceBindMs"
        ),
    }


def missing_browser_evidence(
    sdp: dict[str, Any],
    timeline: dict[str, Any],
    stats: dict[str, Any],
    require_presented: bool,
) -> list[str]:
    failures: list[str] = []
    by_stage = sdp.get("byStage")
    first_by_kind = timeline.get("firstByKind")
    if not isinstance(by_stage, dict):
        by_stage = {}
    if not isinstance(first_by_kind, dict):
        first_by_kind = {}
    if "localOfferAfterPatch" not in by_stage:
        failures.append("missing browser localOfferAfterPatch SDP observation")
    if "remoteAnswer" not in by_stage:
        failures.append("missing browser remoteAnswer SDP observation")
    if "remoteAnswerSet" not in first_by_kind:
        failures.append("missing browser remoteAnswerSet timeline event")
    if timeline.get("connected") is None:
        failures.append("missing browser connectionStateChanged=connected event")
    for kind in ("firstInboundPacket", "firstDecoded", "firstKeyframeDecoded"):
        if kind not in first_by_kind:
            failures.append(f"missing browser {kind} timeline event")
    if require_presented and "firstPresented" not in first_by_kind:
        failures.append("missing browser firstPresented timeline event")
    if stats.get("sampleCount", 0) <= 0:
        failures.append("missing browser WebRTC getStats samples")
    return failures


def build_report(
    trace: Path,
    *,
    max_age_seconds: float | None,
    require_presented: bool,
) -> dict[str, Any]:
    rows = load_events(trace)
    sdp = browser_sdp_observations(rows)
    timeline = browser_timeline(rows)
    stats = browser_stats(rows)
    selected_answer = selected_remote_answer_from_timeline(rows) or selected_remote_answer_from_sdp(sdp)
    latencies = first_frame_latencies(timeline)
    failures = missing_browser_evidence(sdp, timeline, stats, require_presented)
    freshness = trace_freshness_report(trace, max_age_seconds)
    if freshness["freshnessGate"] != "PASS":
        failures.append("trace is stale")
    return {
        "trace": str(trace),
        "browserWebRtcBehaviorGate": "FAIL" if failures else "PASS",
        "failures": failures,
        "freshness": freshness,
        "selectedRemoteAnswer": selected_answer,
        "firstFrameLatencies": latencies,
        "sdp": sdp,
        "timeline": timeline,
        "stats": stats,
    }


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Report browser WebRTC SDP, lifecycle, first-frame, and feedback behavior."
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
        "--allow-missing-presented",
        action="store_true",
        help="do not fail when firstPresented is absent",
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
                        "browserWebRtcBehaviorGate": "FAIL",
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
        max_age_seconds=args.max_age_seconds,
        require_presented=not args.allow_missing_presented,
    )
    print(json.dumps(report, ensure_ascii=False, indent=2))
    return 0 if report["browserWebRtcBehaviorGate"] == "PASS" else 2


if __name__ == "__main__":
    raise SystemExit(main())
