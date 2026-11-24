#!/usr/bin/env python3
"""为 UniFFI 0.32 的 foreign async 回调补齐 Swift 6 Sendable 合同。"""

import re
import sys
from pathlib import Path


def patch_async_helpers(source: str) -> str:
    helpers = {
        "uniffiTraitInterfaceCallAsync": ("makeCall", "handleSuccess", "handleError"),
        "uniffiTraitInterfaceCallAsyncWithError": (
            "makeCall", "handleSuccess", "handleError", "lowerError"
        ),
    }
    for name, parameters in helpers.items():
        pattern = re.compile(
            rf"(private func {name}<[^>]+>\(\n)(.*?)(\n\) \{{)", re.DOTALL
        )
        matches = list(pattern.finditer(source))
        if len(matches) != 1:
            raise ValueError(f"UniFFI async helper changed: {name}")
        match = matches[0]
        header = match.group(1).replace("<T>", "<T: Sendable>").replace(
            "<T, E>", "<T: Sendable, E>"
        )
        signature = match.group(2)
        for parameter in parameters:
            parameter_pattern = re.compile(
                rf"(\b{parameter}: )(?:sending )?@escaping (?:@Sendable )?"
            )
            signature, count = parameter_pattern.subn(r"\1@escaping @Sendable ", signature)
            if count != 1:
                raise ValueError(f"UniFFI async parameter changed: {name}.{parameter}")
        source = source[:match.start()] + header + signature + match.group(3) + source[match.end():]
    return source


def patch_async_callbacks(source: str) -> str:
    callbacks = {
        "checkpoint": (("update", "FfiConverterTypeXboxAuthCheckpoint_lift"),),
        "ensureSession": (
            ("targetType", "FfiConverterString.lift"),
            ("rejectedToken", "FfiConverterOptionString.lift"),
        ),
    }
    for name, arguments in callbacks.items():
        marker = f"        {name}: {{ ("
        if source.count(marker) != 1:
            raise ValueError(f"UniFFI async callback changed: {name}")
        start = source.index(marker)
        end = source.index("                droppedCallback: uniffiOutDroppedCallback", start)
        segment = source[start:end]
        if "let uniffiArgument_" not in segment:
            lifting = (
                "            // 回调入栈时消费 owned RustBuffer，Task 只捕获 Sendable Swift 值。\n"
                + "".join(
                    f"            let uniffiArgument_{parameter} = Result {{ try {converter}({parameter}) }}\n"
                    for parameter, converter in arguments
                )
            )
            segment = segment.replace("            let makeCall = {", lifting + "            let makeCall = {", 1)
        for parameter, converter in arguments:
            original = f"{parameter}: try {converter}({parameter})"
            replacement = f"{parameter}: try uniffiArgument_{parameter}.get()"
            if segment.count(original) + segment.count(replacement) != 1:
                raise ValueError(f"UniFFI async argument changed: {name}.{parameter}")
            segment = segment.replace(original, replacement)
        for closure in ("makeCall", "uniffiHandleSuccess", "uniffiHandleError"):
            pattern = rf"(let {closure} = \{{)(?: @Sendable)?"
            segment, count = re.subn(pattern, r"\1 @Sendable", segment)
            if count != 1:
                raise ValueError(f"UniFFI async closure changed: {name}.{closure}")
        # 保留原有 exactly-once completion 与 dropped/cancel；逐参数 Result 各自消费缓冲区。
        source = source[:start] + segment + source[end:]
    return source


def main() -> None:
    if len(sys.argv) != 2:
        raise SystemExit("usage: patch-uniffi-swift-concurrency.py <generated.swift>")
    path = Path(sys.argv[1])
    source = path.read_text()
    patched = patch_async_callbacks(patch_async_helpers(source))
    if patched != source:
        path.write_text(patched)


if __name__ == "__main__":
    main()
