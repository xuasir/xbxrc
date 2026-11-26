import CoreVideo
import Foundation
@preconcurrency import Metal
import XCTest

@testable import XBXRC

private final class ExecutionValueBox<Value: Sendable>: Sendable {
    private let lock = NSLock()
    private nonisolated(unsafe) var value: Value
    init(_ value: Value) { self.value = value }
    func get() -> Value { lock.withLock { value } }
    func update(_ body: (inout Value) -> Void) { lock.withLock { body(&value) } }
}

private final class InputWireRecorder: Sendable {
    private let lock = NSLock()
    private nonisolated(unsafe) var accepted: [Data] = []
    private nonisolated(unsafe) var failedA = false
    private nonisolated(unsafe) var aAttempts = 0
    private let completed: @Sendable () -> Void

    init(completed: @escaping @Sendable () -> Void) { self.completed = completed }
    func send(_ data: Data) -> Bool {
        let result: (accepted: Bool, complete: Bool) = lock.withLock {
            let mask = data.count >= 18 ? UInt16(data[16]) | UInt16(data[17]) << 8 : 0
            if mask == 16 {
                aAttempts += 1
                if !failedA {
                    failedA = true
                    return (false, false)
                }
            }
            accepted.append(data)
            return (true, accepted.count == 3)
        }
        if result.complete { completed() }
        return result.accepted
    }
    var packets: [Data] { lock.withLock { accepted } }
    var failedPressRetried: Bool { lock.withLock { aAttempts == 2 } }
}

@MainActor
final class StreamingExecutionTests: XCTestCase {
    private func core() -> StreamInputCore {
        var core = StreamInputCore()
        core.configure(
            .init(revision: 1, preferences: .init(), controllerID: nil, layoutKey: ""), at: 0)
        core.accepted(core.candidate(at: 0))
        return core
    }

    private func batch(
        _ states: [IOSStreamGamepadState], at: Double = 10, epoch: UInt64 = 0, sequence: UInt64 = 1
    ) -> InputSampleBatch {
        .init(
            sourceID: "touch", sequence: sequence, inputEpoch: epoch,
            transitions: states.map {
                .init(state: $0, collectedAtMs: at, sourceActive: $0 != .neutral)
            },
            finalState: states.last ?? .neutral, sourceActive: (states.last ?? .neutral) != .neutral
        )
    }

    func testShortPressRemainsPendingUntilEachSendIsAccepted() {
        var core = core()
        core.consumeTouch(batch([.init(buttonMask: 16), .neutral]), at: 10)
        let down = core.candidate(at: 10)
        XCTAssertEqual(down.state.buttonMask, 16)
        XCTAssertEqual(core.candidate(at: 18).state.buttonMask, 16)
        core.accepted(down)
        let up = core.candidate(at: 18)
        XCTAssertEqual(up.state, .neutral)
        core.accepted(up)
        XCTAssertEqual(core.snapshot.acceptedEdges, 2)
    }

    func testSourcePriorityAndSuppressionFencePreserveFinalNeutral() {
        var core = core()
        core.sampleController(.init(raw: .init(buttonMask: 32), connected: true), at: 1)
        core.accepted(core.candidate(at: 1))
        core.consumeTouch(batch([.init(buttonMask: 16)]), at: 10)
        XCTAssertEqual(core.snapshot.state.buttonMask, 32)
        core.sampleController(.init(connected: true), at: 11)
        XCTAssertEqual(core.candidate(at: 11).state.buttonMask, 16)
        core.barrier(epoch: 1, suppressed: true, reason: "menu")
        core.consumeTouch(batch([.init(buttonMask: 16)], epoch: 0, sequence: 2), at: 12)
        XCTAssertTrue(core.candidate(at: 12).barrier)
        XCTAssertEqual(core.candidate(at: 12).state, .neutral)
    }

    func testCapacityAndAgeLimitsRetirePendingEdgesThroughNeutral() {
        var capacity = core()
        let states = (0..<129).map {
            $0.isMultiple(of: 2) ? IOSStreamGamepadState(buttonMask: 16) : .neutral
        }
        capacity.consumeTouch(batch(states), at: 10)
        XCTAssertTrue(capacity.candidate(at: 10).barrier)
        XCTAssertEqual(capacity.snapshot.discardedEdges, 128)
        XCTAssertEqual(capacity.snapshot.pendingEdges, 0)
        var aged = core()
        aged.consumeTouch(batch([.init(buttonMask: 16)]), at: 10)
        XCTAssertTrue(aged.candidate(at: 75).barrier)
        XCTAssertEqual(aged.snapshot.barrierReason, "digitalEdgeExpired")
    }

    func testAnalogUpdatesUseLatestValueWithoutAddingDigitalEdges() {
        var core = core()
        core.consumeTouch(batch([.init(leftThumbX: 0.3), .init(leftThumbX: 0.8)]), at: 10)
        XCTAssertEqual(core.snapshot.pendingEdges, 0)
        XCTAssertGreaterThan(core.candidate(at: 10).state.leftThumbX, 0.5)
    }

    func testTouchBindingSendsDownUpAndRetriesFailedSendDuringMainActorWork() async {
        let delivered = expectation(description: "neutral, down, up accepted")
        let recorder = InputWireRecorder { delivered.fulfill() }
        let worker = StreamingInputWorker(feedback: nil, eventSink: { _ in })
        let input = StreamInputSession(controller: { nil })
        worker.attach(InputChannelLease(send: { recorder.send($0) }))
        input.bindWorker(worker)
        worker.start()
        let at = StreamingFrameFeedbackClock.nowMilliseconds / 1_000
        input.submitTouchLayoutFrame(
            .init(
                layoutID: "smoke", sequence: 1, timestamp: at,
                stateCompletedTimestamp: at, processingDuration: 0, reason: "pointer", events: [],
                transitions: [
                    .init(.init(buttonMask: 16), sourceActive: true),
                    .init(.neutral, sourceActive: false),
                ],
                transitionTimestamps: [at, at], activeControlIDs: [], activeLayerIDs: [],
                pointerCount: 0, gamepad: .neutral))
        blockMainThread()
        let result = await XCTWaiter.fulfillment(of: [delivered], timeout: 2)
        input.unbindWorker(worker)
        worker.stop()
        XCTAssertEqual(result, .completed)
        let packets = Array(recorder.packets.prefix(3))
        XCTAssertEqual(packets.map { UInt16($0[16]) | UInt16($0[17]) << 8 }, [0, 16, 0])
        XCTAssertTrue(recorder.failedPressRetried)
        let sequences = packets.map { packet in
            (0..<4).reduce(UInt32(0)) { $0 | UInt32(packet[2 + $1]) << (8 * $1) }
        }
        XCTAssertEqual(sequences, [1, 2, 3])
    }

    func testDrawableWaitDrainChoosesNewestIngressAndRetiresOldTicket() {
        let slot = StreamingLatestIngressSlot<Int>()
        let oldTicket = slot.submit(1)!
        _ = slot.submit(2)
        let latest = slot.takePending()
        XCTAssertEqual(latest?.value, 2)
        XCTAssertEqual(latest?.replacedCount, 1)
        var delivered: [Int] = []
        _ = slot.drain(oldTicket) { delivered.append($0.value) }
        XCTAssertTrue(delivered.isEmpty)
    }

    func testOldGpuCompletionReleasesItsTicketAndPreservesNewInflight() {
        var buffer: CVPixelBuffer?
        XCTAssertEqual(
            CVPixelBufferCreate(nil, 4, 4, kCVPixelFormatType_32BGRA, nil, &buffer),
            kCVReturnSuccess)
        let mailbox = StreamingLatestFrameMailbox()
        let context = StreamingPresentationTraceContext(
            attemptID: "smoke", generation: 1, peerEpoch: 1)
        func frame() -> StreamingDecodedFrameRef {
            mailbox.makeFrame(
                storage: .pixelBuffer(buffer!), width: 4, height: 4, rotationDegrees: 0,
                isFullRange: false, traceContext: context)
        }
        let old = frame()
        _ = mailbox.submit(old)
        _ = mailbox.beginRender()
        mailbox.clearCandidates()
        let next = frame()
        _ = mailbox.submit(next)
        XCTAssertTrue(mailbox.beginRender().drawableBusy)
        _ = mailbox.finishRender(identifier: old.identifier, requeueAsLatest: false)
        XCTAssertEqual(mailbox.beginRender().frame?.identifier, next.identifier)
        _ = mailbox.finishRender(identifier: old.identifier, requeueAsLatest: false)
        XCTAssertTrue(mailbox.snapshotDrawableBusy())
        _ = mailbox.finishRender(identifier: next.identifier, requeueAsLatest: false)
        XCTAssertEqual(mailbox.retainedFrameCount(including: nil), 0)
    }

    func testPrecompiledPipelineCacheAndGpuWorkProgressDuringMainActorWork() async {
        guard let device = MTLCreateSystemDefaultDevice() else {
            XCTFail("Metal device unavailable")
            return
        }
        let cache = StreamingRenderResourceCache()
        let warmed = expectation(description: "pipeline warmup")
        let warmCount = ExecutionValueBox<(Int64, Int64)>((0, 0))
        cache.queue.async {
            _ = cache.resources(on: device)
            warmCount.update { $0 = (cache.counts.loaded, cache.counts.created) }
            warmed.fulfill()
        }
        let warmResult = await XCTWaiter.fulfillment(of: [warmed], timeout: 3)
        XCTAssertEqual(warmResult, .completed)
        XCTAssertEqual(warmCount.get().0, 1)
        XCTAssertEqual(warmCount.get().1, 3)
        let completed = expectation(description: "GPU completion")
        let encoded = ExecutionValueBox(false)
        let output = ExecutionValueBox<[UInt8]>([])
        cache.queue.async {
            guard let first = cache.resources(on: device), let second = cache.resources(on: device),
                first.nv12 === second.nv12,
                let buffer = device.makeBuffer(length: 16, options: .storageModeShared),
                let command = second.commandQueue.makeCommandBuffer(),
                let encoder = command.makeBlitCommandEncoder()
            else {
                completed.fulfill()
                return
            }
            encoder.fill(buffer: buffer, range: 0..<16, value: 9)
            encoder.endEncoding()
            encoded.update { $0 = true }
            command.addCompletedHandler { _ in
                output.update {
                    $0 = Array(
                        UnsafeBufferPointer(
                            start: buffer.contents().assumingMemoryBound(to: UInt8.self), count: 16)
                    )
                }
                completed.fulfill()
            }
            command.commit()
        }
        blockMainThread()
        XCTAssertTrue(encoded.get())
        let gpuResult = await XCTWaiter.fulfillment(of: [completed], timeout: 2)
        XCTAssertEqual(gpuResult, .completed)
        XCTAssertEqual(output.get(), Array(repeating: 9, count: 16))
    }

    func testResetTimeoutFenceRejectsLateAcknowledgement() async {
        let session = StreamingPresentationSession()
        let scope = StreamExecutionScope(renderEpoch: 1)
        session.bind(scope: scope, active: true, surfaceReady: true) { _, _ in
            try? await Task.sleep(for: .milliseconds(40))
            return .acknowledged
        }
        let started = Task { await session.reset(expected: scope, actionID: 1) }
        try? await Task.sleep(for: .milliseconds(10))
        session.cancelReset(1)
        let acknowledged = await started.value
        XCTAssertEqual(acknowledged, .superseded)
    }

    private func blockMainThread() { Thread.sleep(forTimeInterval: 0.05) }
}
