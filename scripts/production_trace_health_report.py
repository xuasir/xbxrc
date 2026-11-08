#!/usr/bin/env python3
"""生产 v1 / 旧 dev 的共同证据模型；只用已记录事实计算覆盖和六段验收。"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import sys
import time
from collections import Counter, defaultdict
from pathlib import Path
from typing import Any

PASS, FAIL, INSUFFICIENT, UNSUPPORTED = "PASS", "FAIL", "INSUFFICIENT_EVIDENCE", "UNSUPPORTED"
STAGES = ("received", "decoded", "videoPresent", "callbacks", "rendererSubmit")
RATES = ("receivedFps", "decodeFps", "videoPresentFps", "callbackFps", "rendererSubmitFps")
CORE = ("received", "decoded", "videoPresent", "rendererSubmit")
EXPECTED_BLOCKS = {"noCandidate", "disabled", "outsideTopScope", "cancelled"}


def number(value: Any) -> float | None:
    return float(value) if isinstance(value, (int, float)) and not isinstance(value, bool) and math.isfinite(value) else None


def obj(value: Any) -> dict[str, Any]:
    return value if isinstance(value, dict) else {}


def owner(row: dict[str, Any]) -> tuple:
    p = obj(row.get("payload"))
    return row.get("traceRunId"), row.get("sessionId"), p.get("runtimeGeneration"), p.get("clientGeneration"), p.get("observationEpoch", 0)


def observed_at(row: dict[str, Any]) -> float:
    p = obj(row.get("payload"))
    return next((value for value in (number(p.get("observedAtMs")), number(p.get("sampledAtMs")), number(row.get("tsMs"))) if value is not None), 0)


def union_length(intervals: list[tuple[float, float]]) -> float:
    merged: list[list[float]] = []
    for start, end in sorted(intervals):
        if end <= start:
            continue
        if merged and start <= merged[-1][1]:
            merged[-1][1] = max(end, merged[-1][1])
        else:
            merged.append([start, end])
    return sum(end - start for start, end in merged)


def status_join(statuses: list[str]) -> str:
    for state in (FAIL, INSUFFICIENT, UNSUPPORTED):
        if state in statuses:
            return state
    return PASS


def normalize(rows: list[dict[str, Any]]) -> tuple[list[dict[str, Any]], int]:
    manifests: dict[tuple, dict[str, Any]] = {}
    dev_samples: dict[tuple, dict[str, Any]] = {}
    dev_telemetry: dict[tuple, dict[str, Any]] = {}
    for row in rows:
        p = obj(row.get("payload"))
        if row.get("event") == "browserWebRtcStatsObserved":
            dev_samples[(*owner(row), p.get("peerGeneration"), p.get("sampleId"))] = p
    normalized, seen_dev, unsupported = [], set(), 0
    production_owners = {owner(row) for row in rows if row.get("event") == "browserPlaybackHealthObserved" and obj(row.get("payload")).get("productionContractVersion") == 1}
    for row in sorted(rows, key=lambda item: (str(owner(item)), observed_at(item), item.get("seq", 0))):
        p, event, key = obj(row.get("payload")), row.get("event"), owner(row)
        if event == "playbackObservationManifest":
            manifests[key] = p
            continue
        if event == "renderTelemetryObserved":
            dev_telemetry[key] = p
            continue
        manifest = manifests.get(key, {})
        config, capabilities = obj(manifest.get("configuration")), obj(manifest.get("capabilities"))
        if event == "browserPlaybackHealthObserved":
            if p.get("productionContractVersion") not in (None, 1):
                unsupported += 1
                continue
            window = obj(p.get("window"))
            start, end = number(window.get("startMs")), number(window.get("endMs"))
            if start is None or end is None or end <= start:
                continue
            coverage, supply = obj(p.get("coverage")), obj(p.get("frameSupply"))
            valid_ms = min(end - start, max(0, number(coverage.get("validMs")) or 0))
            state = p.get("observationState")
            normalized.append({"row": row, "owner": key, "source": "productionBucket", "start": start, "end": end, "elapsed": end - start,
                "validMs": valid_ms, "state": state, "partial": window.get("partial", False), "counterMs": obj(coverage.get("counterMs")),
                "fps": dict(zip(STAGES, (number(supply.get(field)) for field in RATES))), "delta": obj(supply.get("delta")), "totals": obj(supply.get("totals")),
                "network": obj(p.get("network")), "recovery": obj(p.get("recovery")), "renderer": obj(p.get("renderer")), "supply": supply,
                "configuration": config, "capabilities": capabilities, "target": number(p.get("expectedFps")) or number(config.get("expectedFps")),
                "rendererTarget": number(p.get("rendererTargetFps")) or number(config.get("rendererTargetFps")), "rendererKind": p.get("rendererKind") or config.get("rendererKind"),
                "visibility": p.get("visibility") or manifest.get("visibility", "visible"), "connected": p.get("connectionState") == "connected" or p.get("lifecyclePhase") in ("steady", "connected"),
                "healthSequence": p.get("healthSequence"), "sampleGapMs": number(coverage.get("maxSampleGapMs")), "collection": obj(p.get("collection")), "cadence": number(obj(manifest.get("cadence")).get("healthMs")) or 1000,
                "identity": (p.get("peerGeneration"), p.get("videoGeneration"), p.get("rendererGeneration")), "trace": obj(p.get("trace"))})
        elif event == "browserObservationConsumed" and key not in production_owners:
            identity = (*key, p.get("peerGeneration"), p.get("sampleId"))
            if identity in seen_dev:
                continue
            seen_dev.add(identity)
            sample, facts = dev_samples.get(identity, {}), obj(p.get("facts"))
            end = number(sample.get("sampledAtMs")) or number(p.get("sampledAtMs"))
            elapsed = number(obj(sample.get("delta")).get("elapsedMs"))
            if end is None or elapsed is None or elapsed <= 0:
                continue
            inbound, delta = obj(sample.get("inboundVideo")), obj(sample.get("delta"))
            telemetry = dev_telemetry.get(key, {})
            frame_delta = dict(zip(STAGES, (number(delta.get(field)) for field in ("framesReceivedDelta", "framesDecodedDelta", "videoPresentDelta", "callbackDelta", "rendererSubmitDelta"))))
            for stage, rate in zip(STAGES, RATES):
                if frame_delta[stage] is None and number(facts.get(rate)) is not None:
                    frame_delta[stage] = facts[rate] * elapsed / 1000
            for stage, field in {"framesDropped": "framesDroppedDelta", "freezeCount": "freezeCountDelta", "keyframes": "keyFramesDecodedDelta", "pli": "pliCountDelta", "fir": "firCountDelta", "nack": "nackCountDelta"}.items():
                frame_delta[stage] = number(delta.get(field))
            normalized.append({"row": row, "owner": key, "source": "devCollectionInterval", "start": end - elapsed, "end": end, "elapsed": elapsed,
                "validMs": elapsed if p.get("state") == "fresh" and not sample.get("deltaResetReason") else 0,
                "state": p.get("state"), "partial": bool(sample.get("deltaResetReason")), "counterMs": {},
                "fps": dict(zip(STAGES, (number(facts.get(field)) for field in RATES))), "delta": frame_delta,
                "totals": {"received": inbound.get("framesReceived"), "decoded": inbound.get("framesDecoded"), "keyframes": inbound.get("keyFramesDecoded")},
                "network": {"rttMs": facts.get("rttMs"), "jitterMs": facts.get("jitterMs"), "packetLossRatio": facts.get("lossRatio"), "bitrateKbps": facts.get("bitrateKbps")},
                "recovery": {"decodeMs": telemetry.get("decodeMs")}, "renderer": {"timing": telemetry.get("rendererTiming"), "gpuTiming": telemetry.get("rendererGpuTiming"), "timingScope": "retainedSamples", "submitAgeMs": telemetry.get("rendererSubmitAgeMs"), "videoPresentAgeMs": telemetry.get("videoPresentAgeMs")},
                "supply": {"maxCallbackIntervalMs": telemetry.get("maxCallbackIntervalMs"), "windows": facts.get("windows")},
                "configuration": config, "capabilities": capabilities, "target": number(config.get("expectedFps")) or number(telemetry.get("expectedContentFps")),
                "rendererTarget": number(telemetry.get("rendererTargetFps")), "rendererKind": facts.get("rendererKind"), "visibility": manifest.get("visibility", "visible"),
                "connected": sample.get("connectionState") == "connected", "healthSequence": None, "sampleGapMs": elapsed, "cadence": 250,
                "identity": (p.get("peerGeneration"), p.get("videoGeneration"), p.get("rendererGeneration")), "trace": obj(telemetry.get("trace"))})
    return normalized, unsupported


def partition(samples: list[dict[str, Any]]) -> list[list[dict[str, Any]]]:
    groups: dict[tuple, list[dict[str, Any]]] = defaultdict(list)
    for sample in samples:
        groups[sample["owner"]].append(sample)
    result = []
    for group in groups.values():
        current, previous_stamp = [], None
        for sample in sorted(group, key=lambda item: item["end"]):
            stamp = (sample["identity"], sample["rendererKind"], sample["target"], sample["rendererTarget"], sample["visibility"], json.dumps(sample["configuration"], sort_keys=True))
            if current and stamp != previous_stamp:
                result.append(current)
                current = []
            current.append(sample)
            previous_stamp = stamp
        if current:
            result.append(current)
    return result


def rates_and_coverage(samples: list[dict[str, Any]], end_at: float | None = None) -> dict[str, Any]:
    start, end = min(item["start"] for item in samples), max(item["end"] for item in samples)
    end = max(end, end_at or end)
    duration = end - start
    published = union_length([(max(item["start"], item["end"] - item["cadence"]), item["end"]) for item in samples])
    valid_ms = sum(item["validMs"] if all(item["fps"][stage] is not None for stage in CORE) else 0 for item in samples)
    counter_ms, totals = Counter(), Counter()
    for sample in samples:
        for stage in STAGES:
            if sample["fps"][stage] is not None:
                counter_ms[stage] += min(sample["validMs"], max(0, number(sample["counterMs"].get(stage)) if number(sample["counterMs"].get(stage)) is not None else sample["validMs"]))
        for key, value in sample["delta"].items():
            if number(value) is not None:
                totals[key] += value
    core_coverage = min((counter_ms[stage] for stage in CORE), default=0)
    valid_ms = min(valid_ms, core_coverage)
    fps = {}
    for stage in STAGES:
        intervals = [item for item in samples if item["validMs"] > 0 and item["fps"][stage] is not None]
        elapsed = sum(item["elapsed"] for item in intervals)
        fps[stage] = round(sum(item["fps"][stage] * item["elapsed"] for item in intervals) / elapsed, 3) if elapsed else None
    gaps = [max(0, right["end"] - left["end"] - right["cadence"]) for left, right in zip(samples, samples[1:])]
    gaps.extend((max(0, samples[0]["end"] - start - samples[0]["cadence"]), max(0, end - samples[-1]["end"])))
    sequence_gaps = sum(max(0, right["healthSequence"] - left["healthSequence"] - 1) for left, right in zip(samples, samples[1:]) if isinstance(left["healthSequence"], int) and isinstance(right["healthSequence"], int))
    return {"startMs": start, "endMs": end, "durationMs": duration, "samples": len(samples), "publishedMs": published,
        "publicationRatio": round(min(1, published / duration), 4) if duration else 0, "mediaValidMs": valid_ms,
        "mediaValidRatio": round(min(1, valid_ms / duration), 4) if duration else 0, "longestPublicationGapMs": max(gaps, default=0),
        "maxSampleGapMs": max((item["sampleGapMs"] or 0 for item in samples), default=0), "missingHealthSequences": sequence_gaps,
        "observationStates": dict(Counter(item["state"] for item in samples)), "collectionStates": dict(Counter(obj(item.get("collection")).get("state", "notObserved") for item in samples)),
        "maxCollectionAgeMs": max((number(obj(item.get("collection")).get("ageMs")) or 0 for item in samples), default=0),
        "counterCoverageMs": dict(counter_ms), "fps": fps, "deltaTotals": {key: round(value, 3) for key, value in totals.items()}}


def sustained(samples: list[dict[str, Any]], predicate, minimum_ms: float = 3000) -> list[dict[str, Any]]:
    intervals, active, elapsed = [], [], 0.0
    for item in samples:
        qualified = item["validMs"] >= item["elapsed"] * 0.9 and not item["partial"] and predicate(item)
        if qualified and (not active or item["start"] - active[-1]["end"] <= 1):
            active.append(item)
            elapsed += item["elapsed"]
        else:
            if elapsed >= minimum_ms:
                intervals.append({"startMs": active[0]["start"], "endMs": active[-1]["end"], "durationMs": elapsed, "firstSeq": active[0]["row"].get("seq"), "samples": len(active)})
            active, elapsed = ([item], item["elapsed"]) if qualified else ([], 0)
    if elapsed >= minimum_ms:
        intervals.append({"startMs": active[0]["start"], "endMs": active[-1]["end"], "durationMs": elapsed, "firstSeq": active[0]["row"].get("seq"), "samples": len(active)})
    return intervals


def stage_target(sample: dict[str, Any], renderer=False) -> float | None:
    source = number(sample["configuration"].get("sourceFpsEstimate")) or sample["target"] or number(sample["configuration"].get("requestedFps"))
    if source is None:
        return None
    return min(source, sample["rendererTarget"] or source) if renderer else source


def distribution_report(samples: list[dict[str, Any]], gpu: bool, field: str = "duration") -> dict[str, Any]:
    windows = []
    for sample in samples:
        renderer = sample["renderer"]
        timing = obj(renderer.get("gpuTiming")) if gpu else obj(renderer.get("timing"))
        distribution = obj(timing.get(field))
        count = number(distribution.get("retained")) if number(distribution.get("retained")) is not None else number(distribution.get("count"))
        if count and number(distribution.get("p95Ms")) is not None and (not gpu or timing.get("state") == "ready"):
            windows.append({"seq": sample["row"].get("seq"), "count": count, "scope": renderer.get("timingScope", "retainedSamples"),
                **{key: number(distribution.get(key)) for key in ("p95Ms", "p99Ms", "maxMs")}})
    if not windows:
        return {"status": UNSUPPORTED if gpu and all(obj(item["renderer"].get("gpuTiming")).get("state") in (None, "unavailable") for item in samples) else INSUFFICIENT, "windows": 0}
    return {"status": PASS, "windows": len(windows), "latestCount": windows[-1]["count"], "scope": windows[-1]["scope"],
        "maxReportedP95Ms": max(item["p95Ms"] for item in windows), "maxReportedP99Ms": max((item["p99Ms"] for item in windows if item["p99Ms"] is not None), default=None),
        "maxMs": max((item["maxMs"] for item in windows if item["maxMs"] is not None), default=None), "latest": windows[-1]}


def six_sections(samples: list[dict[str, Any]], coverage: dict[str, Any]) -> tuple[dict[str, Any], list[dict[str, Any]]]:
    evidence = {"samples": len(samples), "validMs": coverage["mediaValidMs"], "window": {"startMs": coverage["startMs"], "endMs": coverage["endMs"]}, "sources": sorted({item["source"] for item in samples})}
    diagnoses: list[dict[str, Any]] = []
    def diagnosis(name, predicate, minimum_ms=3000):
        windows = sustained(samples, predicate, minimum_ms)
        if windows:
            diagnoses.append({"classification": name, "windows": windows})
        return windows
    def lower(sample, stage, renderer=False):
        target, rate = stage_target(sample, renderer), sample["fps"][stage]
        return target is not None and target > 0 and rate is not None and rate < target * 0.9
    network_bad = diagnosis("networkDegraded", lambda item: (number(item["network"].get("packetLossRatio")) or 0) > 0.02 or (number(item["network"].get("jitterMs")) or 0) > 30)
    supply_bad = diagnosis("sourceSupplyDeficit", lambda item: lower(item, "received"))
    decode_bad = diagnosis("decodeLimited", lambda item: lower(item, "decoded") and (item["fps"]["received"] or 0) > (item["fps"]["decoded"] or 0) * 1.05)
    present_bad = diagnosis("videoPresentDeficit", lambda item: lower(item, "videoPresent"))
    renderer_bad = diagnosis("rendererSupplyDeficit", lambda item: lower(item, "rendererSubmit", True))
    callback_sparse = diagnosis("callbackSparseBatching", lambda item: lower(item, "callbacks") and not lower(item, "videoPresent"))
    def timing_slow(sample, group, gpu=False):
        target = stage_target(sample, True)
        renderer = sample["renderer"]
        timing = obj(renderer.get("gpuTiming")) if gpu else obj(renderer.get("timing"))
        value = obj(timing.get(group))
        count = number(value.get("retained")) if number(value.get("retained")) is not None else number(value.get("count"))
        return bool(target and count and number(value.get("p95Ms")) is not None and value["p95Ms"] > 1000 / target and (not gpu or timing.get("state") == "ready"))
    upload_bad = diagnosis("uploadLimited", lambda item: timing_slow(item, "upload"))
    gpu_bad = diagnosis("gpuLimited", lambda item: timing_slow(item, "duration", True))
    draw_bad = diagnosis("cpuDrawLimited", lambda item: timing_slow(item, "duration"))
    submit_delayed = diagnosis("submitPresentDelayed", lambda item: max(number(item["renderer"].get("maxSubmitAgeMs")) or number(item["renderer"].get("submitAgeMs")) or 0, number(item["renderer"].get("maxVideoPresentAgeMs")) or number(item["renderer"].get("videoPresentAgeMs")) or 0) > 100)
    callback_gaps = diagnosis("callbackGap", lambda item: (number(item["supply"].get("maxCallbackIntervalMs")) or 0) > 100)
    recovery_stall = diagnosis("recoveryNoProgress", lambda item: (item["fps"]["decoded"] == 0 or item["fps"]["videoPresent"] == 0) and (item["fps"]["received"] or 0) > 0)
    fps = coverage["fps"]
    difference = lambda a, b: round(fps[a] - fps[b], 3) if fps[a] is not None and fps[b] is not None else None
    delta = coverage["deltaTotals"]
    network_samples = [item for item in samples if all(number(item["network"].get(field)) is not None for field in ("rttMs", "jitterMs", "packetLossRatio"))]
    renderer_observed = any(any(number(item["renderer"].get(field)) is not None for field in ("maxSubmitAgeMs", "maxVideoPresentAgeMs", "submitAgeMs", "videoPresentAgeMs")) or number(obj(obj(item["renderer"].get("timing")).get("duration")).get("p95Ms")) is not None for item in samples)
    targets = sorted({stage_target(item) for item in samples if stage_target(item) is not None})
    renderer_targets = sorted({stage_target(item, True) for item in samples if stage_target(item, True) is not None})
    sections = {
        "networkHealth": {**evidence, "status": FAIL if network_bad else PASS if network_samples else INSUFFICIENT,
            "measuredSamples": len(network_samples), "maxRttMs": max((number(item["network"].get("rttMs")) or 0 for item in network_samples), default=None),
            "maxJitterMs": max((number(item["network"].get("jitterMs")) or 0 for item in network_samples), default=None), "maxLossRatio": max((number(item["network"].get("packetLossRatio")) or 0 for item in network_samples), default=None)},
        "recoveryChainHealth": {**evidence, "status": FAIL if recovery_stall else PASS if fps["decoded"] is not None and fps["videoPresent"] is not None else INSUFFICIENT,
            "capability": "browserPublicStats", "nativeReferenceChainGate": UNSUPPORTED,
            "observedDeltas": {key: delta.get(key) for key in ("keyframes", "pli", "fir", "nack", "freezeCount", "framesDropped")}},
        "mediaSupply": {**evidence, "status": FAIL if supply_bad or decode_bad else PASS if targets and fps["received"] is not None and fps["decoded"] is not None else INSUFFICIENT,
            "sourceTargetsFps": targets, "receivedFps": fps["received"], "decodeFps": fps["decoded"], "receiveDecodeDeltaFps": difference("received", "decoded")},
        "steadySupply": {**evidence, "status": FAIL if present_bad else PASS if targets and fps["videoPresent"] is not None else INSUFFICIENT,
            "videoPresentFps": fps["videoPresent"], "callbackFps": fps["callbacks"], "callbackBatchingObserved": bool(callback_sparse), "callbackGapObserved": bool(callback_gaps),
            "maxCallbackIntervalMs": max((number(item["supply"].get("maxCallbackIntervalMs")) or 0 for item in samples), default=0), "physicalScanout": UNSUPPORTED},
        "submitPresentTail": {**evidence, "status": FAIL if upload_bad or gpu_bad or draw_bad or submit_delayed else PASS if renderer_observed else INSUFFICIENT,
            "cpuDraw": distribution_report(samples, False), "upload": distribution_report(samples, False, "upload"), "cpuSubmit": distribution_report(samples, False, "submit"), "gpu": distribution_report(samples, True)},
        "frameSupplyDelta": {**evidence, "status": FAIL if decode_bad or present_bad or renderer_bad else PASS if targets and all(fps[key] is not None for key in CORE) else INSUFFICIENT,
            "fps": fps, "deltaTotals": delta, "effectiveRendererTargetsFps": renderer_targets,
            "differencesFps": {"receiveDecode": difference("received", "decoded"), "decodeVideoPresent": difference("decoded", "videoPresent"), "videoPresentCallback": difference("videoPresent", "callbacks"), "callbackRendererSubmit": difference("callbacks", "rendererSubmit")},
            "intentionalRateLimitObserved": any(item["rendererTarget"] and stage_target(item) and item["rendererTarget"] < stage_target(item) for item in samples)},
    }
    return sections, diagnoses


def input_report(rows: list[dict[str, Any]]) -> dict[str, Any]:
    interactions = [row for row in rows if row.get("event") == "inputUiInteractionObserved"]
    health = [row for row in rows if row.get("event") == "inputUiHealthObserved"]
    failures, expected_blocks, operations = [], [], defaultdict(list)
    for row in interactions:
        p = obj(row.get("payload"))
        safe = {key: p.get(key) for key in ("operationId", "source", "intent", "scope", "action", "outcome", "reason", "durationMs")}
        safe["seq"] = row.get("seq")
        if p.get("outcome") in ("failed", "blocked"):
            (expected_blocks if p.get("reason") in EXPECTED_BLOCKS else failures).append(safe)
        if p.get("operationId"):
            operations[(*owner(row)[:4], p["operationId"])].append(safe)
    incomplete, paired = [], []
    for items in operations.values():
        starts = [item for item in items if item["outcome"] == "operationStarted"]
        ends = [item for item in items if item["outcome"] in ("operationCompleted", "cancelled", "failed", "blocked")]
        if starts and ends:
            paired.append({"operationId": starts[0]["operationId"], "source": starts[0]["source"], "action": starts[0]["action"], "startSeq": starts[0]["seq"], "resultSeq": ends[-1]["seq"], "outcome": ends[-1]["outcome"], "durationMs": ends[-1]["durationMs"]})
        elif starts:
            incomplete.append(starts[0])
    return {"status": FAIL if failures else INSUFFICIENT if incomplete else PASS if interactions or health else UNSUPPORTED,
        "interactionRows": len(interactions), "healthRows": len(health), "failures": failures, "expectedBlocks": expected_blocks,
        "pairedOperations": paired, "incompleteOperations": incomplete, "outcomes": dict(Counter(obj(row.get("payload")).get("outcome") for row in interactions))}


def ending_report(rows: list[dict[str, Any]]) -> dict[str, Any]:
    summaries = [row for row in rows if row.get("event") == "playbackSessionSummary" and obj(row.get("payload")).get("ended") is True]
    cleanups = [row for row in rows if row.get("event") == "executionCleanupObserved"]
    checkpoints = [row for row in rows if row.get("event") == "traceFlushObserved" and obj(row.get("payload")).get("status") == "flushed"]
    exports = [obj(row.get("payload")) for row in rows if row.get("event") == "traceExportManifest"]
    sessions = {(row.get("traceRunId"), row.get("sessionId")) for row in rows if row.get("event") in ("playbackSessionSummary", "browserPlaybackHealthObserved", "playbackObservationManifest") and row.get("sessionId") is not None}
    results = []
    for run, session in sorted(sessions, key=str):
        matching = [row for row in summaries if row.get("sessionId") == session and row.get("traceRunId") == run]
        if not matching:
            results.append({"sessionId": session, "status": INSUFFICIENT, "summarySeq": None, "cleanupSeq": None, "cleanupState": None, "flushed": False})
            continue
        summary = matching[-1]
        cleanup = next((row for row in reversed(cleanups) if row.get("sessionId") == session and row.get("traceRunId") == summary.get("traceRunId")), None)
        sequence = number(cleanup.get("seq")) if cleanup else None
        flushed = any(row.get("traceRunId") == summary.get("traceRunId") and sequence is not None and (number(obj(row.get("payload")).get("flushedSequence")) or 0) >= sequence for row in checkpoints)
        flushed |= any(export.get("traceRunId") == summary.get("traceRunId") and export.get("flushStatus") == "flushed" and sequence is not None and (number(export.get("cutoffSequence")) or 0) >= sequence for export in exports)
        cleanup_state = obj(cleanup.get("payload")).get("cleanupState") if cleanup else None
        results.append({"sessionId": session, "status": FAIL if cleanup_state == "pending" else PASS if cleanup_state == "completed" and flushed else INSUFFICIENT,
            "summarySeq": summary.get("seq"), "cleanupSeq": sequence, "cleanupState": cleanup_state, "flushed": flushed, "reason": obj(summary.get("payload")).get("reason")})
    return {"status": status_join([item["status"] for item in results]) if results else INSUFFICIENT, "partialEnding": not results or any(item["status"] != PASS for item in results), "sessions": results}


def trace_loss_report(rows: list[dict[str, Any]]) -> dict[str, Any]:
    producer, backend = Counter(), Counter()
    for row in rows:
        p = obj(row.get("payload"))
        trace = obj(p.get("trace"))
        for key in ("dropped", "droppedEssential", "ipcFailed", "ipcTimedOut", "producerRowOversized", "payloadFailed", "ackMissing", "coalesced", "profileFiltered", "queueFull", "rowOversized", "writerUnavailable"):
            producer[key] = max(producer[key], number(trace.get(key)) or 0)
        state = obj(trace.get("backend")) or obj(p.get("backend"))
        for key in ("essentialDropped", "keyDropped", "debugDropped", "rawDropped", "ioDropped", "ioFailures", "oversizedDropped"):
            backend[key] = max(backend[key], number(state.get(key)) or 0)
        if row.get("event") == "traceBudgetNotice":
            for key in ("essentialDropped", "keyDropped", "ioDropped", "oversizedDropped"):
                backend[key] = max(backend[key], number(p.get(key)) or 0)
    return {"status": INSUFFICIENT if producer["droppedEssential"] or backend["essentialDropped"] or backend["keyDropped"] or backend["ioDropped"] else PASS,
        "producerMaxCounters": dict(producer), "backendMaxCounters": dict(backend), "counterScope": "maxObservedCumulativeWithinRun"}


def export_report(rows: list[dict[str, Any]]) -> dict[str, Any]:
    manifests = [obj(row.get("payload")) for row in rows if row.get("event") == "traceExportManifest"]
    return {"status": INSUFFICIENT if any((number(item.get("missingFiles")) or 0) > 0 or (number(item.get("truncatedRows")) or 0) > 0 for item in manifests) else PASS,
        "bundles": [{key: item.get(key) for key in ("traceRunId", "cutoffSequence", "flushStatus", "partialBeginning", "missingFiles", "truncatedRows", "files")} for item in manifests]}


def build_health_report(rows: list[dict[str, Any]], *, min_steady_seconds: float = 60, session_id: str | None = None) -> dict[str, Any]:
    if session_id is not None:
        rows = [row for row in rows if row.get("sessionId") in (session_id, None)]
    samples, unsupported = normalize(rows)
    groups = partition(samples)
    summaries = {owner(row): observed_at(row) for row in rows if row.get("event") == "playbackSessionSummary" and obj(row.get("payload")).get("ended") is True}
    segments = []
    last_by_owner = {group[-1]["owner"]: group for group in groups}
    for group in groups:
        visible = [item for item in group if item["connected"] and item["visibility"] != "hidden"]
        if not visible:
            continue
        end_at = summaries.get(group[-1]["owner"]) if last_by_owner.get(group[-1]["owner"]) is group else None
        coverage = rates_and_coverage(visible, end_at)
        sections, diagnoses = six_sections(visible, coverage)
        coverage_good = coverage["publicationRatio"] >= 0.95 and coverage["longestPublicationGapMs"] <= 3000 and coverage["mediaValidRatio"] >= 0.9
        gate = status_join([value["status"] for value in sections.values()]) if coverage_good and coverage["durationMs"] >= min_steady_seconds * 1000 else INSUFFICIENT
        segments.append({"sessionId": group[0]["owner"][1], "runtimeGeneration": group[0]["owner"][2], "clientGeneration": group[0]["owner"][3], "observationEpoch": group[0]["owner"][4],
            "identity": group[0]["identity"], "configuration": group[0]["configuration"], "gate": gate, "coverage": coverage, "sections": sections, "diagnoses": diagnoses})
    loss, inputs, ending, exported = trace_loss_report(rows), input_report(rows), ending_report(rows), export_report(rows)
    total_steady_ms = sum(segment["coverage"]["durationMs"] for segment in segments)
    segment_statuses = []
    for segment in segments:
        c = segment["coverage"]
        segment_statuses.append(status_join([value["status"] for value in segment["sections"].values()]) if c["publicationRatio"] >= 0.95 and c["longestPublicationGapMs"] <= 3000 and c["mediaValidRatio"] >= 0.9 else INSUFFICIENT)
    gate = status_join(segment_statuses) if total_steady_ms >= min_steady_seconds * 1000 else UNSUPPORTED if unsupported and not samples else INSUFFICIENT
    if (loss["status"] != PASS or exported["status"] != PASS) and gate != UNSUPPORTED:
        gate = INSUFFICIENT
    if inputs["status"] == FAIL and gate == PASS:
        gate = FAIL
    anomalies = [{"seq": row.get("seq"), **{key: obj(row.get("payload")).get(key) for key in ("episodeId", "phase", "reason", "reasons", "durationMs", "sampleCount", "coverage")}} for row in rows if row.get("event") == "browserPlaybackAnomalyObserved"]
    names = ("networkHealth", "recoveryChainHealth", "mediaSupply", "steadySupply", "submitPresentTail", "frameSupplyDelta")
    section_rollup = {name: {"status": status_join([segment["sections"][name]["status"] for segment in segments]) if segments else UNSUPPORTED if unsupported else INSUFFICIENT,
        "segments": len(segments), "samples": sum(segment["sections"][name]["samples"] for segment in segments), "validMs": sum(segment["sections"][name]["validMs"] for segment in segments)} for name in names}
    return {"productionContractVersion": 1, "gate": gate, "acceptanceGate": status_join([gate, ending["status"]]), "sections": section_rollup, "steadyDurationMs": total_steady_ms,
        "thresholds": {"minSteadySeconds": min_steady_seconds, "publicationRatio": 0.95, "maxPublicationGapMs": 3000, "mediaValidRatio": 0.9, "supplyRatio": 0.9, "sustainedMs": 3000},
        "rowCount": len(rows), "healthRecords": sum(item["source"] == "productionBucket" for item in samples), "devIntervals": sum(item["source"] == "devCollectionInterval" for item in samples),
        "unsupportedContractRecords": unsupported, "segments": segments, "input": inputs, "traceLoss": loss, "export": exported, "ending": ending, "anomalies": anomalies,
        "capabilities": {"browserPhysicalScanout": UNSUPPORTED, "browserNativeReferenceChain": UNSUPPORTED}}


def browser_stats_projection(rows: list[dict[str, Any]]) -> dict[str, Any]:
    samples, _ = normalize(rows)
    samples = [item for item in samples if item["source"] == "productionBucket"]
    if not samples:
        return {}
    samples.sort(key=lambda item: item["end"])
    mapping = {"packetsReceivedDelta": "packetsReceived", "bytesReceivedDelta": "bytesReceived", "framesDecodedDelta": "decoded", "keyFramesDecodedDelta": "keyframes", "framesDroppedDelta": "framesDropped", "pliCountDelta": "pli", "firCountDelta": "fir", "nackCountDelta": "nack"}
    totals = {key: sum(number(item["delta"].get(source)) or 0 for item in samples) for key, source in mapping.items()}
    def compact(sample):
        return {"seq": sample["row"].get("seq"), "sampledAtMs": sample["end"], "framesDecoded": sample["totals"].get("decoded"), "keyFramesDecoded": sample["totals"].get("keyframes"), "frameWidth": sample["supply"].get("sourceWidth"), "frameHeight": sample["supply"].get("sourceHeight"), "delta": {key: sample["delta"].get(source) for key, source in mapping.items()}}
    return {"source": "productionBucket", "sampleCount": len(samples), "inboundSampleCount": sum(item["fps"]["received"] is not None for item in samples), "firstSample": compact(samples[0]), "latestSample": compact(samples[-1]), "deltaTotals": totals}


def read_trace(paths: list[Path]) -> tuple[list[dict[str, Any]], dict[str, Any]]:
    rows, seen, malformed, duplicate, files = [], set(), 0, 0, []
    for path in paths:
        data = path.read_bytes()
        files.append({"fileName": path.name, "bytes": len(data), "sha256": hashlib.sha256(data).hexdigest()})
        for line in data.splitlines():
            if not line.strip():
                continue
            try:
                row = json.loads(line)
            except (ValueError, UnicodeDecodeError):
                malformed += 1
                continue
            if not isinstance(row, dict):
                malformed += 1
                continue
            key = (row.get("traceRunId"), row.get("seq"))
            if key[1] is not None:
                if key in seen:
                    duplicate += 1
                    continue
                seen.add(key)
            rows.append(row)
    return rows, {"files": files, "malformedRows": malformed, "duplicateRows": duplicate}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("trace", nargs="*", type=Path)
    parser.add_argument("--latest", action="store_true")
    parser.add_argument("--runtime-log-dir", type=Path, default=Path("runtime-logs"))
    parser.add_argument("--session-id")
    parser.add_argument("--min-steady-seconds", type=float, default=60)
    parser.add_argument("--max-age-seconds", type=float)
    parser.add_argument("--fail-on-gate", action="store_true")
    parser.add_argument("--require-clean-ending", action="store_true")
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    paths = args.trace
    if not paths and args.latest:
        paths = sorted(args.runtime_log_dir.glob("runtime-trace-*.jsonl"), key=lambda path: path.stat().st_mtime, reverse=True)[:1]
    if not paths:
        parser.error("请提供 trace 路径，或使用 --latest")
    rows, loading = read_trace(paths)
    report = build_health_report(rows, min_steady_seconds=args.min_steady_seconds, session_id=args.session_id)
    report["loading"] = loading
    age = max(0, time.time() - max(path.stat().st_mtime for path in paths))
    report["freshness"] = {"ageSeconds": round(age, 3), "status": FAIL if args.max_age_seconds is not None and age > args.max_age_seconds else PASS}
    if loading["malformedRows"] and report["gate"] == PASS:
        report["gate"] = INSUFFICIENT
    report["acceptanceGate"] = status_join([report["gate"], report["ending"]["status"], report["freshness"]["status"]])
    gate = report["acceptanceGate"] if args.require_clean_ending else report["gate"]
    if report["freshness"]["status"] == FAIL:
        gate = FAIL
    encoded = json.dumps(report, ensure_ascii=False, indent=2) + "\n"
    if args.output:
        args.output.write_text(encoded, encoding="utf-8")
    else:
        sys.stdout.write(encoded)
    return 2 if args.fail_on_gate and gate != PASS else 0


if __name__ == "__main__":
    raise SystemExit(main())
