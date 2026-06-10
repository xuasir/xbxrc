import json
import os
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[4]
SCRIPT_PATH = (
    REPO_ROOT
    / ".agents/skills/analyze-runtime-logs/scripts/trace_receive_feedback_report.py"
)
WEBRTC_GATE_PATH = (
    REPO_ROOT
    / ".agents/skills/analyze-runtime-logs/scripts/trace_webrtc_acceptance_gate.py"
)
MIDSEGMENT_PATH = (
    REPO_ROOT
    / ".agents/skills/analyze-runtime-logs/scripts/trace_midsegment_report.py"
)


def write_trace(rows: list[dict]) -> Path:
    with tempfile.NamedTemporaryFile("w", suffix=".jsonl", delete=False) as handle:
        for row in rows:
            handle.write(json.dumps(row, ensure_ascii=False))
            handle.write("\n")
        return Path(handle.name)


def write_named_trace(trace_dir: Path, name: str, rows: list[dict]) -> Path:
    trace_path = trace_dir / name
    with trace_path.open("w") as handle:
        for row in rows:
            handle.write(json.dumps(row, ensure_ascii=False))
            handle.write("\n")
    return trace_path


def run_report(trace_path: Path, *args: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [sys.executable, "-B", str(SCRIPT_PATH), *args, str(trace_path)],
        capture_output=True,
        text=True,
        check=False,
    )


def run_webrtc_gate(trace_path: Path, *args: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [sys.executable, "-B", str(WEBRTC_GATE_PATH), *args, str(trace_path)],
        capture_output=True,
        text=True,
        check=False,
    )


def run_midsegment(trace_path: Path, *args: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [sys.executable, "-B", str(MIDSEGMENT_PATH), str(trace_path), *args],
        capture_output=True,
        text=True,
        check=False,
    )


class TraceReceiveFeedbackReportTest(unittest.TestCase):
    @staticmethod
    def combined_gate_pass_rows() -> list[dict]:
        return [
            {
                "seq": 1,
                "tsMs": 1_000,
                "event": "statsSnapshot",
                "payload": {
                    "stats": {
                        "hostFramePresentEpoch": 1,
                        "sessionPhase": "steady",
                        "videoOwnerState": "stable",
                        "video_decoder_recovery_state": "nominal",
                    }
                },
            },
            {
                "seq": 2,
                "tsMs": 1_100,
                "event": "keyframeRequestOutcome",
                "payload": {"outcome": "sent", "ledgerGeneration": 7},
            },
            {
                "seq": 3,
                "tsMs": 1_120,
                "event": "h264InspectionObserved",
                "payload": {
                    "isIdr": True,
                    "bootstrapReady": True,
                    "ledgerGeneration": 7,
                },
            },
            {
                "seq": 4,
                "tsMs": 1_140,
                "event": "pictureRecoveryTransition",
                "payload": {
                    "toPhase": "CleanAnchorCommitted",
                    "ledgerGeneration": 7,
                },
            },
            {
                "seq": 5,
                "tsMs": 1_150,
                "event": "receiveFeedbackDecision",
                "payload": {
                    "action": "none",
                    "coalescing": "none",
                    "keyframeRequired": False,
                    "responseState": "usable-idr",
                    "receiveDisplayState": "display-stable",
                    "ledgerGeneration": 7,
                },
            },
            {
                "seq": 6,
                "tsMs": 1_160,
                "event": "pictureRecoveryTransition",
                "payload": {"toPhase": "DisplayStable", "ledgerGeneration": 7},
            },
            {
                "seq": 7,
                "tsMs": 80_000,
                "event": "statsSnapshot",
                "payload": {
                    "stats": {
                        "sessionPhase": "steady",
                        "submitAgeMs": 10.0,
                        "presentAgeMs": 8.0,
                        "submitToPresentMs": 18.0,
                        "decodeFps": 30.0,
                        "presentFps": 30.0,
                        "hostFramePresentEpoch": 2,
                    }
                },
            },
            {
                "seq": 8,
                "tsMs": 80_010,
                "event": "hostMailboxTakeDecision",
                "payload": {"decision": "ready", "hasPendingFrame": False},
            },
            {
                "seq": 9,
                "tsMs": 90_000,
                "event": "statsSnapshot",
                "payload": {
                    "stats": {
                        "sessionPhase": "steady",
                        "submitAgeMs": 12.0,
                        "presentAgeMs": 9.0,
                        "submitToPresentMs": 20.0,
                        "decodeFps": 30.0,
                        "presentFps": 29.0,
                        "hostFramePresentEpoch": 3,
                    }
                },
            },
        ]

    def test_strict_gate_passes_when_receive_media_and_display_close(self) -> None:
        trace_path = write_trace(
            [
                {
                    "seq": 1,
                    "tsMs": 100,
                    "event": "keyframeRequestOutcome",
                    "payload": {"outcome": "sent", "ledgerGeneration": 7},
                },
                {
                    "seq": 2,
                    "tsMs": 120,
                    "event": "h264InspectionObserved",
                    "payload": {
                        "isIdr": True,
                        "bootstrapReady": True,
                        "ledgerGeneration": 7,
                    },
                },
                {
                    "seq": 3,
                    "tsMs": 140,
                    "event": "pictureRecoveryTransition",
                    "payload": {
                        "toPhase": "CleanAnchorCommitted",
                        "ledgerGeneration": 7,
                    },
                },
                {
                    "seq": 4,
                    "tsMs": 150,
                    "event": "receiveFeedbackDecision",
                    "payload": {
                        "action": "none",
                        "coalescing": "none",
                        "keyframeRequired": False,
                        "responseState": "usable-idr",
                        "receiveDisplayState": "display-stable",
                        "ledgerGeneration": 7,
                    },
                },
                {
                    "seq": 5,
                    "tsMs": 160,
                    "event": "pictureRecoveryTransition",
                    "payload": {"toPhase": "DisplayStable", "ledgerGeneration": 7},
                },
            ]
        )
        try:
            result = run_report(
                trace_path,
                "--fail-on-gate",
                "--require-media-recovered",
                "--require-display-stable",
            )
        finally:
            trace_path.unlink(missing_ok=True)

        self.assertEqual(result.returncode, 0, result.stderr)
        report = json.loads(result.stdout)
        self.assertEqual(report["receiveFeedbackGate"], "PASS")
        self.assertEqual(report["keyframeChain"]["cleanAnchorCommitted"], 1)
        self.assertEqual(report["keyframeChain"]["displayStable"], 1)

    def test_strict_gate_accepts_clean_anchor_as_response_evidence(self) -> None:
        trace_path = write_trace(
            [
                {
                    "seq": 1,
                    "tsMs": 100,
                    "event": "keyframeRequestOutcome",
                    "payload": {"outcome": "sent", "ledgerGeneration": 7},
                },
                {
                    "seq": 2,
                    "tsMs": 140,
                    "event": "pictureRecoveryTransition",
                    "payload": {
                        "toPhase": "CleanAnchorCommitted",
                        "ledgerGeneration": 7,
                    },
                },
                {
                    "seq": 3,
                    "tsMs": 160,
                    "event": "stableServingSettled",
                    "payload": {
                        "keyframeRequired": False,
                        "responseState": "usable-idr",
                        "receiveDisplayState": "display-stable",
                        "ledgerGeneration": 7,
                    },
                },
            ]
        )
        try:
            result = run_report(
                trace_path,
                "--fail-on-gate",
                "--require-media-recovered",
                "--require-display-stable",
            )
        finally:
            trace_path.unlink(missing_ok=True)

        self.assertEqual(result.returncode, 0, result.stderr)
        report = json.loads(result.stdout)
        self.assertEqual(report["receiveFeedbackGate"], "PASS")
        self.assertEqual(report["keyframeChain"]["responseObserved"], 1)
        self.assertEqual(report["keyframeChain"]["cleanAnchorCommitted"], 1)
        self.assertEqual(report["keyframeChain"]["displayStable"], 1)

    def test_reports_local_control_keyframe_request_catalog(self) -> None:
        trace_path = write_trace(
            [
                {
                    "seq": 1,
                    "tsMs": 100,
                    "event": "channelMessageCatalog",
                    "payload": {
                        "direction": "local",
                        "channel": "control",
                        "kindMessage": "videoKeyframeRequested",
                    },
                }
            ]
        )
        try:
            result = run_report(trace_path)
        finally:
            trace_path.unlink(missing_ok=True)

        self.assertEqual(result.returncode, 0, result.stderr)
        report = json.loads(result.stdout)
        self.assertEqual(
            report["controlKeyframeRequests"]["localControlVideoKeyframeRequested"],
            1,
        )

    def test_strict_gate_fails_when_receive_gate_fails(self) -> None:
        trace_path = write_trace(
            [
                {
                    "seq": 1,
                    "tsMs": 100,
                    "event": "receiveFeedbackDecision",
                    "payload": {
                        "action": "requestFir",
                        "coalescing": "fresh-sent",
                        "arbiterMismatchTotal": 1,
                    },
                }
            ]
        )
        try:
            result = run_report(trace_path, "--fail-on-gate")
        finally:
            trace_path.unlink(missing_ok=True)

        self.assertEqual(result.returncode, 2)
        report = json.loads(result.stdout)
        self.assertEqual(report["receiveFeedbackGate"], "FAIL")
        self.assertIn("arbiterMismatchTotal", report["receiveFeedbackGateFailures"])

    def test_insert_gate_allows_receive_steady_action_stage(self) -> None:
        trace_path = write_trace(
            [
                {
                    "seq": 1,
                    "tsMs": 100,
                    "event": "insertGateDecision",
                    "payload": {
                        "decision": "emit",
                        "keyframeRequired": False,
                        "packetRecoveryActionStage": "steady",
                        "referenceState": "continuous",
                    },
                }
            ]
        )
        try:
            result = run_report(trace_path)
        finally:
            trace_path.unlink(missing_ok=True)

        self.assertEqual(result.returncode, 0)
        report = json.loads(result.stdout)
        self.assertEqual(report["insertSurfacePhaseActionStage"], 0)
        self.assertEqual(report["receiveFeedbackGate"], "PASS")

    def test_insert_gate_fails_when_surface_phase_leaks_into_action_stage(self) -> None:
        trace_path = write_trace(
            [
                {
                    "seq": 1,
                    "tsMs": 100,
                    "event": "insertGateDecision",
                    "payload": {
                        "decision": "holdRepair",
                        "keyframeRequired": False,
                        "packetRecoveryActionStage": "supply-break",
                        "referenceState": "continuous",
                    },
                }
            ]
        )
        try:
            result = run_report(trace_path, "--fail-on-gate")
        finally:
            trace_path.unlink(missing_ok=True)

        self.assertEqual(result.returncode, 2)
        report = json.loads(result.stdout)
        self.assertEqual(report["insertSurfacePhaseActionStage"], 1)
        self.assertIn("insertSurfacePhaseActionStage", report["receiveFeedbackGateFailures"])

    def test_combined_webrtc_gate_passes_receive_and_midsegment(self) -> None:
        trace_path = write_trace(self.combined_gate_pass_rows())
        try:
            result = run_webrtc_gate(trace_path)
        finally:
            trace_path.unlink(missing_ok=True)

        self.assertEqual(result.returncode, 0, result.stderr)
        report = json.loads(result.stdout)
        self.assertEqual(report["acceptanceGate"], "PASS")
        self.assertEqual(report["receive"]["exitCode"], 0)
        self.assertEqual(report["midsegment"]["exitCode"], 0)
        self.assertEqual(report["midsegment"]["globalLatencyGate"], "PASS")
        self.assertEqual(report["midsegment"]["mediaSupplyGate"], "PASS")
        self.assertEqual(report["midsegment"]["steadySupplyGate"], "PASS")

    def test_midsegment_auto_window_anchors_at_first_steady_snapshot(self) -> None:
        rows = [
            {"seq": 1, "tsMs": 1_000, "event": "traceStarted", "payload": {}},
            {
                "seq": 2,
                "tsMs": 80_000,
                "event": "statsSnapshot",
                "payload": {
                    "stats": {
                        "sessionPhase": "connecting",
                        "submitAgeMs": 0.0,
                        "presentAgeMs": 0.0,
                        "submitToPresentMs": 0.0,
                        "decodeFps": 0.0,
                        "presentFps": 0.0,
                        "hostFramePresentEpoch": 1,
                    }
                },
            },
            {
                "seq": 3,
                "tsMs": 85_000,
                "event": "statsSnapshot",
                "payload": {
                    "stats": {
                        "sessionPhase": "connecting",
                        "submitAgeMs": 0.0,
                        "presentAgeMs": 0.0,
                        "submitToPresentMs": 0.0,
                        "decodeFps": 0.0,
                        "presentFps": 0.0,
                        "hostFramePresentEpoch": 1,
                    }
                },
            },
            {
                "seq": 4,
                "tsMs": 100_000,
                "event": "statsSnapshot",
                "payload": {
                    "stats": {
                        "sessionPhase": "steady",
                        "submitAgeMs": 10.0,
                        "presentAgeMs": 8.0,
                        "submitToPresentMs": 18.0,
                        "decodeFps": 30.0,
                        "presentFps": 30.0,
                        "hostFramePresentEpoch": 2,
                        "hostMailboxEnqueueCountTotal": 10,
                    }
                },
            },
            {
                "seq": 5,
                "tsMs": 110_000,
                "event": "statsSnapshot",
                "payload": {
                    "stats": {
                        "sessionPhase": "steady",
                        "submitAgeMs": 11.0,
                        "presentAgeMs": 8.0,
                        "submitToPresentMs": 19.0,
                        "decodeFps": 30.0,
                        "presentFps": 30.0,
                        "hostFramePresentEpoch": 12,
                        "hostMailboxEnqueueCountTotal": 20,
                    }
                },
            },
        ]
        trace_path = write_trace(rows)
        try:
            result = run_midsegment(trace_path)
        finally:
            trace_path.unlink(missing_ok=True)

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("source=auto-steady", result.stdout)
        self.assertIn("session_phase steady ratio: 100.0%", result.stdout)
        self.assertIn("STEADY_SUPPLY_GATE: PASS", result.stdout)

    def test_combined_webrtc_gate_selects_latest_trace(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            trace_dir = Path(temp_dir)
            older = write_named_trace(
                trace_dir,
                "runtime-trace-1-1.jsonl",
                [{"seq": 1, "tsMs": 1_000, "event": "receiveFeedbackDecision", "payload": {}}],
            )
            latest = write_named_trace(
                trace_dir,
                "runtime-trace-2-1.jsonl",
                self.combined_gate_pass_rows(),
            )
            old_time = time.time() - 60.0
            os.utime(older, (old_time, old_time))
            now = time.time()
            os.utime(latest, (now, now))

            result = run_webrtc_gate(
                "--latest",
                "--runtime-log-dir",
                str(trace_dir),
            )

        self.assertEqual(result.returncode, 0, result.stderr)
        report = json.loads(result.stdout)
        self.assertTrue(report["trace"].endswith("runtime-trace-2-1.jsonl"))
        self.assertEqual(report["traceFreshness"]["freshnessGate"], "PASS")

    def test_combined_webrtc_gate_fails_stale_latest_trace(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            trace_dir = Path(temp_dir)
            trace_path = write_named_trace(
                trace_dir,
                "runtime-trace-1-1.jsonl",
                self.combined_gate_pass_rows(),
            )
            old_time = time.time() - 120.0
            os.utime(trace_path, (old_time, old_time))

            result = run_webrtc_gate(
                "--latest",
                "--runtime-log-dir",
                str(trace_dir),
                "--max-age-seconds",
                "10",
            )

        self.assertEqual(result.returncode, 2)
        report = json.loads(result.stdout)
        self.assertEqual(report["acceptanceGate"], "FAIL")
        self.assertEqual(report["traceFreshness"]["freshnessGate"], "FAIL")


if __name__ == "__main__":
    unittest.main()
