import copy
import importlib.util
import json
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from browser_runtime_stability_report import build_stability_report


def row(event, payload, seq):
    return {"event": event, "payload": {"clientGeneration": 1, "runtimeGeneration": 1, **payload}, "sessionId": "fixture", "traceRunId": "fixture", "seq": seq}


def healthy():
    facts = {"receivedFps": 60, "decodeFps": 60, "videoPresentFps": 60, "callbackFps": 60, "rendererSubmitFps": 60, "presentFps": 60,
        "rendererKind": "webgl2", "sourceWidth": 2560, "sourceHeight": 1440, "outputWidth": 2560, "outputHeight": 1440,
        "jitterMs": 5, "rttMs": 15, "windows": {"short": {"durationMs": 3000}}}
    resources = {field: 0 for field in ("videoElements", "audioElements", "currentRenderers", "pendingRenderers", "nativeFrameCallbacks", "frameRequests", "captureRequests", "buttonReleaseTimers", "microphoneTracks", "microphonePermissionRequests", "audioContexts", "audioNodes")}
    resources["renderers"] = []
    active = {**resources, "videoElements": 1, "currentRenderers": 1, "nativeFrameCallbacks": 1, "frameRequests": 2, "renderers": [{"generation": 1, "kind": "webgl2", "gpuObjects": 8, "frameRequests": 1, "animationCallbacks": 0, "captureWaiters": 0, "domNodes": 1, "drawCount": 60, "duplicateDrawCount": 0}]}
    return [
        row("browserWebRtcStatsObserved", {"peerGeneration": 2, "sampleId": "2:1", "collectionStartedAtMs": 1000, "sampledAtMs": 1010}, 1),
        row("browserObservationConsumed", {"peerGeneration": 2, "sampleId": "2:1", "consumer": "panel", "state": "fresh", "facts": facts}, 2),
        row("browserObservationConsumed", {"peerGeneration": 2, "sampleId": "2:1", "consumer": "supervision", "state": "fresh", "facts": copy.deepcopy(facts)}, 3),
        row("browserMediaResourcesSnapshot", {"reason": "snapshot", "resources": active}, 4),
        row("renderTelemetryObserved", {"rendererGeneration": 1, "sampledAtMs": 1010, "rendererGpuErrorChecks": 1,
            "trace": {"inFlight": 1, "pending": 0, "droppedEssential": 0}}, 4),
        row("browserMediaResourcesObserved", {"reason": "clientClosed", "resources": resources}, 5),
    ]


class StabilityFixtures(unittest.TestCase):
    def reasons(self, rows):
        return [item["reason"] for item in build_stability_report(rows)["failures"]]

    def test_complete_observation_and_cleanup(self):
        self.assertEqual(build_stability_report(healthy())["gate"], "PASS")

    def test_shared_frame_polling_is_bounded_and_released(self):
        rows = healthy()
        rows[3]["payload"]["resources"]["animationCallbacks"] = 1
        rows[3]["payload"]["resources"]["releasedFrameSource"] = {"frameRequests": 0, "nativeFrameCallbacks": 0, "animationCallbacks": 0}
        rows[-1]["payload"]["resources"]["animationCallbacks"] = 0
        self.assertEqual(build_stability_report(rows)["gate"], "PASS")
        rows[3]["payload"]["resources"]["animationCallbacks"] = 2
        rows[3]["payload"]["resources"]["releasedFrameSource"]["animationCallbacks"] = 1
        rows[-1]["payload"]["resources"]["animationCallbacks"] = 1
        reasons = self.reasons(rows)
        self.assertIn("resourceBound:animationCallbacks", reasons)
        self.assertIn("releasedVideoCallbacksRemain", reasons)
        self.assertIn("cleanupNotZero:animationCallbacks", reasons)

    def test_empty_and_missing_panel_evidence(self):
        self.assertEqual(build_stability_report([])["gate"], "MISSING")
        self.assertIn("sharedSamples", build_stability_report(healthy()[::2])["missingEvidence"])

    def test_changed_facts_fail_for_the_same_sample(self):
        rows = healthy()
        rows[2]["payload"]["facts"]["decodeFps"] = 0
        self.assertIn("sameSampleFactsMismatch", self.reasons(rows))

    def test_peer_and_client_generations_isolate_counter_reuse(self):
        rows = healthy()
        fresh = copy.deepcopy(rows[2])
        fresh["payload"]["clientGeneration"] = 2
        fresh["payload"]["facts"]["decodeFps"] = 0
        rows.append(fresh)
        closed = copy.deepcopy(healthy()[-1])
        closed["payload"]["clientGeneration"] = 2
        rows.append(closed)
        self.assertEqual(build_stability_report(rows)["gate"], "PASS")

    def test_overlapping_collection_and_excessive_rate(self):
        rows = healthy()
        for i in range(1, 6):
            rows.append(row("browserWebRtcStatsObserved", {"peerGeneration": 2, "collectionStartedAtMs": 1000 + i, "sampledAtMs": 1010 + i}, 6 + i))
        reasons = self.reasons(rows)
        self.assertIn("overlappingGetStats", reasons)
        self.assertIn("collectionRateAbove4Hz", reasons)

    def test_four_hz_at_window_boundary_is_valid(self):
        rows = healthy()
        closed = rows.pop()
        for i in range(1, 10):
            rows.append(row("browserWebRtcStatsObserved", {"peerGeneration": 2, "collectionStartedAtMs": 1000 + i * 250, "sampledAtMs": 1010 + i * 250}, 6 + i))
        rows.append(closed)
        self.assertEqual(build_stability_report(rows)["gate"], "PASS")

    def test_failed_cleanup_and_duplicate_draw_fail(self):
        rows = healthy()
        rows[3]["payload"]["resources"]["renderers"][0]["duplicateDrawCount"] = 1
        rows[-1]["payload"]["resources"]["captureRequests"] = 1
        rows[-1]["payload"]["resources"]["readyTimers"] = 1
        reasons = self.reasons(rows)
        self.assertIn("duplicateFrameProcessing", reasons)
        self.assertIn("cleanupNotZero:captureRequests", reasons)
        self.assertIn("cleanupNotZero:readyTimers", reasons)

    def test_each_owner_requires_cleanup_and_late_commits_fail(self):
        rows = healthy()
        active = copy.deepcopy(rows[3])
        active["payload"]["clientGeneration"] = 2
        rows.append(active)
        self.assertIn("allClientOwnersClosed", build_stability_report(rows)["missingEvidence"])
        rows.append(copy.deepcopy(rows[3]))
        self.assertIn("resourceCommitAfterClientClosed", self.reasons(rows))

    def test_recovery_improvement_requires_new_video_progress(self):
        rows = healthy()
        rows.append(row("recoveryActionEffectEvaluated", {"action": "playbackRebuild", "result": "improved", "newFramesObserved": False, "playbackGenerationAdvanced": False}, 6))
        self.assertIn("improvementWithoutNewFrames", self.reasons(rows))
        self.assertIn("playbackImprovementWithoutGenerationChange", self.reasons(rows))

    def test_actual_source_floor_and_five_stage_chain(self):
        rows = healthy()
        rows[1]["payload"]["facts"]["outputWidth"] = 1920
        rows[2]["payload"]["facts"]["outputWidth"] = 1920
        self.assertIn("outputBelowActualSource", self.reasons(rows))
        for row_ in rows:
            facts = row_["payload"].get("facts")
            if facts:
                facts.pop("callbackFps")
        self.assertIn("frameChainSamples", build_stability_report(rows)["missingEvidence"])

    def test_soft_actions_require_domain_coverage_interval_and_capability(self):
        rows = healthy()
        rows.append(row("qualityActionPlanned", {"reason": "sustainedCongestion", "qualitySupported": False,
            "evidence": {"domain": "network", "coverageMs": 2500, "sustainedMs": 2000, "observedAtMs": 10000, "previousActionAtMs": 0}}, 7))
        reasons = self.reasons(rows)
        self.assertIn("softActionWithoutContinuousCoverage", reasons)
        self.assertIn("softActionIntervalBelow15s", reasons)
        self.assertIn("networkActionWithoutRemoteCapability", reasons)

    def test_interval_fps_and_policy_window_fps_have_independent_durations(self):
        rows = healthy()
        rows[0]["payload"]["delta"] = {"elapsedMs": 250}
        window = {
            "durationMs": 3000, "inboundFpsAvg": 30, "decodeFpsAvg": 28,
            "videoPresentFpsAvg": 26, "callbackFpsAvg": 15, "rendererSubmitFpsAvg": 13,
            "receivedCoverageMs": 3000, "decodeCoverageMs": 3000, "videoCoverageMs": 3000,
            "callbackCoverageMs": 3000, "rendererCoverageMs": 2500,
        }
        for index in (1, 2):
            rows[index]["payload"]["facts"]["windows"]["short"] = copy.deepcopy(window)
        report = build_stability_report(rows)
        self.assertEqual(report["gate"], "PASS")
        sample = report["frameSupply"][0]
        self.assertEqual(sample["rateScope"], "sampleInterval")
        self.assertEqual(sample["windowMs"], 250)
        self.assertEqual(sample["policyWindowMs"], 3000)
        self.assertEqual(sample["receivedFps"], 60)
        self.assertEqual(sample["windowFps"]["receivedFps"], 30)
        self.assertEqual(sample["windowFps"]["rendererSubmitFps"], 13)
        self.assertEqual(sample["windowCoverageMs"]["rendererSubmitFps"], 2500)

    def test_trace_and_periodic_gpu_query_budgets(self):
        rows = healthy()
        telemetry = copy.deepcopy(rows[-2])
        telemetry["payload"].update({"sampledAtMs": 2010, "rendererGpuErrorChecks": 61, "trace": {"inFlight": 5, "pending": 33, "droppedEssential": 1}})
        rows.insert(-1, telemetry)
        reasons = self.reasons(rows)
        self.assertIn("traceQueueBudgetExceeded", reasons)
        self.assertIn("essentialTraceEventsLost", reasons)
        self.assertIn("gpuQueriesAbovePeriodicBudget", reasons)

    def test_rtt_deadline_and_post_action_coverage(self):
        rows = healthy()
        rows.append(row("recoveryActionEffectEvaluated", {"action": "keyframeRequest", "result": "improved", "newFramesObserved": True,
            "requiredObserveMs": 18000, "postActionCoverageMs": 1500}, 7))
        reasons = self.reasons(rows)
        self.assertIn("improvementBeyondPathObservationDeadline", reasons)
        self.assertIn("improvementWithoutPostActionCoverage", reasons)

    def test_monotonic_observation_time_orders_late_ipc_delivery(self):
        rows = healthy()
        for index, item in enumerate(rows):
            item["payload"]["observedAtMs"] = 1000 + index * 10
        rows = list(reversed(rows))
        self.assertEqual(build_stability_report(rows)["gate"], "PASS")

    def test_network_and_renderer_effects_require_coverage_deadline_and_generation(self):
        rows = healthy()
        closed = rows.pop()
        rows.extend([
            row("qualityActionPlanned", {"qualitySupported": True, "reason": "sustainedCongestion",
                "evidence": {"domain": "network", "coverageMs": 3000, "sustainedMs": 3000, "observedAtMs": 10000}}, 7),
            row("qualityActionEffectEvaluated", {"result": "improved", "postActionCoverageMs": 3000, "requiredObserveMs": 3000}, 8),
            row("renderLocalActionPlanned", {"reason": "sustainedRendererPressure",
                "evidence": {"domain": "renderer", "coverageMs": 3000, "sustainedMs": 3000, "observedAtMs": 10000}}, 9),
            row("renderLocalActionEffectEvaluated", {"result": "improved", "postActionCoverageMs": 3000,
                "rendererGeneration": 2, "baselineRendererGeneration": 1}, 10),
        ])
        closed["seq"] = 11
        rows.append(closed)
        self.assertEqual(build_stability_report(rows)["gate"], "PASS")
        rows[-4]["payload"].update({"postActionCoverageMs": 1500, "requiredObserveMs": 18000})
        rows[-2]["payload"]["baselineRendererGeneration"] = 2
        reasons = self.reasons(rows)
        self.assertIn("improvementWithoutPostActionCoverage", reasons)
        self.assertIn("improvementBeyondPathObservationDeadline", reasons)
        self.assertIn("renderImprovementWithoutGenerationChange", reasons)

    def test_existing_behavior_report_enforces_the_opted_in_stability_gate(self):
        root = Path(__file__).resolve().parents[2]
        path = root / ".agents/skills/analyze-runtime-logs/scripts/trace_browser_webrtc_behavior_report.py"
        spec = importlib.util.spec_from_file_location("browser_behavior_integration", path)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        rows = healthy()
        for seq, stage in enumerate(("localOfferAfterPatch", "remoteAnswer"), 6):
            rows.append(row("browserWebRtcSdpObserved", {"stage": stage, "h264Payloads": []}, seq))
        for seq, kind in enumerate(("remoteAnswerSet", "firstInboundPacket", "firstDecoded", "firstKeyframeDecoded", "firstPresented"), 8):
            rows.append(row("browserWebRtcTimelineObserved", {"kind": kind, "observedAtMs": 1000}, seq))
        rows.append(row("browserWebRtcTimelineObserved", {"kind": "connectionStateChanged", "connectionState": "connected", "observedAtMs": 1000}, 13))
        with tempfile.TemporaryDirectory(prefix="browser-stability-fixture-") as directory:
            trace = Path(directory) / "trace.jsonl"
            trace.write_text("\n".join(json.dumps(item) for item in rows), encoding="utf-8")
            report = module.build_report(trace, max_age_seconds=None, require_presented=True, require_stability=True)
            self.assertEqual(report["browserWebRtcBehaviorGate"], "PASS")
            rows[2]["payload"]["facts"]["decodeFps"] = 0
            trace.write_text("\n".join(json.dumps(item) for item in rows), encoding="utf-8")
            report = module.build_report(trace, max_age_seconds=None, require_presented=True, require_stability=True)
            self.assertEqual(report["browserWebRtcBehaviorGate"], "FAIL")
            legacy = module.build_report(trace, max_age_seconds=None, require_presented=True)
            self.assertEqual(legacy["browserWebRtcBehaviorGate"], "PASS")


if __name__ == "__main__":
    unittest.main()
