from pathlib import Path
from datetime import datetime
from zoneinfo import ZoneInfo
import hashlib
import json
import re
import subprocess
import sys

binary = Path(sys.argv[1])
destination = Path(sys.argv[2])
symbols = subprocess.check_output(["xcrun", "nm", "-nm", str(binary)], text=True)
rows = symbols.splitlines()
candidates = []
for row in rows:
    name = row.rsplit(None, 1)[-1]
    if (
        "(__TEXT,__text)" in row
        and name.startswith("__ZN14xbox_streaming")
        and ("SessionStartupScope" in name or "SessionCancelToken" in name)
        and "3run" in name
    ):
        candidates.append(name)
assert candidates, "operation symbols present"
disassembly = subprocess.check_output(
    [
        "/opt/homebrew/opt/llvm/bin/llvm-objdump",
        "--disassemble-symbols=" + ",".join(candidates),
        str(binary),
    ],
    text=True,
)
functions = []
current = None
for row in disassembly.splitlines():
    header = re.match(r"^([0-9a-f]+) <(.+)>:$", row)
    if header:
        if current:
            functions.append(current)
        current = {
            "address": "0x" + header.group(1),
            "symbol": header.group(2),
            "prologue": [],
        }
    elif current is not None and len(current["prologue"]) < 22:
        instruction = re.match(r"\s*[0-9a-f]+:\s+(?:[0-9a-f]{8}\s+)?(.*)", row)
        if instruction:
            current["prologue"].append(instruction.group(1))
if current:
    functions.append(current)

def immediate(match):
    value = int(match.group(1), 0)
    return value << (int(match.group(2), 0) if match.group(2) else 0)

for function in functions:
    prefix = 0
    probe_bound = 0
    sp_subtractions = []
    for instruction in function["prologue"]:
        preindex = re.search(r"\[sp, #-(0x[0-9a-f]+|[0-9]+)\]!", instruction)
        if preindex:
            prefix += int(preindex.group(1), 0)
        bound = re.search(
            r"\bsub\s+x9, sp, #(0x[0-9a-f]+|[0-9]+)(?:, lsl #(0x[0-9a-f]+|[0-9]+))?",
            instruction,
        )
        if bound:
            probe_bound += immediate(bound)
        linear = re.search(
            r"\bsub\s+sp, sp, #(0x[0-9a-f]+|[0-9]+)(?:, lsl #(0x[0-9a-f]+|[0-9]+))?",
            instruction,
        )
        if linear:
            sp_subtractions.append(immediate(linear))
    if probe_bound:
        sp_subtractions = [value for value in sp_subtractions if value != 4096]
    function["frameBytes"] = prefix + probe_bound + sum(sp_subtractions)
    function["kind"] = "async" if "closure" in function["symbol"] else "constructor"

result = {
    "capturedAt": datetime.now(ZoneInfo("Asia/Shanghai")).isoformat(),
    "binary": str(binary),
    "binarySha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
    "scope": "Debug iPhoneOS arm64 operation constructors and async prologues; individual frames, not complete runtime call stack",
    "screenshotProviderSymbols": sum("StreamingAccessProvider" in row for row in rows),
    "currentProviderSymbols": sum("IosSessionFlowProvider" in row for row in rows),
    "functions": functions,
    "maxFrameBytes": {
        kind: max(
            function["frameBytes"]
            for function in functions
            if function["kind"] == kind
        )
        for kind in ["async", "constructor"]
    },
}
destination.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n")
print(
    json.dumps(
        {
            "output": str(destination),
            "functions": len(functions),
            "maxFrameBytes": result["maxFrameBytes"],
            "screenshotProviderSymbols": result["screenshotProviderSymbols"],
        }
    )
)
