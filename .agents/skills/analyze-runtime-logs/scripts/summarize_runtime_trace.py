#!/usr/bin/env python3

import json
import sys
from collections import Counter
from pathlib import Path


SUSPICIOUS_TERMS = (
    "error",
    "fail",
    "timeout",
    "unavailable",
    "drop",
    "dropped",
    "panic",
    "stall",
    "reconnect",
    "reset",
    "provision",
    "waiting",
    "keyframe",
    "deadline",
)


def fmt_ms(value: int | None) -> str:
    if value is None:
        return "n/a"
    seconds = value / 1000
    return f"{seconds:.3f}s"


def short_payload(payload: object) -> str:
    if isinstance(payload, dict):
        for key in ("message", "reason", "error", "status", "phase", "action"):
            value = payload.get(key)
            if value:
                return str(value)
        items: list[str] = []
        for key in ("level", "from", "to", "streamState", "transportState"):
            value = payload.get(key)
            if value is not None:
                items.append(f"{key}={value}")
        if items:
            return " ".join(items)
        text = json.dumps(payload, ensure_ascii=False, sort_keys=True)
        return text[:160]
    return str(payload)[:160]


def is_suspicious(row: dict) -> bool:
    haystacks = [
        str(row.get("event", "")).lower(),
        str(row.get("domain", "")).lower(),
        str(row.get("category", "")).lower(),
        json.dumps(row.get("payload", ""), ensure_ascii=False).lower(),
    ]
    return any(term in haystack for haystack in haystacks for term in SUSPICIOUS_TERMS)


def main() -> int:
    if len(sys.argv) != 2:
        print("usage: summarize_runtime_trace.py <trace.jsonl>", file=sys.stderr)
        return 2

    path = Path(sys.argv[1])
    if not path.is_file():
        print(f"trace file not found: {path}", file=sys.stderr)
        return 2

    rows: list[dict] = []
    with path.open("r", encoding="utf-8") as handle:
        for line_no, line in enumerate(handle, start=1):
            line = line.strip()
            if not line:
                continue
            try:
                row = json.loads(line)
            except json.JSONDecodeError as exc:
                print(f"invalid json at line {line_no}: {exc}", file=sys.stderr)
                return 1
            if not isinstance(row, dict):
                continue
            rows.append(row)

    if not rows:
        print(f"file: {path}")
        print("rows: 0")
        return 0

    category_counts = Counter(str(row.get("category", "unknown")) for row in rows)
    domain_counts = Counter(str(row.get("domain", "unknown")) for row in rows)
    event_counts = Counter(
        f"{row.get('category', 'unknown')}/{row.get('domain', 'unknown')}/{row.get('event', 'unknown')}"
        for row in rows
    )
    session_counts = Counter(str(row.get("sessionId")) for row in rows if row.get("sessionId"))
    log_levels = Counter(
        str(row.get("payload", {}).get("level", "unknown"))
        for row in rows
        if row.get("category") == "log" and isinstance(row.get("payload"), dict)
    )

    first_ts = next((row.get("tsMs") for row in rows if isinstance(row.get("tsMs"), int)), None)
    last_ts = next((row.get("tsMs") for row in reversed(rows) if isinstance(row.get("tsMs"), int)), None)
    duration_ms = None
    if isinstance(first_ts, int) and isinstance(last_ts, int):
        duration_ms = last_ts - first_ts

    suspicious_rows = [row for row in rows if is_suspicious(row)]

    print(f"file: {path}")
    print(f"rows: {len(rows)}")
    print(f"time_range_ms: {first_ts} -> {last_ts} (duration={fmt_ms(duration_ms)})")
    print(
        "categories: "
        + ", ".join(f"{name}={count}" for name, count in category_counts.most_common())
    )
    print("domains: " + ", ".join(f"{name}={count}" for name, count in domain_counts.most_common(12)))
    if session_counts:
        print(
            "sessions: "
            + ", ".join(f"{name}={count}" for name, count in session_counts.most_common(8))
        )
    if log_levels:
        print(
            "log_levels: "
            + ", ".join(f"{name}={count}" for name, count in log_levels.most_common())
        )

    print("\ntop_events:")
    for name, count in event_counts.most_common(20):
        print(f"  - {count:5d} {name}")

    print("\nsuspicious_rows:")
    # 这里保留首批异常线索，优先帮助代理缩小阅读窗口。
    for row in suspicious_rows[:30]:
        print(
            "  - "
            f"seq={row.get('seq')} tsMs={row.get('tsMs')} "
            f"{row.get('category')}/{row.get('domain')}/{row.get('event')} "
            f"session={row.get('sessionId')} "
            f"summary={short_payload(row.get('payload'))}"
        )

    if len(suspicious_rows) > 30:
        print(f"  - ... {len(suspicious_rows) - 30} more suspicious rows omitted")

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
