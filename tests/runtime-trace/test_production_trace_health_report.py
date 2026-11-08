import copy
import importlib.util
import json
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "scripts"))
from production_trace_health_report import build_health_report, browser_stats_projection, read_trace


def healthy(seconds=90, source_fps=60, renderer_fps=None):
    renderer_fps = renderer_fps or source_fps
    rows = []
    def row(event, payload):
        rows.append({"schemaVersion": 3, "seq": len(rows) + 1, "traceRunId": "fixture", "sessionId": "session", "traceProfile": "production", "event": event,
            "payload": {"productionContractVersion": 1, "runtimeGeneration": 1, "clientGeneration": 1, "observationEpoch": 1, **payload}})
    row("playbackObservationManifest", {"observedAtMs": 0, "configuration": {"requestedFps": 60, "expectedFps": source_fps, "rendererTargetFps": renderer_fps, "sharpness": 3},
        "capabilities": {"mediaBackend": "browser", "nativeRecoveryGate": "unsupported", "physicalPresent": "unavailable"}, "cadence": {"healthMs": 1000}, "visibility": "visible"})
    distribution = {"count": 256, "p95Ms": 0.8, "p99Ms": 1.2, "maxMs": 2}
    for second in range(1, seconds + 1):
        fps = dict(receivedFps=source_fps, decodeFps=source_fps, videoPresentFps=source_fps, callbackFps=source_fps, rendererSubmitFps=renderer_fps)
        delta = dict(received=source_fps, decoded=source_fps, videoPresent=source_fps, callbacks=source_fps, rendererSubmit=renderer_fps,
            framesDropped=0, freezeCount=0, keyframes=0, pli=0, fir=0, nack=0, bytesReceived=1000000, packetsReceived=1000, rateLimitDrops=source_fps-renderer_fps, submitFailures=0)
        row("browserPlaybackHealthObserved", {"observedAtMs": second * 1000, "healthSequence": second, "sampleId": f"1:{second}", "sampledAtMs": second*1000,
            "peerGeneration": 1, "videoGeneration": 1, "rendererGeneration": 1, "observationState": "fresh", "connectionState": "connected", "visibility": "visible", "expectedFps": source_fps, "rendererTargetFps": renderer_fps, "rendererKind": "webgl2",
            "window": {"startMs": (second-1)*1000, "endMs": second*1000, "elapsedMs": 1000, "sampleCount": 4, "partial": False},
            "coverage": {"validMs": 1000, "ratio": 1, "maxSampleGapMs": 250, "counterMs": {key: 1000 for key in ("received", "decoded", "videoPresent", "callbacks", "rendererSubmit")}},
            "network": {"rttMs": 8, "jitterMs": 2, "packetLossRatio": 0, "bitrateKbps": 8000}, "recovery": {"nativeRecoveryGate": "unsupported"},
            "frameSupply": {**fps, "delta": delta, "totals": {key: value*second for key, value in delta.items()}, "maxCallbackIntervalMs": 16.67, "sourceWidth": 2560, "sourceHeight": 1440},
            "renderer": {"timingScope": "retainedSamples", "timing": {field: copy.deepcopy(distribution) for field in ("duration", "upload", "submit", "interval")}, "gpuTiming": {"state": "ready", "duration": copy.deepcopy(distribution)}, "maxSubmitAgeMs": 5, "maxVideoPresentAgeMs": 5}, "trace": {"droppedEssential": 0, "backend": {"keyDropped": 0}}})
    row("playbackSessionSummary", {"observedAtMs": seconds*1000, "ended": True, "reason": "sessionEnded", "durationMs": seconds*1000})
    row("executionCleanupObserved", {"cleanupState": "completed", "results": {"runtimeStopped": True, "remoteClosed": True}})
    row("traceFlushObserved", {"status": "flushed", "flushedSequence": len(rows)})
    return rows


def health_rows(rows):
    return [row for row in rows if row["event"] == "browserPlaybackHealthObserved"]


class ProductionTraceFixtures(unittest.TestCase):
    def classifications(self, report):
        return {item["classification"] for segment in report["segments"] for item in segment["diagnoses"]}

    def test_complete_six_sections_and_clean_ending(self):
        report = build_health_report(healthy())
        self.assertEqual(report["gate"], "PASS")
        self.assertEqual(report["acceptanceGate"], "PASS")
        self.assertEqual(len(report["sections"]), 6)
        coverage = report["segments"][0]["coverage"]
        self.assertEqual(coverage["publicationRatio"], 1)
        self.assertEqual(coverage["mediaValidRatio"], 1)
        self.assertEqual(coverage["deltaTotals"]["received"], 5400)
        self.assertEqual(report["capabilities"]["browserPhysicalScanout"], "UNSUPPORTED")

    def test_source_30fps_and_user_renderer_limit_are_valid_targets(self):
        for rows in (healthy(source_fps=30), healthy(renderer_fps=30)):
            report = build_health_report(rows)
            self.assertEqual(report["gate"], "PASS")
            self.assertNotIn("sourceSupplyDeficit", self.classifications(report))
        self.assertTrue(build_health_report(healthy(renderer_fps=30))["segments"][0]["sections"]["frameSupplyDelta"]["intentionalRateLimitObserved"])

    def test_decode_deficit_is_localized_with_frame_delta(self):
        rows = healthy()
        for row in health_rows(rows):
            row["payload"]["frameSupply"].update(decodeFps=40, videoPresentFps=40, rendererSubmitFps=40)
            row["payload"]["frameSupply"]["delta"].update(decoded=40, videoPresent=40, rendererSubmit=40)
        report = build_health_report(rows)
        self.assertEqual(report["gate"], "FAIL")
        self.assertIn("decodeLimited", self.classifications(report))
        self.assertEqual(report["segments"][0]["sections"]["mediaSupply"]["receiveDecodeDeltaFps"], 20)

    def test_upload_and_gpu_tails_keep_reported_window_semantics(self):
        for name in ("uploadLimited", "gpuLimited"):
            rows = healthy()
            for row in health_rows(rows):
                renderer = row["payload"]["renderer"]
                distribution = renderer["gpuTiming"]["duration"] if name == "gpuLimited" else renderer["timing"]["upload"]
                distribution.update(p95Ms=22, p99Ms=30, maxMs=35)
            report = build_health_report(rows)
            self.assertEqual(report["gate"], "FAIL")
            self.assertIn(name, self.classifications(report))
            tail = report["segments"][0]["sections"]["submitPresentTail"]["gpu" if name == "gpuLimited" else "upload"]
            self.assertEqual(tail["maxReportedP99Ms"], 30)
            self.assertEqual(tail["latestCount"], 256)
            self.assertEqual(tail["scope"], "retainedSamples")

    def test_sparse_callback_with_normal_video_and_submit_supply(self):
        rows = healthy()
        for row in health_rows(rows):
            row["payload"]["frameSupply"].update(callbackFps=30, maxCallbackIntervalMs=150)
            row["payload"]["frameSupply"]["delta"]["callbacks"] = 30
        report = build_health_report(rows)
        self.assertEqual(report["gate"], "PASS")
        self.assertIn("callbackSparseBatching", self.classifications(report))
        self.assertTrue(report["segments"][0]["sections"]["steadySupply"]["callbackBatchingObserved"])

    def test_network_and_zero_supply_use_valid_observations(self):
        rows = healthy()
        for row in health_rows(rows):
            row["payload"]["network"]["packetLossRatio"] = 0.1
            row["payload"]["frameSupply"].update(receivedFps=0, decodeFps=0, videoPresentFps=0, rendererSubmitFps=0)
        report = build_health_report(rows)
        self.assertEqual(report["gate"], "FAIL")
        self.assertIn("networkDegraded", self.classifications(report))
        self.assertIn("sourceSupplyDeficit", self.classifications(report))

    def test_missing_publication_and_pending_media_have_separate_coverage(self):
        rows = [row for row in healthy() if row["event"] != "browserPlaybackHealthObserved" or row["payload"]["healthSequence"] % 5]
        report = build_health_report(rows)
        self.assertEqual(report["gate"], "INSUFFICIENT_EVIDENCE")
        self.assertLess(report["segments"][0]["coverage"]["publicationRatio"], 0.95)
        rows = healthy()
        for row in health_rows(rows)[40:50]:
            row["payload"]["coverage"].update(validMs=0, maxSampleGapMs=5000)
            row["payload"]["observationState"] = "stale"
            row["payload"]["collection"] = {"state": "pending", "ageMs": 5000}
        report = build_health_report(rows)
        coverage = report["segments"][0]["coverage"]
        self.assertEqual(coverage["publicationRatio"], 1)
        self.assertLess(coverage["mediaValidRatio"], 0.9)
        self.assertEqual(report["gate"], "INSUFFICIENT_EVIDENCE")

    def test_hidden_intervals_and_generation_changes_are_partitioned(self):
        rows = healthy()
        for row in health_rows(rows)[30:40]:
            row["payload"]["visibility"] = "hidden"
        for row in health_rows(rows)[40:]:
            row["payload"]["peerGeneration"] = 2
        report = build_health_report(rows)
        self.assertEqual(len(report["segments"]), 2)
        self.assertEqual(report["steadyDurationMs"], 80000)
        self.assertEqual(report["gate"], "PASS")

    def test_writer_key_loss_and_partial_ending_remain_explicit(self):
        rows = healthy()
        health_rows(rows)[-1]["payload"]["trace"]["backend"]["keyDropped"] = 1
        self.assertEqual(build_health_report(rows)["gate"], "INSUFFICIENT_EVIDENCE")
        rows = healthy()[:-2]
        report = build_health_report(rows)
        self.assertEqual(report["gate"], "PASS")
        self.assertEqual(report["acceptanceGate"], "INSUFFICIENT_EVIDENCE")
        self.assertTrue(report["ending"]["partialEnding"])
        rows = healthy()
        rows[-2]["payload"]["cleanupState"] = "pending"
        self.assertEqual(build_health_report(rows)["ending"]["status"], "FAIL")

    def test_touch_menu_failure_and_expected_geometric_boundary(self):
        rows = healthy()
        rows.append({"seq": 1000, "traceRunId": "fixture", "sessionId": "session", "event": "inputUiInteractionObserved", "payload": {"operationId": "4", "source": "touch", "action": "fullscreen", "scope": "actionMenu", "outcome": "blocked", "reason": "inactiveAncestorScope"}})
        self.assertEqual(build_health_report(rows)["input"]["status"], "FAIL")
        self.assertEqual(build_health_report(rows)["gate"], "FAIL")
        rows[-1]["payload"]["reason"] = "noCandidate"
        self.assertEqual(build_health_report(rows)["input"]["status"], "PASS")

    def test_old_production_and_newer_contract_do_not_receive_false_pass(self):
        report = build_health_report([{"event": "sessionStatePolled", "payload": {"streamState": "Provisioned"}}])
        self.assertEqual(report["gate"], "INSUFFICIENT_EVIDENCE")
        self.assertEqual(len(report["sections"]), 6)
        rows = healthy()
        for row in health_rows(rows):
            row["payload"]["productionContractVersion"] = 2
        self.assertEqual(build_health_report(rows)["gate"], "UNSUPPORTED")

    def test_dev_samples_are_deduplicated_and_project_to_the_same_five_stages(self):
        rows = []
        for sample in range(1, 361):
            metadata = {"traceRunId": "fixture", "sessionId": "session", "traceProfile": "dev"}
            p = {"runtimeGeneration": 1, "clientGeneration": 1, "peerGeneration": 1, "sampleId": str(sample), "sampledAtMs": sample*250}
            rows.append({**metadata, "event": "browserWebRtcStatsObserved", "payload": {**p, "connectionState": "connected", "delta": {"elapsedMs": 250}}})
            facts = dict(zip(("receivedFps", "decodeFps", "videoPresentFps", "callbackFps", "rendererSubmitFps"), [60]*5))
            for consumer in ("display", "panel"):
                rows.append({**metadata, "event": "browserObservationConsumed", "payload": {**p, "state": "fresh", "consumer": consumer, "facts": facts}})
        report = build_health_report(rows)
        self.assertEqual(report["devIntervals"], 360)
        self.assertEqual(report["segments"][0]["coverage"]["fps"]["received"], 60)

    def test_export_loading_and_browser_behavior_projection(self):
        rows = healthy()
        projection = browser_stats_projection(rows)
        self.assertEqual(projection["sampleCount"], 90)
        self.assertEqual(projection["deltaTotals"]["framesDecodedDelta"], 5400)
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "trace.jsonl"
            path.write_text("\n".join(json.dumps(row) for row in rows + [rows[-1]]) + "\n{partial", encoding="utf-8")
            parsed, loading = read_trace([path])
            self.assertEqual(len(parsed), len(rows))
            self.assertEqual(loading["malformedRows"], 1)
            self.assertEqual(loading["duplicateRows"], 1)

    def test_export_metadata_keeps_retention_and_missing_files_explicit(self):
        rows = healthy()
        rows.insert(0, {"event": "traceExportManifest", "payload": {"traceRunId": "fixture", "partialBeginning": True, "missingFiles": 0, "truncatedRows": 0, "flushStatus": "flushed", "cutoffSequence": 1000, "files": []}})
        report = build_health_report(rows)
        self.assertEqual(report["gate"], "PASS")
        self.assertTrue(report["export"]["bundles"][0]["partialBeginning"])
        rows[0]["payload"]["missingFiles"] = 1
        self.assertEqual(build_health_report(rows)["gate"], "INSUFFICIENT_EVIDENCE")

    def test_existing_behavior_report_accepts_production_contract_with_handshake(self):
        path = Path(__file__).resolve().parents[2] / ".agents/skills/analyze-runtime-logs/scripts/trace_browser_webrtc_behavior_report.py"
        spec = importlib.util.spec_from_file_location("production_behavior_fixture", path)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        rows = healthy()
        for stage in ("localOfferAfterPatch", "remoteAnswer"):
            rows.append({"event": "browserWebRtcSdpObserved", "payload": {"stage": stage, "ssrcCount": 2, "h264Payloads": [{"payloadType": "102", "profileLevelId": "4d0033", "packetizationMode": "1"}]}})
        for kind in ("remoteAnswerSet", "connectionStateChanged", "firstInboundPacket", "firstDecoded", "firstKeyframeDecoded", "firstPresented"):
            rows.append({"event": "browserWebRtcTimelineObserved", "payload": {"kind": kind, "connectionState": "connected", "selectedPayloadType": "102", "selectedMimeType": "video/H264", "selectedProfileLevelId": "4d0033"}})
        with tempfile.TemporaryDirectory() as directory:
            trace = Path(directory) / "trace.jsonl"
            trace.write_text("\n".join(json.dumps(row) for row in rows), encoding="utf-8")
            report = module.build_report(trace, max_age_seconds=None, require_presented=True, require_stability=True)
            self.assertEqual(report["browserWebRtcBehaviorGate"], "PASS")
            self.assertEqual(report["productionPlaybackHealth"]["gate"], "PASS")
            self.assertEqual(report["selectedRemoteAnswer"]["profileFamily"], "high")
            self.assertEqual(report["sdp"]["byStage"]["remoteAnswer"]["videoSsrcCount"], 2)


if __name__ == "__main__":
    unittest.main()
