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


def patch_rust_future_cancellation(source: str) -> str:
    marker = "// XBXRC: Swift cancellation owns the Rust future lifecycle."
    if marker in source:
        return source
    pattern = re.compile(r"(\s+)freeFunc: (ffi_\w+_rust_future_)free_(\w+),")
    source, count = pattern.subn(
        r"\1freeFunc: \2free_\3,\1cancelFunc: \2cancel_\3,", source
    )
    if count == 0:
        raise ValueError("UniFFI Rust future call sites changed")
    start = source.index("fileprivate func uniffiRustCallAsync<F, T>(")
    end = source.index("// Callback handlers for an async calls.", start)
    source = source[:start] + '''// XBXRC: Swift cancellation owns the Rust future lifecycle.
fileprivate final class UniffiRustFutureCancellation: @unchecked Sendable {
    private let lock = NSLock()
    private var handle: UInt64?
    private let cancelFunc: @Sendable (UInt64) -> Void
    init(handle: UInt64, cancelFunc: @escaping @Sendable (UInt64) -> Void) {
        self.handle = handle
        self.cancelFunc = cancelFunc
    }
    func cancel() {
        lock.lock()
        defer { lock.unlock() }
        if let handle { cancelFunc(handle) }
    }
    func finish(_ free: (UInt64) -> Void) {
        lock.lock()
        defer { lock.unlock() }
        if let handle {
            self.handle = nil
            free(handle)
        }
    }
}

fileprivate func uniffiRustCallAsync<F, T>(
    rustFutureFunc: () -> UInt64,
    pollFunc: (UInt64, @escaping UniffiRustFutureContinuationCallback, UInt64) -> (),
    completeFunc: (UInt64, UnsafeMutablePointer<RustCallStatus>) -> F,
    freeFunc: (UInt64) -> (),
    cancelFunc: @escaping @Sendable (UInt64) -> Void,
    liftFunc: (F) throws -> T,
    errorHandler: ((RustBuffer) throws -> Swift.Error)?
) async throws -> T {
    uniffiEnsureXboxIosBridgeInitialized()
    // 只有可抛错读取消费 Swift 取消；不可抛错的 teardown 继续完成清理。
    let cancellable = errorHandler != nil
    if cancellable { try Task.checkCancellation() }
    let rustFuture = rustFutureFunc()
    let cancellation = UniffiRustFutureCancellation(handle: rustFuture, cancelFunc: cancelFunc)
    defer { cancellation.finish(freeFunc) }
    return try await withTaskCancellationHandler {
        if cancellable { try Task.checkCancellation() }
        var pollResult: Int8
        repeat {
            pollResult = await withUnsafeContinuation {
                pollFunc(rustFuture, { handle, result in
                    uniffiFutureContinuationCallback(handle: handle, pollResult: result)
                }, uniffiContinuationHandleMap.insert(obj: $0))
            }
            if cancellable { try Task.checkCancellation() }
        } while pollResult != UNIFFI_RUST_FUTURE_POLL_READY
        let result = try liftFunc(makeRustCall(
            { completeFunc(rustFuture, $0) }, errorHandler: errorHandler
        ))
        if cancellable { try Task.checkCancellation() }
        return result
    } onCancel: {
        if cancellable { cancellation.cancel() }
    }
}

''' + source[end:]
    return source


def main() -> None:
    if len(sys.argv) != 2:
        raise SystemExit("usage: patch-uniffi-swift-concurrency.py <generated.swift>")
    path = Path(sys.argv[1])
    source = path.read_text()
    patched = patch_rust_future_cancellation(patch_async_callbacks(patch_async_helpers(source)))
    if patched != source:
        path.write_text(patched)


if __name__ == "__main__":
    main()
