#!/usr/bin/env python3
"""生产 trace 健康报告入口，消费仓库共享分析器。"""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[4] / "scripts"))
from production_trace_health_report import main

if __name__ == "__main__":
    raise SystemExit(main())
