import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[4]
SCRIPT_PATH = REPO_ROOT / ".agents/skills/analyze-runtime-logs/scripts/summarize_runtime_trace.py"
def summarize_trace(trace_path: Path) -> dict:
    result = subprocess.run(
        [
            sys.executable,
            str(SCRIPT_PATH),
            str(trace_path),
            "--json",
            "--exclude-categories",
            "log",
        ],
        check=True,
        capture_output=True,
        text=True,
    )
    return json.loads(result.stdout)


def summarize_rows(rows: list[dict]) -> dict:
    with tempfile.NamedTemporaryFile("w", suffix=".jsonl", delete=False) as handle:
        for row in rows:
            handle.write(json.dumps(row, ensure_ascii=False))
            handle.write("\n")
        temp_path = Path(handle.name)
    try:
        return summarize_trace(temp_path)
    finally:
        temp_path.unlink(missing_ok=True)


class SummarizeRuntimeTraceBlackboxTest(unittest.TestCase):
    def test_schema_v3_profile_dimension_importance_counts_are_reported(self) -> None:
        summary = summarize_rows(
            [
                {
                    "schemaVersion": 3,
                    "seq": 1,
                    "tsMs": 100,
                    "traceMode": "production",
                    "traceProfile": "production",
                    "dimension": "recovery",
                    "importance": "key",
                    "category": "decision",
                    "domain": "xbxengine",
                    "event": "recoveryDecisionLedger",
                    "payload": {"gateResult": "accepted"},
                },
                {
                    "schemaVersion": 3,
                    "seq": 2,
                    "tsMs": 120,
                    "traceMode": "production",
                    "traceProfile": "production",
                    "dimension": "core",
                    "importance": "essential",
                    "category": "state",
                    "domain": "trace",
                    "event": "traceBudgetNotice",
                    "payload": {
                        "reason": "writerQueuePressure",
                        "debugDropped": 3,
                        "rawDropped": 7,
                    },
                },
            ]
        )
        counts = summary["counts"]

        self.assertEqual(counts["traceProfiles"]["production"], 2)
        self.assertEqual(counts["traceModes"]["production"], 2)
        self.assertEqual(counts["dimensions"]["recovery"], 1)
        self.assertEqual(counts["dimensions"]["core"], 1)
        self.assertEqual(counts["importance"]["key"], 1)
        self.assertEqual(counts["importance"]["essential"], 1)
        self.assertEqual(counts["traceBudgetNotices"], 1)

    def test_first_frame_health_keeps_continuation_seen_phase(self) -> None:
        summary = summarize_rows(
            [
                {
                    "seq": 1,
                    "tsMs": 100,
                    "category": "event",
                    "domain": "rtc",
                    "event": "firstFrameLatencyObserved",
                    "payload": {
                        "episodeId": 906,
                        "recoveryEpoch": 12,
                        "controlReadyToPliSentMs": 20.0,
                        "terminalPhase": "ContinuationSeen",
                        "incompleteReason": "continuationOnlyAwaitingIdr",
                    },
                }
            ]
        )
        first_frame = summary["recoveryAudit"]["firstFrameHealth"]

        self.assertEqual(first_frame["observationCount"], 1)
        self.assertEqual(first_frame["terminalPhaseCounts"]["ContinuationSeen"], 1)
        self.assertEqual(
            first_frame["incompleteReasonCounts"]["continuationOnlyAwaitingIdr"], 1
        )
        self.assertEqual(first_frame["controlReadyToPliSentMsCount"], 1)

    def test_control_plane_health_counts_feedback_availability_transitions_by_semantics(self) -> None:
        summary = summarize_rows(
            [
                {
                    "seq": 1,
                    "tsMs": 100,
                    "category": "event",
                    "domain": "rtc",
                    "event": "feedbackTargetAvailabilityChanged",
                    "payload": {
                        "target": "videoRtcpFeedback",
                        "state": "ready",
                        "reason": "feedbackTargetBound",
                    },
                },
                {
                    "seq": 2,
                    "tsMs": 120,
                    "category": "event",
                    "domain": "rtc",
                    "event": "feedbackTargetAvailabilityChanged",
                    "payload": {
                        "target": "videoRtcpFeedback",
                        "state": "unbound",
                        "reason": "feedbackTargetUnbound",
                    },
                },
                {
                    "seq": 3,
                    "tsMs": 150,
                    "category": "event",
                    "domain": "rtc",
                    "event": "feedbackTargetAvailabilityChanged",
                    "payload": {
                        "target": "videoRtcpFeedback",
                        "state": "ready",
                        "reason": "feedbackTargetBound",
                    },
                },
            ]
        )
        control_plane = summary["recoveryAudit"]["controlPlaneHealth"]

        self.assertEqual(control_plane["feedbackTargetAvailabilityChangedCount"], 3)
        self.assertEqual(control_plane["feedbackTargetStateCounts"]["ready"], 2)
        self.assertEqual(control_plane["feedbackTargetStateCounts"]["unbound"], 1)
        self.assertEqual(control_plane["feedbackTargetReasonCounts"]["feedbackTargetBound"], 2)
        self.assertEqual(
            control_plane["feedbackTargetReasonCounts"]["feedbackTargetUnbound"], 1
        )

    def test_bootstrap_health_counts_post_recovery_degradation_samples(self) -> None:
        summary = summarize_rows(
            [
                {
                    "seq": 1,
                    "tsMs": 100,
                    "category": "event",
                    "domain": "rtc",
                    "event": "h264InspectionObserved",
                    "payload": {
                        "boundEpisodeId": 43,
                        "admissionAccepted": True,
                        "isIdr": False,
                        "bootstrapRejectReason": "bootstrapMissingIdr",
                        "rejectClassification": "continuationAcceptedWhileAwaitingIdr",
                        "isPostRecoveryDegradation": True,
                    },
                },
                {
                    "seq": 2,
                    "tsMs": 120,
                    "category": "event",
                    "domain": "rtc",
                    "event": "h264InspectionObserved",
                    "payload": {
                        "boundEpisodeId": 44,
                        "admissionAccepted": True,
                        "isIdr": False,
                        "bootstrapRejectReason": "bootstrapMissingIdr",
                        "rejectClassification": "continuationAcceptedWhileAwaitingIdr",
                        "isPostRecoveryDegradation": False,
                    },
                },
            ]
        )
        bootstrap = summary["recoveryAudit"]["bootstrapHealth"]

        self.assertEqual(bootstrap["observationCount"], 2)
        self.assertEqual(bootstrap["postRecoveryDegradationCount"], 1)
        self.assertEqual(
            bootstrap["rejectClassificationCounts"]["continuationAcceptedWhileAwaitingIdr"], 2
        )

    def test_presentation_health_counts_host_stall_and_stale_frame_drops(self) -> None:
        summary = summarize_rows(
            [
                {
                    "seq": 1,
                    "tsMs": 100,
                    "category": "state",
                    "domain": "xbxengine",
                    "event": "hostMailboxState",
                    "payload": {
                        "cadencePhase": "steady",
                        "noPendingPressureLevel": "normal",
                        "displayedFrameStale": False,
                        "retainedOldFrameRisk": False,
                        "presentAgeMs": 12.0,
                    },
                },
                {
                    "seq": 2,
                    "tsMs": 160,
                    "category": "state",
                    "domain": "xbxengine",
                    "event": "hostMailboxState",
                    "payload": {
                        "cadencePhase": "starved",
                        "noPendingPressureLevel": "critical",
                        "displayedFrameStale": True,
                        "retainedOldFrameRisk": True,
                        "presentAgeMs": 486.0,
                    },
                },
                {
                    "seq": 3,
                    "tsMs": 170,
                    "category": "event",
                    "domain": "xbxengine",
                    "event": "frameDropped",
                    "payload": {
                        "reason": "dropBackpressure",
                        "stage": "render",
                        "detail": "scheduledFrameStale",
                        "frameRecoveryDisposition": "repairing",
                    },
                },
                {
                    "seq": 4,
                    "tsMs": 171,
                    "category": "event",
                    "domain": "xbxengine",
                    "event": "frameDeadlineMissed",
                    "payload": {
                        "reason": "dropLate",
                        "stage": "present",
                        "detail": "submittedFrameStale",
                        "frameRecoveryDisposition": "rebuilding-supply",
                    },
                },
            ]
        )
        presentation = summary["recoveryAudit"]["presentationHealth"]

        self.assertEqual(presentation["hostMailboxStateCount"], 2)
        self.assertEqual(presentation["displayedFrameStaleCount"], 1)
        self.assertEqual(presentation["retainedOldFrameRiskCount"], 1)
        self.assertEqual(presentation["cadencePhaseCounts"]["steady"], 1)
        self.assertEqual(presentation["cadencePhaseCounts"]["starved"], 1)
        self.assertEqual(presentation["noPendingPressureLevelCounts"]["normal"], 1)
        self.assertEqual(presentation["noPendingPressureLevelCounts"]["critical"], 1)
        self.assertEqual(presentation["frameDropEventCount"], 2)
        self.assertEqual(presentation["frameDropReasonCounts"]["dropBackpressure"], 1)
        self.assertEqual(presentation["frameDropReasonCounts"]["dropLate"], 1)
        self.assertEqual(presentation["frameDropStageCounts"]["render"], 1)
        self.assertEqual(presentation["frameDropStageCounts"]["present"], 1)
        self.assertEqual(presentation["frameDropDetailCounts"]["scheduledFrameStale"], 1)
        self.assertEqual(presentation["frameDropDetailCounts"]["submittedFrameStale"], 1)
        self.assertEqual(presentation["scheduledFrameStaleCount"], 1)
        self.assertEqual(presentation["submittedFrameStaleCount"], 1)
        self.assertEqual(presentation["recoveryValuedFrameDropCount"], 2)


if __name__ == "__main__":
    unittest.main()
