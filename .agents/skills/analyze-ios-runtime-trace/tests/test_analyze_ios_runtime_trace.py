from __future__ import annotations

import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


SCRIPT = Path(__file__).parents[1] / "scripts" / "analyze_ios_runtime_trace.py"


def envelope(
    seq: int,
    event: str,
    *,
    domain: str = "test",
    category: str = "event",
    profile: str = "dev",
    operation_id: str | None = None,
    payload: dict | None = None,
) -> dict:
    values = {"platform": "ios", **(payload or {})}
    if operation_id is not None:
        values["operationId"] = operation_id
    return {
        "schemaVersion": 3,
        "seq": seq,
        "tsMs": 1_000 + seq,
        "traceMode": profile,
        "traceProfile": profile,
        "dimension": "core",
        "importance": "key",
        "category": category,
        "domain": domain,
        "event": event,
        "sessionId": "session-1",
        "payload": values,
    }


def write_trace(path: Path, rows: list[dict], truncated_tail: bytes | None = None) -> None:
    data = b"".join(
        json.dumps(row, separators=(",", ":"), ensure_ascii=False).encode() + b"\n"
        for row in rows
    )
    if truncated_tail is not None:
        data += truncated_tail
    path.write_bytes(data)


class AnalyzeIOSRuntimeTraceTests(unittest.TestCase):
    def run_analyzer(self, path: Path, *args: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [sys.executable, "-B", str(SCRIPT), str(path), *args],
            check=False,
            capture_output=True,
            text=True,
        )

    def test_healthy_cached_library_trace_passes_all_flow_gate(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "runtime-trace-ios-1000-1.jsonl"
            rows = [
                envelope(
                    1,
                    "fileOpened",
                    domain="trace",
                    category="state",
                    payload={"maxFileBytes": 16_384, "maxFiles": 2},
                ),
                envelope(2, "appLaunchStarted", domain="ios-app"),
                envelope(3, "authRestoreStarted", domain="auth", operation_id="auth-1"),
                envelope(4, "authRestoreSucceeded", domain="auth", operation_id="auth-1"),
                envelope(5, "libraryPageAppeared", domain="library-ui"),
                envelope(6, "cacheRestoreStarted", domain="cloud-library", operation_id="cache-1"),
                envelope(7, "cacheRestoreHit", domain="cloud-library", operation_id="cache-1"),
                envelope(8, "catalogActivationSkipped", domain="cloud-library"),
                envelope(9, "contentPresented", domain="library-ui"),
            ]
            write_trace(path, rows, b'{"schemaVersion":3')

            result = self.run_analyzer(path, "--strict", "--require-flow", "all")
            report = json.loads(result.stdout)

            self.assertEqual(result.returncode, 0, result.stdout)
            self.assertEqual(report["gate"], "PASS")
            self.assertEqual(len(report["schema"]["truncatedTails"]), 1)
            self.assertTrue(report["budget"]["rawFiles"][0]["withinBudget"])

    def test_budget_sequence_and_privacy_violations_fail(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "runtime-trace-ios-1000-1.jsonl"
            rows = [
                envelope(
                    2,
                    "fileOpened",
                    domain="trace",
                    category="state",
                    payload={"maxFileBytes": 10, "maxFiles": 1},
                ),
                envelope(
                    1,
                    "sample",
                    payload={
                        "refreshToken": "secret-value",
                        "message": "https://example.invalid/path",
                    },
                ),
            ]
            write_trace(path, rows)

            result = self.run_analyzer(path, "--strict")
            report = json.loads(result.stdout)

            self.assertEqual(result.returncode, 2)
            self.assertEqual(report["gate"], "FAIL")
            self.assertGreater(report["failures"]["budgetViolations"], 0)
            self.assertGreater(report["failures"]["sequenceViolations"], 0)
            self.assertGreater(report["failures"]["privacyViolations"], 0)

    def test_file_ids_are_sorted_numerically(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            write_trace(root / "runtime-trace-ios-1000-10.jsonl", [envelope(10, "sample")])
            write_trace(root / "runtime-trace-ios-1000-2.jsonl", [envelope(2, "sample")])

            result = self.run_analyzer(root)
            report = json.loads(result.stdout)

            self.assertEqual(result.returncode, 0)
            self.assertEqual(
                [Path(value).name for value in report["scope"]["files"]],
                ["runtime-trace-ios-1000-2.jsonl", "runtime-trace-ios-1000-10.jsonl"],
            )


if __name__ == "__main__":
    unittest.main()
