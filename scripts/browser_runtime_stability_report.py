"""浏览器采样、共享事实、资源交接和绘制的可复算稳定性门槛。"""

from __future__ import annotations

import math
from collections import Counter, defaultdict, deque
from typing import Any

FRAME_FIELDS = ("receivedFps", "decodeFps", "videoPresentFps", "callbackFps", "rendererSubmitFps")
WINDOW_RATE_FIELDS = ("inboundFpsAvg", "decodeFpsAvg", "videoPresentFpsAvg", "callbackFpsAvg", "rendererSubmitFpsAvg")
WINDOW_COVERAGE_FIELDS = ("receivedCoverageMs", "decodeCoverageMs", "videoCoverageMs", "callbackCoverageMs", "rendererCoverageMs")
POLICY_EVENTS = {"qualityActionPlanned", "renderLocalActionPlanned", "recoveryActionPlanned"}


def finite(value: Any) -> bool:
    return isinstance(value, (float, int)) and not isinstance(value, bool) and math.isfinite(value)


def build_stability_report(rows: list[dict[str, Any]]) -> dict[str, Any]:
    failures: list[dict[str, Any]] = []
    evidence: Counter[str] = Counter()
    samples: dict[tuple, dict[str, Any]] = {}
    consumers: dict[tuple, set[str]] = defaultdict(set)
    collection: dict[tuple, dict[str, Any]] = {}
    sample_intervals: dict[tuple, float] = {}
    ticks: dict[tuple, deque] = defaultdict(deque)
    renderer_draws: dict[tuple, float] = {}
    active_owners: set[tuple] = set()
    closed_owners: set[tuple] = set()
    frame_supply: list[dict[str, Any]] = []
    telemetry: dict[tuple, dict[str, Any]] = {}
    action_domains: Counter[str] = Counter()

    # 前端 IPC 允许到达乱序；带完整单调时钟的 owner 按实际观测时间复算。
    relevant = {"browserObservationConsumed", "browserWebRtcStatsObserved", "browserMediaResourcesObserved", "browserMediaResourcesSnapshot", "renderTelemetryObserved", "recoveryActionEffectEvaluated", "qualityActionEffectEvaluated", "renderLocalActionEffectEvaluated", *POLICY_EVENTS}
    grouped: dict[tuple, list[dict[str, Any]]] = defaultdict(list)
    for row in rows:
        payload = row.get("payload") or {}
        if row.get("event") in relevant and isinstance(payload, dict):
            grouped[(row.get("traceRunId"), row.get("sessionId"), payload.get("clientGeneration"))].append(row)
    rows = []
    for group in grouped.values():
        if all(finite((r.get("payload") or {}).get("observedAtMs")) for r in group):
            group.sort(key=lambda r: r["payload"]["observedAtMs"])
        rows.extend(group)

    def fail(reason: str, row: dict[str, Any]) -> None:
        failures.append({"reason": reason, "seq": row.get("seq")})

    for row in rows:
        payload = row.get("payload") or {}
        if not isinstance(payload, dict):
            continue
        event = row.get("event")
        owner = (row.get("traceRunId"), row.get("sessionId"), payload.get("clientGeneration"))
        if event == "browserObservationConsumed":
            evidence["observationEvents"] += 1
            active_owners.add(owner)
            if owner in closed_owners:
                fail("observationCommitAfterClientClosed", row)
            identity = (*owner, payload.get("peerGeneration"), payload.get("sampleId"))
            if payload.get("sampleId") is None or payload.get("clientGeneration") is None:
                fail("observationIdentityMissing", row)
                continue
            # 新鲜度和呈现 age 随读取时间变化；这些字段的动态值单独解释。
            facts = payload.get("facts")
            if not isinstance(facts, dict):
                fail("observationFactsMissing", row)
                continue
            if identity in samples and samples[identity] != facts:
                fail("sameSampleFactsMismatch", row)
            first_sample = identity not in samples
            samples[identity] = facts
            consumers[identity].add(payload.get("consumer", "unknown"))
            if first_sample and all(finite(facts.get(field)) and facts[field] >= 0 for field in FRAME_FIELDS):
                evidence["frameChainSamples"] += 1
                short = (facts.get("windows") or {}).get("short") or {}
                frame_supply.append({"sampleId": payload.get("sampleId"), "peerGeneration": payload.get("peerGeneration"),
                    "videoGeneration": payload.get("videoGeneration"), "rendererGeneration": payload.get("rendererGeneration"),
                    "sampledAtMs": payload.get("sampledAtMs"), "rateScope": "sampleInterval", "windowMs": sample_intervals.get(identity),
                    "policyWindowMs": short.get("durationMs"),
                    "windowFps": dict(zip(FRAME_FIELDS, (short.get(field) for field in WINDOW_RATE_FIELDS))),
                    "windowCoverageMs": dict(zip(FRAME_FIELDS, (short.get(field) for field in WINDOW_COVERAGE_FIELDS))),
                    **{field: facts[field] for field in FRAME_FIELDS},
                    "receiveDecodeDelta": facts["receivedFps"] - facts["decodeFps"],
                    "decodeVideoDelta": facts["decodeFps"] - facts["videoPresentFps"],
                    "videoCallbackDelta": facts["videoPresentFps"] - facts["callbackFps"],
                    "callbackSubmitDelta": facts["callbackFps"] - facts["rendererSubmitFps"]})
            dimensions = [facts.get(field) for field in ("sourceWidth", "sourceHeight", "outputWidth", "outputHeight")]
            if all(finite(v) and v > 0 for v in dimensions):
                evidence["actualDimensionSamples"] += 1
                if facts.get("rendererKind") in ("webgl2", "webgl2_sr") and (dimensions[2] < dimensions[0] or dimensions[3] < dimensions[1]):
                    fail("outputBelowActualSource", row)
            if payload.get("state") == "fresh":
                evidence["freshObservationEvents"] += 1
        elif event == "browserWebRtcStatsObserved":
            if payload.get("clientGeneration") is None and payload.get("runtimeGeneration") is None:
                evidence["legacyCollectionEvents"] += 1
                continue
            if payload.get("clientGeneration") is None or payload.get("peerGeneration") is None:
                fail("collectionIdentityMissing", row)
                continue
            key = (*owner, payload["peerGeneration"])
            delta = payload.get("delta")
            if isinstance(delta, dict) and finite(delta.get("elapsedMs")) and delta["elapsedMs"] > 0:
                sample_intervals[(*key, payload.get("sampleId"))] = delta["elapsedMs"]
            active_owners.add(owner)
            if owner in closed_owners:
                fail("statsCommitAfterClientClosed", row)
            start, end = payload.get("collectionStartedAtMs"), payload.get("sampledAtMs")
            if not finite(start) or not finite(end) or end < start:
                fail("collectionTimingMissingOrInvalid", row)
                continue
            previous = collection.get(key)
            if previous and start < previous["sampledAtMs"]:
                fail("overlappingGetStats", row)
            if previous and end <= previous["sampledAtMs"]:
                fail("nonMonotonicSample", row)
            collection[key] = payload
            window = ticks[key]
            while window and window[0] <= start - 1000:
                window.popleft()
            window.append(start)
            if len(window) > 4:
                fail("collectionRateAbove4Hz", row)
            evidence["collectionEvents"] += 1
        elif event in ("browserMediaResourcesObserved", "browserMediaResourcesSnapshot"):
            resources = payload.get("resources")
            if not isinstance(resources, dict):
                fail("resourceSnapshotMissing", row)
                continue
            evidence["resourceEvents"] += 1
            if payload.get("clientGeneration") is None:
                fail("resourceIdentityMissing", row)
            active_owners.add(owner)
            if owner in closed_owners and payload.get("reason") != "clientClosed":
                fail("resourceCommitAfterClientClosed", row)
            for field in ("currentRenderers", "pendingRenderers", "nativeFrameCallbacks", "videoElements", "audioElements"):
                value = resources.get(field)
                if not finite(value) or value < 0 or value > 1:
                    fail(f"resourceBound:{field}", row)
            animation_callbacks = resources.get("animationCallbacks", 0)
            if not finite(animation_callbacks) or animation_callbacks < 0 or animation_callbacks > 1:
                fail("resourceBound:animationCallbacks", row)
            renderers = resources.get("renderers")
            if not isinstance(renderers, list) or len(renderers) > 2:
                fail("rendererListMissingOrUnbounded", row)
                continue
            if len(renderers) != (resources.get("currentRenderers", 0) + resources.get("pendingRenderers", 0)):
                fail("rendererOwnershipCountMismatch", row)
            for renderer in renderers:
                if not isinstance(renderer, dict):
                    fail("rendererSnapshotInvalid", row)
                    continue
                if renderer.get("kind") == "video":
                    evidence["nativeRendererEvents"] += 1
                    continue
                if finite(renderer.get("drawCount")):
                    count = renderer["drawCount"]
                    key = (*owner, renderer.get("generation"))
                    if key in renderer_draws and count < renderer_draws[key]:
                        fail("rendererDrawCounterRegression", row)
                    renderer_draws[key] = count
                    if count > 0:
                        evidence["shaderDrawEvents"] += 1
                if (renderer.get("duplicateDrawCount") or 0) > 0:
                    fail("duplicateFrameProcessing", row)
                for field in ("gpuObjects", "frameRequests", "animationCallbacks", "captureWaiters", "domNodes"):
                    if not finite(renderer.get(field)) or renderer[field] < 0:
                        fail(f"rendererResourceMissing:{field}", row)
            released = resources.get("releasedFrameSource")
            if isinstance(released, dict) and (released.get("frameRequests") != 0 or released.get("nativeFrameCallbacks") != 0 or released.get("animationCallbacks", 0) != 0):
                fail("releasedVideoCallbacksRemain", row)
            if payload.get("reason") == "clientClosed":
                evidence["closedClients"] += 1
                closed_owners.add(owner)
                for field in ("videoElements", "audioElements", "currentRenderers", "pendingRenderers", "nativeFrameCallbacks", "frameRequests", "captureRequests", "buttonReleaseTimers", "microphoneTracks", "microphonePermissionRequests", "audioContexts", "audioNodes"):
                    if resources.get(field) != 0:
                        fail(f"cleanupNotZero:{field}", row)
                if renderers:
                    fail("closedClientRetainsRenderer", row)
                if resources.get("readyTimers", 0) != 0:
                    fail("cleanupNotZero:readyTimers", row)
                if resources.get("animationCallbacks", 0) != 0:
                    fail("cleanupNotZero:animationCallbacks", row)
        elif event == "renderTelemetryObserved":
            evidence["telemetryEvents"] += 1
            trace = payload.get("trace")
            if isinstance(trace, dict):
                evidence["traceBudgetSamples"] += 1
                if not finite(trace.get("inFlight")) or not 0 <= trace["inFlight"] <= 4 or not finite(trace.get("pending")) or not 0 <= trace["pending"] <= 32:
                    fail("traceQueueBudgetExceeded", row)
                if (trace.get("droppedEssential") or 0) > 0:
                    fail("essentialTraceEventsLost", row)
            if (payload.get("rendererDuplicateDrawCount") or 0) > 0:
                fail("duplicateFrameProcessing", row)
            key = (*owner, payload.get("rendererGeneration"))
            previous = telemetry.get(key)
            at = payload.get("sampledAtMs")
            checks = payload.get("rendererGpuErrorChecks")
            if finite(at) and finite(checks):
                evidence["gpuBudgetSamples"] += 1
                dimensions = ("sourceWidth", "sourceHeight", "outputWidth", "outputHeight")
                if previous and all(previous.get(k) == payload.get(k) for k in dimensions):
                    elapsed = at - previous["sampledAtMs"]
                    if elapsed > 0 and checks - previous["rendererGpuErrorChecks"] > math.ceil(elapsed / 1000) + 1:
                        fail("gpuQueriesAbovePeriodicBudget", row)
                telemetry[key] = payload
        elif event in POLICY_EVENTS:
            proof = payload.get("evidence")
            evidence["softActionEvents"] += 1
            if not isinstance(proof, dict):
                evidence["actionProofMissing"] += 1
                continue
            evidence["qualifiedActionEvents"] += 1
            domain = proof.get("domain")
            action_domains[domain or "unknown"] += 1
            reason = payload.get("reason", "")
            rollback = reason.startswith("enhancementTrial") or reason == "ineffectiveRestore"
            required = 15000 if reason in ("healthyRestore", "healthyEnhancementTrial") else 3000
            if domain not in ("network", "renderer", "decode"):
                fail("softActionDomainMissing", row)
            if not rollback and (not finite(proof.get("coverageMs")) or proof["coverageMs"] < required - 1 or not finite(proof.get("sustainedMs")) or proof["sustainedMs"] < required - 1):
                fail("softActionWithoutContinuousCoverage", row)
            at, previous = proof.get("observedAtMs"), proof.get("previousActionAtMs")
            if not rollback and finite(at) and finite(previous) and at - previous < 15000:
                fail("softActionIntervalBelow15s", row)
            if domain == "network" and payload.get("qualitySupported") is not True:
                fail("networkActionWithoutRemoteCapability", row)
        elif event == "recoveryActionEffectEvaluated":
            evidence["effectEvents"] += 1
            if payload.get("result") == "improved" and payload.get("newFramesObserved") is not True:
                fail("improvementWithoutNewFrames", row)
            if payload.get("action") == "playbackRebuild" and payload.get("result") == "improved" and payload.get("playbackGenerationAdvanced") is not True:
                fail("playbackImprovementWithoutGenerationChange", row)
            if payload.get("result") == "improved" and finite(payload.get("requiredObserveMs")) and payload["requiredObserveMs"] > 15000:
                fail("improvementBeyondPathObservationDeadline", row)
            if payload.get("result") == "improved" and (not finite(payload.get("postActionCoverageMs")) or payload["postActionCoverageMs"] < 2999):
                fail("improvementWithoutPostActionCoverage", row)
        elif event in ("qualityActionEffectEvaluated", "renderLocalActionEffectEvaluated"):
            name = "networkEffectEvents" if event == "qualityActionEffectEvaluated" else "rendererEffectEvents"
            evidence[name] += 1
            if payload.get("result") == "improved":
                if not finite(payload.get("postActionCoverageMs")) or payload["postActionCoverageMs"] < 2999:
                    fail("improvementWithoutPostActionCoverage", row)
                if finite(payload.get("requiredObserveMs")) and payload["requiredObserveMs"] > 15000:
                    fail("improvementBeyondPathObservationDeadline", row)
                if event == "renderLocalActionEffectEvaluated" and payload.get("rendererGeneration") == payload.get("baselineRendererGeneration"):
                    fail("renderImprovementWithoutGenerationChange", row)

    evidence["sharedSamples"] = sum({"panel", "supervision"}.issubset(group) for group in consumers.values())
    missing = []
    for name in ("collectionEvents", "freshObservationEvents", "sharedSamples", "resourceEvents", "closedClients", "frameChainSamples", "actualDimensionSamples", "traceBudgetSamples"):
        if evidence[name] == 0:
            missing.append(name)
    if evidence["actionProofMissing"]:
        missing.append("softActionQualification")
    for domain, effect in (("network", "networkEffectEvents"), ("renderer", "rendererEffectEvents"), ("decode", "effectEvents")):
        if action_domains[domain] and evidence[effect] == 0:
            missing.append(effect)
    if evidence["shaderDrawEvents"] and evidence["gpuBudgetSamples"] == 0:
        missing.append("gpuBudgetSamples")
    if evidence["shaderDrawEvents"] == 0 and evidence["nativeRendererEvents"] == 0:
        missing.append("actualRendererEvidence")
    if active_owners - closed_owners:
        missing.append("allClientOwnersClosed")
    return {
        "gate": "FAIL" if failures else "MISSING" if missing else "PASS",
        "failures": failures,
        "missingEvidence": missing,
        "evidence": dict(evidence),
        "frameSupply": frame_supply[-32:],
        "actionDomains": dict(action_domains),
        "boundaries": [
            "资源计数覆盖当前 owner；堆/RSS 和 GPU 系统回收由实机长测验证。",
            "drawDurationMaxMs 表达 JS/WebGL 提交耗时；实际 GPU/上屏与 glass-to-glass 需要外部测量。",
            "五段帧链按样本/代际分列；吞吐差表达交付与处理差值，帧身份缺失时保留窗口边界误差。",
            "frameSupply 的 windowMs/rateScope 表达采集间隔；policyWindowMs/windowFps/windowCoverageMs 独立表达策略窗与各段覆盖。",
            "软回退与健康恢复分别验证 3s/15s 覆盖；失败试探回到已验证路径，回滚单列。",
            "quarantinedPermissionRequests 表达已失效且等待系统结算的权限请求；晚到音轨由权限 owner 释放。",
        ],
    }
