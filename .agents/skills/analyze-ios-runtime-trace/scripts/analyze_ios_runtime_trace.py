#!/usr/bin/env python3
"""Validate and summarize XBXRC iOS Runtime Trace JSONL files."""

from __future__ import annotations

import argparse
import json
import re
import sys
from collections import Counter, defaultdict
from pathlib import Path
from typing import Any, Iterable


REQUIRED_FIELDS = {
    "schemaVersion",
    "seq",
    "tsMs",
    "traceMode",
    "traceProfile",
    "dimension",
    "importance",
    "category",
    "domain",
    "event",
    "sessionId",
    "payload",
}
ALLOWED_PROFILES = {"production", "dev"}
ALLOWED_CATEGORIES = {"event", "decision", "state", "snapshot", "log"}
ALLOWED_DIMENSIONS = {
    "core",
    "lifecycle",
    "network",
    "recovery",
    "media_supply",
    "presentation",
    "input",
    "native_video",
    "frontend",
    "engine_log",
}
ALLOWED_IMPORTANCE = {"essential", "key", "debug", "raw"}
PROFILE_BUDGETS = {
    "production": {"maxFileBytes": 8 * 1024 * 1024, "maxFiles": 4},
    "dev": {"maxFileBytes": 32 * 1024 * 1024, "maxFiles": 6},
}
SENSITIVE_FRAGMENTS = (
    "token",
    "seed",
    "jwk",
    "handle",
    "oauth",
    "authorization",
    "callbackurl",
    "accountid",
    "xuid",
    "xid",
    "uhs",
    "refreshcode",
)
RAW_SECRET_PATTERNS = (
    ("http-url", re.compile(r"https?://", re.IGNORECASE)),
    ("callback-url", re.compile(r"ms-xal-", re.IGNORECASE)),
    ("bearer", re.compile(r"\bbearer\s+[A-Za-z0-9._~+/=-]+", re.IGNORECASE)),
    ("gs-token", re.compile(r"\bgs(?:token)?[=: ]+[A-Za-z0-9._~+/=-]+", re.IGNORECASE)),
    ("refresh-token", re.compile(r"refresh[_ -]?token[=: ]+[^\s,;}]+", re.IGNORECASE)),
    ("cloud-identity", re.compile(r"cloud-[0-9a-f]{16}", re.IGNORECASE)),
)
FINGERPRINT_RE = re.compile(r"^[0-9a-f]{16}$")

FLOW_REQUIREMENTS = {
    "startup": [
        ({"appLaunchStarted"}, "app launch"),
        ({"authRestoreStarted"}, "auth restore start"),
        ({"authRestoreSucceeded", "authRestoreFailed"}, "auth restore outcome"),
    ],
    "library": [
        ({"libraryPageAppeared"}, "library page"),
        ({"cacheRestoreStarted"}, "cache restore start"),
        (
            {
                "cacheRestoreHit",
                "cacheRestoreMiss",
                "cacheRestoreRejected",
                "cacheRestoreFailed",
                "cacheRestoreSkipped",
            },
            "cache restore outcome",
        ),
        ({"catalogRefreshStarted", "catalogActivationSkipped"}, "catalog activation decision"),
        ({"skeletonPresented", "contentPresented"}, "library presentation"),
    ],
}

PAIR_RULES = {
    "authRestoreStarted": {"authRestoreSucceeded", "authRestoreFailed"},
    "cloudAccessBoundaryStarted": {"cloudAccessBoundarySucceeded", "cloudAccessBoundaryFailed"},
    "catalogRefreshStarted": {
        "catalogRefreshCommitted",
        "catalogRefreshFailed",
        "catalogRefreshCancelled",
        "catalogRefreshDiscarded",
    },
    "metadataPageStarted": {
        "metadataPageCommitted",
        "metadataPageUnchanged",
        "metadataPageFailed",
        "metadataPageCancelled",
        "metadataPageDiscarded",
    },
    "imageCandidateStarted": {"imageCandidateSucceeded", "imageCandidateFailed"},
    "userRefreshRequested": {"userRefreshCompleted"},
    "gameLibraryBoundaryStarted": {"gameLibraryBoundarySucceeded", "gameLibraryBoundaryFailed"},
    "playtimesBoundaryStarted": {"playtimesBoundarySucceeded", "playtimesBoundaryFailed"},
    "achievementsBoundaryStarted": {"achievementsBoundarySucceeded", "achievementsBoundaryFailed"},
}


def trace_file_order(path: Path) -> tuple[int, int, str]:
    match = re.fullmatch(r"runtime-trace-ios-(\d+)-(\d+)\.jsonl", path.name)
    if match:
        return (int(match.group(1)), int(match.group(2)), path.name)
    return (sys.maxsize, sys.maxsize, path.name)


def discover_files(inputs: Iterable[str]) -> list[Path]:
    files: set[Path] = set()
    for raw in inputs:
        path = Path(raw).expanduser()
        if path.is_dir():
            files.update(path.glob("runtime-trace-ios-*.jsonl"))
            files.update(path.glob("XBXRC-iOS-Trace-*.jsonl"))
        elif path.is_file():
            files.add(path)
    return sorted(files, key=trace_file_order)


def normalized_key(value: str) -> str:
    return value.lower().replace("_", "").replace("-", "")


def is_raw_trace(path: Path) -> bool:
    return path.name.startswith("runtime-trace-ios-")


def scan_payload(
    value: Any,
    profile: str,
    file: Path,
    line_number: int,
    violations: list[dict[str, Any]],
    key: str | None = None,
) -> None:
    if key is not None:
        normalized = normalized_key(key)
        if any(fragment in normalized for fragment in SENSITIVE_FRAGMENTS):
            if not isinstance(value, bool) and value != "<redacted>":
                violations.append(
                    {"file": str(file), "line": line_number, "type": "sensitive-key", "field": key}
                )
        if normalized == "productid" and profile == "production":
            if not isinstance(value, str) or not FINGERPRINT_RE.fullmatch(value):
                violations.append(
                    {"file": str(file), "line": line_number, "type": "raw-product-id", "field": key}
                )
        if normalized in {"streamtitleid", "xboxtitleid"}:
            if not isinstance(value, str) or not FINGERPRINT_RE.fullmatch(value):
                violations.append(
                    {"file": str(file), "line": line_number, "type": "raw-title-id", "field": key}
                )

    if isinstance(value, dict):
        for child_key, child_value in value.items():
            scan_payload(child_value, profile, file, line_number, violations, child_key)
    elif isinstance(value, list):
        for child in value:
            scan_payload(child, profile, file, line_number, violations, key)


def parse_rows(files: list[Path], session_id: str | None) -> tuple[list[dict[str, Any]], dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    invalid_rows: list[dict[str, Any]] = []
    truncated_tails: list[dict[str, Any]] = []
    schema_violations: list[dict[str, Any]] = []
    privacy_violations: list[dict[str, Any]] = []

    for path in files:
        data = path.read_bytes()
        lines = data.splitlines()
        has_complete_tail = data.endswith(b"\n") or data.endswith(b"\r")
        for index, raw_line in enumerate(lines, start=1):
            if not raw_line.strip():
                continue
            try:
                row = json.loads(raw_line)
            except (json.JSONDecodeError, UnicodeDecodeError) as error:
                item = {"file": str(path), "line": index, "error": str(error)}
                if index == len(lines) and not has_complete_tail:
                    truncated_tails.append(item)
                else:
                    invalid_rows.append(item)
                continue
            if not isinstance(row, dict):
                invalid_rows.append({"file": str(path), "line": index, "error": "row is not an object"})
                continue
            if session_id is not None and row.get("sessionId") != session_id:
                continue

            missing = sorted(REQUIRED_FIELDS - set(row))
            if missing:
                schema_violations.append(
                    {"file": str(path), "line": index, "type": "missing-fields", "fields": missing}
                )
            checks = (
                (row.get("schemaVersion") == 3, "schema-version"),
                (row.get("traceProfile") in ALLOWED_PROFILES, "trace-profile"),
                (row.get("traceMode") == row.get("traceProfile"), "trace-mode"),
                (row.get("category") in ALLOWED_CATEGORIES, "category"),
                (row.get("dimension") in ALLOWED_DIMENSIONS, "dimension"),
                (row.get("importance") in ALLOWED_IMPORTANCE, "importance"),
                (type(row.get("seq")) is int, "seq-type"),
                (type(row.get("tsMs")) is int, "timestamp-type"),
                (isinstance(row.get("sessionId"), str) and bool(row.get("sessionId")), "session-id"),
                (isinstance(row.get("domain"), str) and bool(row.get("domain")), "domain"),
                (isinstance(row.get("event"), str) and bool(row.get("event")), "event"),
                (isinstance(row.get("payload"), dict), "payload-type"),
            )
            for valid, violation_type in checks:
                if not valid:
                    schema_violations.append(
                        {"file": str(path), "line": index, "type": violation_type}
                    )
            if row.get("traceProfile") == "production" and row.get("importance") in {"debug", "raw"}:
                schema_violations.append(
                    {"file": str(path), "line": index, "type": "production-importance"}
                )

            serialized = raw_line.decode("utf-8", errors="replace")
            for violation_type, pattern in RAW_SECRET_PATTERNS:
                if pattern.search(serialized):
                    privacy_violations.append(
                        {"file": str(path), "line": index, "type": violation_type}
                    )
            payload = row.get("payload")
            if isinstance(payload, dict):
                if payload.get("platform") != "ios":
                    schema_violations.append(
                        {"file": str(path), "line": index, "type": "platform"}
                    )
                scan_payload(
                    payload,
                    str(row.get("traceProfile", "")),
                    path,
                    index,
                    privacy_violations,
                )

            row["_file"] = str(path)
            row["_line"] = index
            rows.append(row)

    return rows, {
        "invalidRows": invalid_rows,
        "truncatedTails": truncated_tails,
        "schemaViolations": schema_violations,
        "privacyViolations": privacy_violations,
    }


def sequence_report(rows: list[dict[str, Any]]) -> dict[str, Any]:
    grouped: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for row in rows:
        grouped[str(row.get("sessionId"))].append(row)

    violations: list[dict[str, Any]] = []
    sessions: dict[str, Any] = {}
    for session, session_rows in grouped.items():
        previous: int | None = None
        duplicates = 0
        regressions = 0
        for row in session_rows:
            seq = row.get("seq")
            if not isinstance(seq, int):
                continue
            if previous is not None and seq <= previous:
                violation_type = "duplicate" if seq == previous else "regression"
                duplicates += int(violation_type == "duplicate")
                regressions += int(violation_type == "regression")
                violations.append(
                    {
                        "sessionId": session,
                        "file": row.get("_file"),
                        "line": row.get("_line"),
                        "previousSeq": previous,
                        "seq": seq,
                        "type": violation_type,
                    }
                )
            previous = seq
        sequences = [row["seq"] for row in session_rows if isinstance(row.get("seq"), int)]
        sessions[session] = {
            "rows": len(session_rows),
            "firstSeq": sequences[0] if sequences else None,
            "lastSeq": sequences[-1] if sequences else None,
            "duplicates": duplicates,
            "regressions": regressions,
        }
    return {"sessions": sessions, "violations": violations}


def budget_report(files: list[Path], rows: list[dict[str, Any]]) -> dict[str, Any]:
    rows_by_file: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for row in rows:
        rows_by_file[str(row.get("_file"))].append(row)

    raw_files: list[dict[str, Any]] = []
    violations: list[dict[str, Any]] = []
    directories: dict[str, list[Path]] = defaultdict(list)
    for path in files:
        if not is_raw_trace(path):
            continue
        file_rows = rows_by_file.get(str(path), [])
        if not file_rows:
            continue
        directories[str(path.parent)].append(path)
        profiles = [str(row.get("traceProfile")) for row in file_rows]
        profile = profiles[0] if profiles else "unknown"
        opened = next(
            (row for row in file_rows if row.get("domain") == "trace" and row.get("event") == "fileOpened"),
            None,
        )
        payload = opened.get("payload", {}) if opened else {}
        configured_bytes = payload.get("maxFileBytes") or PROFILE_BUDGETS.get(profile, {}).get("maxFileBytes")
        configured_files = payload.get("maxFiles") or PROFILE_BUDGETS.get(profile, {}).get("maxFiles")
        actual_bytes = path.stat().st_size
        within_budget = isinstance(configured_bytes, int) and actual_bytes <= configured_bytes
        item = {
            "file": str(path),
            "profile": profile,
            "actualBytes": actual_bytes,
            "maxFileBytes": configured_bytes,
            "maxFiles": configured_files,
            "withinBudget": within_budget,
        }
        raw_files.append(item)
        if not within_budget:
            violations.append({**item, "type": "file-size"})

    retention: list[dict[str, Any]] = []
    for directory, directory_files in directories.items():
        latest_file = sorted(directory_files, key=trace_file_order)[-1]
        latest = next(item for item in raw_files if item["file"] == str(latest_file))
        limit = latest["maxFiles"]
        valid = limit is not None and len(directory_files) <= limit
        item = {"directory": directory, "files": len(directory_files), "maxFiles": limit, "withinBudget": valid}
        retention.append(item)
        if not valid:
            violations.append({**item, "type": "retention"})

    return {
        "rawFiles": raw_files,
        "aggregateExports": [str(path) for path in files if not is_raw_trace(path)],
        "retention": retention,
        "pressureNotices": sum(1 for row in rows if row.get("event") == "traceBudgetNotice"),
        "violations": violations,
    }


def coverage_report(rows: list[dict[str, Any]], required_flow: str) -> dict[str, Any]:
    events = {str(row.get("event")) for row in rows}
    requested = [] if required_flow == "none" else [required_flow]
    if required_flow == "all":
        requested = ["startup", "library"]
    missing: list[dict[str, str]] = []
    flows: dict[str, Any] = {}
    for flow in ("startup", "library"):
        checks = []
        for alternatives, label in FLOW_REQUIREMENTS[flow]:
            observed = sorted(alternatives & events)
            checks.append({"label": label, "alternatives": sorted(alternatives), "observed": observed})
            if flow in requested and not observed:
                missing.append({"flow": flow, "anchor": label})
        flows[flow] = {"required": flow in requested, "checks": checks}
    return {"flows": flows, "missingRequiredAnchors": missing}


def pairing_report(rows: list[dict[str, Any]]) -> dict[str, Any]:
    by_session_operation: dict[tuple[str, str], list[dict[str, Any]]] = defaultdict(list)
    for row in rows:
        payload = row.get("payload")
        operation = payload.get("operationId") if isinstance(payload, dict) else None
        if isinstance(operation, str):
            by_session_operation[(str(row.get("sessionId")), operation)].append(row)

    violations: list[dict[str, Any]] = []
    evaluated = 0
    for (session, operation), operation_rows in by_session_operation.items():
        events = {str(row.get("event")) for row in operation_rows}
        for start, outcomes in PAIR_RULES.items():
            if start not in events:
                continue
            evaluated += 1
            if not outcomes & events:
                start_row = next(row for row in operation_rows if row.get("event") == start)
                violations.append(
                    {
                        "sessionId": session,
                        "operationId": operation,
                        "startEvent": start,
                        "expectedOutcomes": sorted(outcomes),
                        "seq": start_row.get("seq"),
                    }
                )
    return {"evaluatedStarts": evaluated, "violations": violations}


def count_values(rows: list[dict[str, Any]], field: str) -> dict[str, int]:
    return dict(sorted(Counter(str(row.get(field)) for row in rows).items()))


def build_report(files: list[Path], rows: list[dict[str, Any]], parsing: dict[str, Any], required_flow: str) -> dict[str, Any]:
    sequence = sequence_report(rows)
    budget = budget_report(files, rows)
    coverage = coverage_report(rows, required_flow)
    pairing = pairing_report(rows)
    failures = {
        "noRows": int(not rows),
        "invalidRows": len(parsing["invalidRows"]),
        "schemaViolations": len(parsing["schemaViolations"]),
        "sequenceViolations": len(sequence["violations"]),
        "privacyViolations": len(parsing["privacyViolations"]),
        "budgetViolations": len(budget["violations"]),
        "coverageViolations": len(coverage["missingRequiredAnchors"]),
        "pairingViolations": len(pairing["violations"]),
    }
    timestamps = [row["tsMs"] for row in rows if isinstance(row.get("tsMs"), int)]
    return {
        "gate": "PASS" if not any(failures.values()) else "FAIL",
        "scope": {
            "files": [str(path) for path in files],
            "rows": len(rows),
            "sessions": sorted({str(row.get("sessionId")) for row in rows}),
            "firstTsMs": min(timestamps) if timestamps else None,
            "lastTsMs": max(timestamps) if timestamps else None,
            "profiles": count_values(rows, "traceProfile"),
            "domains": count_values(rows, "domain"),
            "events": count_values(rows, "event"),
            "categories": count_values(rows, "category"),
            "dimensions": count_values(rows, "dimension"),
            "importance": count_values(rows, "importance"),
        },
        "failures": failures,
        "schema": {
            "invalidRows": parsing["invalidRows"],
            "truncatedTails": parsing["truncatedTails"],
            "violations": parsing["schemaViolations"],
        },
        "sequence": sequence,
        "privacy": {"violations": parsing["privacyViolations"]},
        "budget": budget,
        "coverage": coverage,
        "pairing": pairing,
    }


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("inputs", nargs="+", help="JSONL files or directories")
    parser.add_argument("--session-id", help="only analyze one launch session")
    parser.add_argument(
        "--require-flow",
        choices=("none", "startup", "library", "all"),
        default="none",
        help="fail when required critical flow anchors are missing",
    )
    parser.add_argument("--strict", action="store_true", help="exit 2 when any gate fails")
    parser.add_argument("--pretty", action="store_true", help="pretty-print JSON")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    files = discover_files(args.inputs)
    if not files:
        print(json.dumps({"gate": "FAIL", "error": "no trace files found"}, ensure_ascii=False))
        return 2
    rows, parsing = parse_rows(files, args.session_id)
    report = build_report(files, rows, parsing, args.require_flow)
    print(json.dumps(report, ensure_ascii=False, indent=2 if args.pretty else None, sort_keys=True))
    return 2 if args.strict and report["gate"] != "PASS" else 0


if __name__ == "__main__":
    sys.exit(main())
