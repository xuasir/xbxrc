#!/usr/bin/env python3
"""按原始时钟、peer 代次和可见性窗口复验显示帧供给。"""

import argparse
import collections
import csv
import hashlib
import json
import math
from pathlib import Path


def numeric(value):
    return isinstance(value, (int, float)) and not isinstance(value, bool) and math.isfinite(value)


def distribution(values):
    values = sorted(value for value in values if numeric(value))
    if not values:
        return {"n": 0}
    return {
        "n": len(values),
        "min": values[0],
        "mean": sum(values) / len(values),
        **{name: values[math.ceil(len(values) * q) - 1] for name, q in [("p50", .5), ("p95", .95), ("p99", .99)]},
        "max": values[-1],
    }


def clock(row):
    return row["payload"].get("sourceMonotonicMs", row["tsMs"])


def frame_key(row):
    payload = row["payload"]
    return tuple(payload.get(key) for key in ["attemptId", "generation", "peerEpoch", "frameId"])


def snapshot_key(row):
    payload = row["payload"]
    return tuple(payload.get(key) for key in ["attemptId", "generation", "peerEpoch", "sampleId"])


def foreground_intervals(rows):
    phases = [row for row in rows if row["event"] == "scenePhaseChanged"]
    active = not phases or phases[0]["payload"].get("from") == "active"
    start = min(clock(row) for row in rows)
    intervals = []
    for row in phases:
        now = clock(row)
        next_active = row["payload"].get("to") == "active"
        if active and not next_active:
            intervals.append((start, now))
        elif not active and next_active:
            start = now
        active = next_active
    if active:
        intervals.append((start, max(clock(row) for row in rows)))
    return intervals


def summarize_windows(windows):
    width = sum(window["sampleWindowMs"] for window in windows)
    count_fields = ["framesReceivedDelta", "framesDecodedDelta", "submitCountDelta", "presentCountDelta", "drawablePresentCountDelta"]
    counts = {key: sum(window.get(key, 0) for window in windows) for key in count_fields}
    rates = {key: value * 1000 / width for key, value in counts.items()} if width else {}
    decoded = counts["framesDecodedDelta"]
    return {
        "windows": len(windows),
        "windowSeconds": width / 1000,
        "counts": counts,
        "weightedFps": rates,
        "decodeToGpuGapPercent": 100 * (decoded - counts["presentCountDelta"]) / decoded if decoded else None,
        "decodeToDrawableGapPercent": 100 * (decoded - counts["drawablePresentCountDelta"]) / decoded if decoded else None,
    }


def analyze_attempt(rows, launch, intervals, index):
    grouped = collections.defaultdict(list)
    for row in rows:
        grouped[row["event"]].append(row)
    presents = grouped["displayPresent"]
    first_present = {}
    for row in presents:
        payload = row["payload"]
        epoch = payload.get("peerEpoch")
        at = payload["presentedAtMonotonicMs"]
        first_present[epoch] = min(first_present.get(epoch, at), at)
    first_frame_at = min(first_present.values()) if first_present else clock(launch)
    submissions = {frame_key(row): row for row in grouped["displaySubmit"]}
    completions = {frame_key(row): row for row in presents}
    accepted = {frame_key(row): row for row in grouped["videoFrameAcceptedToMailbox"]}
    metrics = {}
    fields_by_event = {
        "videoFrameAcceptedToMailbox": ["ingressQueueMs", "frameAcceptWorkMs"],
        "displaySubmit": ["drawToSubmitMs", "drawableAcquireMs"],
        "displayPresent": ["gpuToSlotReleaseMs", "gpuToMainActorMs", "frameFeedbackWorkMs", "displayTelemetryWorkMs", "mainActorWorkBeforeTraceMs", "gpuToTraceRecordMs"],
        "rtcHealthSnapshot": ["rttMs", "jitterMs", "receiveBitrateBps"],
        "frameFeedback": ["rawDecodeMs", "rawDecodedToRenderMs", "wireDecodeMs", "wireDecodedToRenderMs", "pendingFrameCount"],
        "displayHealthSnapshot": ["submitToPresentP50Ms", "submitToPresentP95Ms", "submitToPresentP99Ms", "submitToDrawablePresentP50Ms", "submitToDrawablePresentP95Ms", "submitToDrawablePresentP99Ms"],
    }
    for event, fields in fields_by_event.items():
        for field in fields:
            metrics[event + "." + field] = distribution(row["payload"].get(field) for row in grouped[event])
    metrics["submitToGpuCompletionMs"] = distribution(
        row["payload"]["presentedAtMonotonicMs"] - submissions[frame_key(row)]["payload"]["submittedAtMonotonicMs"]
        for row in presents if frame_key(row) in submissions
    )
    metrics["writerQueueDelayMs"] = distribution(row["payload"].get("writerQueueDelayMs") for row in rows)
    displays = {snapshot_key(row): row for row in grouped["displayHealthSnapshot"]}
    windows = []
    for row in grouped["rtcHealthSnapshot"]:
        payload = row["payload"]
        display = displays.get(snapshot_key(row))
        width = payload.get("sampleWindowMs")
        start, end = payload.get("windowStartMonotonicMs"), payload.get("windowEndMonotonicMs")
        if display is None or not numeric(width) or width <= 0 or not numeric(start) or not numeric(end) or payload.get("counterReset"):
            continue
        display_payload = display["payload"]
        display_width = display_payload.get("sampleWindowMs")
        if not numeric(display_width) or display_width <= 0 or display_payload.get("counterReset"):
            continue
        # 两侧采用同一 sampleId；各自的真实采样边界仍要核对。
        assert abs(width - display_width) < .001, (row["seq"], "display sample width differs")
        assert abs(start - display_payload["windowStartMonotonicMs"]) < .001
        assert abs(end - display_payload["windowEndMonotonicMs"]) < .001
        epoch = payload.get("peerEpoch")
        foreground = any(a <= start <= end <= b for a, b in intervals)
        after_first = epoch in first_present and start >= first_present[epoch]
        window = {
            "attempt": index,
            "peerEpoch": epoch,
            "rtcSeq": row["seq"],
            "displaySeq": display["seq"],
            "sampleId": payload["sampleId"],
            "sampleWindowMs": width,
            "windowStartMonotonicMs": start,
            "windowEndMonotonicMs": end,
            "startSecondsAfterFirstGpu": (start - first_frame_at) / 1000,
            "endSecondsAfterFirstGpu": (end - first_frame_at) / 1000,
            "fullyForeground": foreground,
            "afterFirstPresent": after_first,
            "rttMs": payload.get("rttMs"),
            "jitterMs": payload.get("jitterMs"),
            "packetsLostDelta": payload.get("packetsLostDelta"),
            "freezeCount": payload.get("freezeCount"),
            "freezeDurationMs": payload.get("freezeDurationMs"),
            "ingressDropCumulative": display_payload.get("dropCountByReason", {}).get("ingressCoalesced", 0),
            "mailboxOverwriteCumulative": display_payload.get("mailboxOverwriteCount", 0),
            "submitToDrawablePresentP95Ms": display_payload.get("submitToDrawablePresentP95Ms"),
            "submitToDrawablePresentP99Ms": display_payload.get("submitToDrawablePresentP99Ms"),
        }
        for key in ["framesReceivedDelta", "framesDecodedDelta"]:
            window[key] = payload.get(key, 0)
            window[key + "Fps"] = window[key] * 1000 / width
        for key in ["submitCountDelta", "presentCountDelta", "drawablePresentCountDelta"]:
            window[key] = display_payload.get(key, 0)
            window[key + "Fps"] = window[key] * 1000 / width
        window["normalSupply"] = foreground and after_first and window["framesDecodedDeltaFps"] >= 50
        windows.append(window)
    steady = [window for window in windows if window["fullyForeground"] and window["afterFirstPresent"]]
    normal = [window for window in windows if window["normalSupply"]]
    drops = collections.Counter()
    for row in grouped["videoFrameDropped"]:
        drops[row["payload"]["reason"]] += row["payload"].get("count", 1)
    ordered_presents = sorted(presents, key=lambda row: row["payload"]["presentedAtMonotonicMs"])
    gaps = []
    for previous, current in zip(ordered_presents, ordered_presents[1:]):
        start = previous["payload"]["presentedAtMonotonicMs"]
        end = current["payload"]["presentedAtMonotonicMs"]
        if end - start < 100:
            continue
        same_window = [window for window in steady if window["windowStartMonotonicMs"] < end and window["windowEndMonotonicMs"] > start]
        gap_drops = [row for row in grouped["videoFrameDropped"] if start <= clock(row) <= end + 100]
        gaps.append({
            "durationMs": end - start,
            "fromSeq": previous["seq"],
            "toSeq": current["seq"],
            "fromFrameId": previous["payload"]["frameId"],
            "toFrameId": current["payload"]["frameId"],
            "startSecondsAfterFirstGpu": (start - first_frame_at) / 1000,
            "endSecondsAfterFirstGpu": (end - first_frame_at) / 1000,
            "previousGpuToMainActorMs": previous["payload"].get("gpuToMainActorMs"),
            "dropFramesDuringGapAnd100MsTail": sum(row["payload"].get("count", 1) for row in gap_drops),
            "overlappingWindows": same_window,
        })
    ten_second_groups = []
    for bucket in sorted({math.floor(window["startSecondsAfterFirstGpu"] / 10) for window in steady}):
        selected = [window for window in steady if bucket * 10 <= window["startSecondsAfterFirstGpu"] < (bucket + 1) * 10]
        ten_second_groups.append({"startSecondsAfterFirstGpu": bucket * 10, **summarize_windows(selected)})
    last_rtc = grouped["rtcHealthSnapshot"][-1] if grouped["rtcHealthSnapshot"] else None
    terminal = grouped["terminalSelected"][-1] if grouped["terminalSelected"] else None
    overwrite_events = len(grouped["mailboxOverwrite"])
    report = {
        "attempt": index,
        "attemptId": launch["payload"].get("attemptId"),
        "launchSeq": launch["seq"],
        "target": launch["payload"].get("target"),
        "firstGpuMonotonicMs": first_frame_at,
        "foregroundAfterFirstPresent": summarize_windows(steady),
        "normalSupply": summarize_windows(normal),
        "tenSecondGroups": ten_second_groups,
        "bindingCountsByPeerEpoch": dict(collections.Counter(str(row["payload"].get("peerEpoch")) for row in grouped["videoSurfaceAttached"])),
        "stateCounts": dict(collections.Counter(row["payload"].get("state") for row in grouped["streamingStateChanged"])),
        "dropCounts": dict(drops),
        "frameDelivery": {
            "accepted": len(accepted),
            "submitted": len(submissions),
            "gpuCompleted": len(completions),
            "mailboxOverwriteEvents": overwrite_events,
            "acceptedMinusOverwriteMatchesSubmit": len(accepted) - overwrite_events == len(submissions),
            "everySubmitHasMatchingGpuCompletion": set(submissions) == set(completions),
            "submittedWithoutAcceptance": len(set(submissions) - set(accepted)),
            "entryFrameCountFromAcceptedPlusIngressDrops": len(accepted) + drops.get("ingressCoalesced", 0),
        },
        "metrics": metrics,
        "gpuGapsOver100Ms": sorted(gaps, key=lambda gap: gap["durationMs"], reverse=True),
        "lowWindows": sorted(steady, key=lambda window: window["presentCountDeltaFps"])[:12],
        "lastRtc": last_rtc,
        "lastDisplay": grouped["displayHealthSnapshot"][-1] if grouped["displayHealthSnapshot"] else None,
        "lastFrameFeedback": grouped["frameFeedback"][-1] if grouped["frameFeedback"] else None,
        "lastRtcToStopMs": clock(terminal) - clock(last_rtc) if terminal and last_rtc else None,
        "terminal": grouped["sessionTerminal"],
        "configuration": [{"seq": row["seq"], "payload": {key: row["payload"].get(key) for key in ["deviceModel", "osVersion", "targetVideoWidth", "targetVideoHeight", "maxVideoBitrateKbps", "iceTransportPolicy", "preferIPv6", "preventResolutionDrops"]}} for row in grouped["peerRuntimeConfigured"]],
        "supervision": {field: dict(collections.Counter(str(row["payload"].get(field)) for row in grouped["runtimeSupervisionSnapshot"])) for field in ["networkPressure", "qualityState", "connectionRecoveryActive", "fresh"]},
        "controlsVisibilityMarkers": grouped["streamControlsVisibilityChanged"],
    }
    return report, windows


def write_json(root, name, value):
    (root / name).write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--baseline", type=Path)
    args = parser.parse_args()
    raw = args.source.read_bytes()
    rows = [json.loads(line) for line in raw.splitlines() if line.strip()]
    assert len({row["sessionId"] for row in rows}) == 1, "Analyze each launch session separately"
    assert all(row["seq"] == index for index, row in enumerate(rows, 1)), "Expected a complete contiguous raw trace"
    args.output.mkdir(parents=True, exist_ok=True)
    intervals = foreground_intervals(rows)
    launches = [row for row in rows if row["event"] == "streamLaunchStarted"]
    attempts, all_windows = [], []
    for index, launch in enumerate(launches):
        end_seq = launches[index + 1]["seq"] if index + 1 < len(launches) else rows[-1]["seq"] + 1
        selected = [row for row in rows if launch["seq"] <= row["seq"] < end_seq]
        report, windows = analyze_attempt(selected, launch, intervals, index + 1)
        attempts.append(report)
        all_windows.extend(windows)
    summary = {
        "input": {"path": str(args.source), "sha256": hashlib.sha256(raw).hexdigest(), "bytes": len(raw), "rows": len(rows), "firstTsMs": min(row["tsMs"] for row in rows), "lastTsMs": max(row["tsMs"] for row in rows)},
        "foregroundIntervalsMonotonicMs": intervals,
        "attempts": attempts,
        "eventCounts": dict(collections.Counter(row["event"] for row in rows)),
        "writerQueueDelayMs": distribution(row["payload"].get("writerQueueDelayMs") for row in rows),
        "method": {
            "fps": "sum(counter delta) / sum(actual sampleWindowMs); same attempt, generation, peerEpoch, sampleId and equal RTC/display time boundaries",
            "foreground": "complete sample inside active scene interval after first GPU completion",
            "normalSupply": "foreground sample with decode FPS >= 50, matching previous baseline method",
            "quantiles": "nearest rank; per-frame GPU timings and rolling drawable snapshot quantiles are reported separately",
            "gpuGap": "sort by actual presentedAtMonotonicMs, independent of delayed MainActor trace delivery",
            "ingressTimingBoundary": "timings observe accepted latest frames; coalesced discarded frames have no individual queue-age sample",
            "drawableBoundary": "drawable FPS uses aligned health deltas; per-drawable timestamps remain sampled cumulative distributions",
        },
    }
    timeline_events = {"scenePhaseChanged", "streamLaunchStarted", "sessionReady", "sessionSuspended", "sessionResumeValidated", "streamingStateChanged", "displayPipelineReady", "displayPipelineReadyDelivered", "videoSurfaceAttached", "firstVideoFrame", "displayFirstPresent", "steadyMediaObserved", "fallbackTurnRetry", "runtimeRebuildStarted", "runtimeRebuildOfferStarted", "runtimeRebuildNegotiated", "peerConnected", "sessionTerminal", "terminalSelected", "presentationDetached", "inputStopped", "iceTasksCancelled", "peerClosed", "remoteSessionClosed", "accessReleased", "audioPlayoutGlitchDetected"}
    ranges = [(125, 142), (430, 478), (535, 556), (13380, 13405), (15500, 15595), (24916, 24940), (25460, 25478), (26000, 26020), (26143, 26163)]
    anchors = {
        "timeline": [row for row in rows if row["event"] in timeline_events],
        "rawRanges": [{"fromSeq": start, "toSeq": end, "rows": [row for row in rows if start <= row["seq"] <= end]} for start, end in ranges],
        "largestGpuToMainActor": sorted([row for row in rows if row["event"] == "displayPresent"], key=lambda row: row["payload"].get("gpuToMainActorMs", 0), reverse=True)[:10],
        "largestIngressQueue": sorted([row for row in rows if row["event"] == "videoFrameAcceptedToMailbox"], key=lambda row: row["payload"].get("ingressQueueMs", 0), reverse=True)[:10],
        "largestIngressCoalescedDrops": sorted([row for row in rows if row["event"] == "videoFrameDropped"], key=lambda row: row["payload"].get("count", 1), reverse=True)[:10],
    }
    write_json(args.output, "health-summary.json", summary)
    write_json(args.output, "evidence-anchors.json", anchors)
    with (args.output / "steady-windows.csv").open("w", newline="") as file:
        fields = sorted({key for window in all_windows for key in window})
        writer = csv.DictWriter(file, fieldnames=fields)
        writer.writeheader()
        writer.writerows(all_windows)
    if args.baseline:
        baseline = json.loads(args.baseline.read_text())
        comparison = {
            "baselineInput": baseline["input"],
            "newInput": summary["input"],
            "method": "同 sampleId、完整前台、首显后的解码 >=50FPS 窗口；低供给窗口在完整前台结果中保留",
            "baselineNormalSupply": baseline["baselineNormalSupply"],
            "newNormalSupply": [{"attempt": attempt["attempt"], **attempt["normalSupply"]} for attempt in attempts],
            "baselineBindingCounts": baseline["attempts"],
            "newBindingCounts": [attempt["bindingCountsByPeerEpoch"] for attempt in attempts],
            "baselineApplicationDrops": baseline["applicationDropCounts"],
            "newApplicationDrops": [attempt["dropCounts"] for attempt in attempts],
            "interpretation": "两份1440p日志的场景、运行时负载、采样时长和网络条件各自记录；新日志验证轨道绑定收敛，实际帧供给差仍待优化。",
        }
        write_json(args.output, "comparison.json", comparison)
    write_json(args.output, "analysis-input.json", {"input": summary["input"], "scriptSha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(), "method": summary["method"]})
    for attempt in attempts:
        print(json.dumps({"attempt": attempt["attempt"], "foreground": attempt["foregroundAfterFirstPresent"], "normalSupply": attempt["normalSupply"], "bindingCounts": attempt["bindingCountsByPeerEpoch"], "dropCounts": attempt["dropCounts"], "frameDelivery": attempt["frameDelivery"], "metrics": {key: value for key, value in attempt["metrics"].items() if key in ["submitToGpuCompletionMs", "videoFrameAcceptedToMailbox.ingressQueueMs", "displayPresent.gpuToMainActorMs", "displayPresent.gpuToTraceRecordMs"]}, "lastRtcToStopMs": attempt["lastRtcToStopMs"]}, ensure_ascii=False))


if __name__ == "__main__":
    main()
