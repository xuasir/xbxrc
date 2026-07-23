from __future__ import annotations

import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


SCRIPT = Path(__file__).parents[1] / "check-streaming-core-trace.py"


def envelope(
    seq: int,
    event: str,
    *,
    attempt_id: str | None = "attempt-1",
    generation: int | None = 3,
    peer_epoch: int | None = None,
    payload: dict | None = None,
) -> dict:
    values = {"platform": "ios", **(payload or {})}
    if attempt_id is not None:
        values["attemptId"] = attempt_id
    if generation is not None:
        values["generation"] = generation
    if peer_epoch is not None:
        values["peerEpoch"] = peer_epoch
    return {
        "schemaVersion": 3,
        "seq": seq,
        "tsMs": 1_000 + seq,
        "traceMode": "dev",
        "traceProfile": "dev",
        "dimension": "core",
        "importance": "key",
        "category": "state",
        "domain": "ios-streaming",
        "event": event,
        "sessionId": "session-1",
        "payload": values,
    }


def channel_profiles() -> dict:
    return {
        "channelCount": 4,
        "profiles": [
            {"label": "input", "protocol": "1.0", "ordered": True},
            {"label": "control", "protocol": "controlV1", "ordered": True},
            {"label": "chat", "protocol": "chatV1", "ordered": True},
            {"label": "message", "protocol": "messageV1", "ordered": True},
        ],
    }


def healthy_rows(
    *,
    peer_epoch: int = 7,
    remote_candidate_from_snapshot: bool = False,
) -> list[dict]:
    rows = [
        envelope(1, "streamLaunchStarted", payload={"target": "cloud"}),
        envelope(2, "sessionReady"),
        envelope(3, "answerApplied", peer_epoch=peer_epoch),
        envelope(4, "localIceStarted", peer_epoch=peer_epoch),
        envelope(5, "localIceCompleted", peer_epoch=peer_epoch),
        envelope(
            6,
            "remoteIceBatchReceived",
            peer_epoch=peer_epoch,
            payload={"candidateCount": 1, "endOfCandidates": False},
        ),
    ]
    if remote_candidate_from_snapshot:
        rows.append(
            envelope(
                6,
                "rtcHealthSnapshot",
                peer_epoch=peer_epoch,
                payload={"selectedRemoteCandidateType": "relay"},
            )
        )
    else:
        rows.append(
            envelope(
                6,
                "remoteIceBatchApplied",
                peer_epoch=peer_epoch,
                payload={"candidateCount": 1},
            )
        )
    rows.extend(
        [
            envelope(7, "remoteIceCompleted", peer_epoch=peer_epoch),
            envelope(8, "peerConnected", peer_epoch=peer_epoch),
            envelope(
                9,
                "dataChannelProfilesCreated",
                peer_epoch=peer_epoch,
                payload=channel_profiles(),
            ),
            envelope(10, "controlBootstrapPreHandshakeCompleted", peer_epoch=peer_epoch),
            envelope(10, "messageHandshakeSent", peer_epoch=peer_epoch),
            envelope(11, "messageHandshakeAcked", peer_epoch=peer_epoch),
            envelope(12, "messagePostHandshakeCompleted", peer_epoch=peer_epoch),
        ]
    )
    rows.append(envelope(13, "controlBootstrapCompleted", peer_epoch=peer_epoch))
    rows.extend(
        [
            envelope(14, "controlReady", peer_epoch=peer_epoch),
            envelope(15, "firstVideoFrame", peer_epoch=peer_epoch),
            envelope(
                16,
                "streamingStateChanged",
                payload={"state": "playing"},
            ),
            envelope(17, "steadyMediaObserved", peer_epoch=peer_epoch),
            envelope(18, "videoSurfaceAttached", peer_epoch=peer_epoch),
            envelope(
                19,
                "videoSurfaceSized",
                peer_epoch=peer_epoch,
                payload={"width": 1_920.0, "height": 1_080.0},
            ),
            envelope(
                20,
                "videoSurfaceRendererReady",
                peer_epoch=peer_epoch,
                payload={"frameWidth": 1_920.0, "frameHeight": 1_080.0},
            ),
            envelope(21, "terminalSelected"),
            envelope(22, "iceTasksCancelled"),
            envelope(23, "peerClosed"),
            envelope(24, "remoteSessionClosed"),
            envelope(25, "accessReleased"),
        ]
    )
    for seq, row in enumerate(rows, start=1):
        row["seq"] = seq
        row["tsMs"] = 1_000 + seq
    return rows


def write_trace(path: Path, rows: list[dict]) -> None:
    path.write_text(
        "".join(
            f"{json.dumps(row, separators=(',', ':'), ensure_ascii=False)}\n"
            for row in rows
        ),
        encoding="utf-8",
    )


class CheckStreamingCoreTraceTests(unittest.TestCase):
    def run_gate(self, path: Path, *args: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [sys.executable, "-B", str(SCRIPT), str(path), *args],
            check=False,
            capture_output=True,
            text=True,
        )

    def test_healthy_streaming_core_trace_passes(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "runtime-trace-ios-1000-1.jsonl"
            write_trace(path, healthy_rows())

            result = self.run_gate(path, "--strict")
            report = json.loads(result.stdout)

            self.assertEqual(result.returncode, 0, result.stdout)
            self.assertEqual(report["gate"], "PASS")
            attempt = report["streamingCore"]["attempts"][0]
            self.assertEqual(attempt["completePeerEpochs"], [7])
            self.assertEqual(attempt["playingStates"][0]["peerEpoch"], 7)
            control = attempt["peerEpochs"][0]["checks"]["controlReady"]
            self.assertFalse(control["details"]["legacyFallback"])
            self.assertEqual(control["details"]["ignoredLegacyCount"], 1)

    def test_selected_remote_candidate_is_application_evidence(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "runtime-trace-ios-1000-1.jsonl"
            write_trace(path, healthy_rows(remote_candidate_from_snapshot=True))

            result = self.run_gate(path, "--strict")
            report = json.loads(result.stdout)

            self.assertEqual(result.returncode, 0, result.stdout)
            remote = report["streamingCore"]["attempts"][0]["peerEpochs"][0][
                "checks"
            ]["remoteIceApplied"]
            self.assertEqual(remote["details"]["sources"], ["selectedRemoteCandidate"])

    def test_full_core_chain_must_share_one_peer_epoch(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "runtime-trace-ios-1000-1.jsonl"
            rows = healthy_rows()
            for row in rows:
                if row["event"] in {
                    "firstVideoFrame",
                    "steadyMediaObserved",
                    "videoSurfaceAttached",
                    "videoSurfaceSized",
                    "videoSurfaceRendererReady",
                }:
                    row["payload"]["peerEpoch"] = 8
            write_trace(path, rows)

            result = self.run_gate(path, "--strict")
            report = json.loads(result.stdout)

            self.assertEqual(result.returncode, 2)
            attempt = report["streamingCore"]["attempts"][0]
            self.assertEqual(attempt["completePeerEpochs"], [])
            self.assertIn(
                "no-complete-peer-epoch",
                {violation["type"] for violation in attempt["violations"]},
            )

    def test_missing_presentation_context_is_diagnostic(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "runtime-trace-ios-1000-1.jsonl"
            rows = healthy_rows()
            for row in rows:
                if row["event"].startswith("videoSurface"):
                    row["payload"].pop("attemptId")
                    row["payload"].pop("generation")
                    row["payload"].pop("peerEpoch")
            write_trace(path, rows)

            result = self.run_gate(path, "--strict")
            report = json.loads(result.stdout)

            self.assertEqual(result.returncode, 2)
            violations = report["streamingCore"]["contextViolations"]
            self.assertEqual(len(violations), 3)
            self.assertEqual(
                {violation["event"] for violation in violations},
                {
                    "videoSurfaceAttached",
                    "videoSurfaceSized",
                    "videoSurfaceRendererReady",
                },
            )

    def test_playing_allows_two_row_event_sink_reorder(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "runtime-trace-ios-1000-1.jsonl"
            rows = healthy_rows()
            playing = next(row for row in rows if row["event"] == "streamingStateChanged")
            rows.remove(playing)
            control_ready_index = next(
                index for index, row in enumerate(rows) if row["event"] == "controlReady"
            )
            rows.insert(control_ready_index, playing)
            for seq, row in enumerate(rows, start=1):
                row["seq"] = seq
                row["tsMs"] = 1_000 + seq
            write_trace(path, rows)

            result = self.run_gate(path, "--strict")
            report = json.loads(result.stdout)

            self.assertEqual(result.returncode, 0, result.stdout)
            state = report["streamingCore"]["attempts"][0]["playingStates"][0]
            self.assertTrue(state["reordered"])
            self.assertEqual(state["reorderRows"], 2)

    def test_legacy_control_bootstrap_is_ready_fallback(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "runtime-trace-ios-1000-1.jsonl"
            rows = [row for row in healthy_rows() if row["event"] != "controlReady"]
            write_trace(path, rows)

            result = self.run_gate(path, "--strict")
            report = json.loads(result.stdout)

            self.assertEqual(result.returncode, 0, result.stdout)
            control = report["streamingCore"]["attempts"][0]["peerEpochs"][0][
                "checks"
            ]["controlReady"]
            self.assertTrue(control["details"]["legacyFallback"])

    def test_post_handshake_control_anchors_are_required(self) -> None:
        required = {
            "messageHandshakeSent": "messageHandshakeSent",
            "messagePostHandshakeCompleted": "messagePostHandshakeCompleted",
            "controlBootstrapCompleted": "controlBootstrapCompleted",
        }
        for event, check_name in required.items():
            with self.subTest(event=event), tempfile.TemporaryDirectory() as directory:
                path = Path(directory) / "runtime-trace-ios-1000-1.jsonl"
                rows = [row for row in healthy_rows() if row["event"] != event]
                write_trace(path, rows)

                result = self.run_gate(path, "--strict")
                report = json.loads(result.stdout)

                self.assertEqual(result.returncode, 2)
                peer = report["streamingCore"]["attempts"][0]["peerEpochs"][0]
                self.assertIn(check_name, peer["failedChecks"])

    def test_terminal_must_be_unique(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "runtime-trace-ios-1000-1.jsonl"
            rows = healthy_rows()
            terminal_index = next(
                index for index, row in enumerate(rows) if row["event"] == "terminalSelected"
            )
            rows.insert(terminal_index, envelope(1, "terminalSelected"))
            for seq, row in enumerate(rows, start=1):
                row["seq"] = seq
                row["tsMs"] = 1_000 + seq
            write_trace(path, rows)

            result = self.run_gate(path, "--strict")
            report = json.loads(result.stdout)

            self.assertEqual(result.returncode, 2)
            violations = report["streamingCore"]["attempts"][0]["violations"]
            terminal = next(
                violation
                for violation in violations
                if violation.get("anchor") == "terminalSelected"
            )
            self.assertEqual(terminal["observed"], 2)

    def test_core_cleanup_must_be_complete(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "runtime-trace-ios-1000-1.jsonl"
            rows = [row for row in healthy_rows() if row["event"] != "peerClosed"]
            write_trace(path, rows)

            result = self.run_gate(path, "--strict")
            report = json.loads(result.stdout)

            self.assertEqual(result.returncode, 2)
            violations = report["streamingCore"]["attempts"][0]["violations"]
            cleanup = next(
                violation for violation in violations if violation.get("anchor") == "peerClosed"
            )
            self.assertEqual(cleanup["type"], "streaming-cleanup-count")

    def test_attempt_generation_must_stay_stable(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "runtime-trace-ios-1000-1.jsonl"
            rows = healthy_rows()
            playing = next(row for row in rows if row["event"] == "streamingStateChanged")
            playing["payload"]["generation"] = 4
            write_trace(path, rows)

            result = self.run_gate(path, "--strict")
            report = json.loads(result.stdout)

            self.assertEqual(result.returncode, 2)
            conflicts = report["streamingCore"]["generationViolations"]
            self.assertEqual(conflicts[0]["generations"], [3, 4])

    def test_renderer_ready_requires_positive_frame_dimensions(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "runtime-trace-ios-1000-1.jsonl"
            rows = healthy_rows()
            renderer = next(row for row in rows if row["event"] == "videoSurfaceRendererReady")
            renderer["payload"]["frameWidth"] = 0.0
            write_trace(path, rows)

            result = self.run_gate(path, "--strict")
            report = json.loads(result.stdout)

            self.assertEqual(result.returncode, 2)
            peer = report["streamingCore"]["attempts"][0]["peerEpochs"][0]
            self.assertIn("videoSurfaceRendererReady", peer["failedChecks"])


if __name__ == "__main__":
    unittest.main()
