import CoreVideo
import Foundation
import Metal
@preconcurrency import Metal
import QuartzCore
import UIKit
@preconcurrency import WebRTC
import simd

#if !targetEnvironment(simulator)
    import MetalFX
#endif

struct StreamingTraceCounterWindow: Sendable {
    private var previousAt: Double?
    private var previousScope: String?
    private var previousCounters: [String: Int64] = [:]

    mutating func observe(at: Double, scope: String, counters: [String: Int64?])
        -> [String: IOSRuntimeTraceValue]
    {
        let sameWindow = previousScope == scope && previousAt.map { at > $0 } == true
        var payload: [String: IOSRuntimeTraceValue] = [
            "sampledAtMonotonicMs": .double(at),
            "sampleClock": .string("monotonic"),
            "windowStartMonotonicMs": sameWindow ? .double(previousAt!) : .null,
            "windowEndMonotonicMs": .double(at),
            "sampleWindowMs": sameWindow ? .double(at - previousAt!) : .null,
            "sampleCount": .integer(sameWindow ? 2 : 1),
        ]
        let reset =
            sameWindow
            && counters.contains { key, value in
                key != "packetsLost"
                    && value.flatMap { v in previousCounters[key].map { v < $0 } } == true
            }
        payload["counterReset"] = .bool(reset)
        for (key, value) in counters {
            if sameWindow, !reset, let value, let previous = previousCounters[key] {
                payload[key + "Delta"] = .integer(value - previous)
            } else {
                payload[key + "Delta"] = .null
            }
        }
        previousScope = scope
        previousAt = at
        previousCounters = counters.compactMapValues { $0 }
        return payload
    }
}

struct StreamingFrameIntervalEstimator: Sendable {
    private var previousAtMilliseconds: Double?
    private(set) var intervalMilliseconds: Double?

    mutating func observe(_ atMilliseconds: Double) {
        guard atMilliseconds.isFinite, atMilliseconds >= 0 else { return }
        defer { previousAtMilliseconds = atMilliseconds }
        guard let previousAtMilliseconds else { return }
        let interval = atMilliseconds - previousAtMilliseconds
        guard interval > 0, interval < 2000 else { return }
        intervalMilliseconds = intervalMilliseconds.map { $0 * 0.8 + interval * 0.2 } ?? interval
    }
}

struct StreamingDisplayConfiguration: Equatable, Sendable {
    let sourceWidth: Int
    let sourceHeight: Int
    let outputWidth: Int
    let outputHeight: Int
    let superResolutionActive: Bool
    let sharpness: Int
}

struct StreamingDisplayHealthSnapshot: Equatable, Sendable {
    let submitCount: Int64
    let presentCount: Int64
    let lastSubmitAtMilliseconds: Double?
    let lastPresentAtMilliseconds: Double?
    let submitToPresentP50Milliseconds: Double?
    let submitToPresentP95Milliseconds: Double?
    let displayedFrameDelta: Int64?
    let mailboxOverwriteCount: Int64
    let dropCountByReason: [String: Int64]
    let currentDrawableBusy: Bool
    var frameIntervalMilliseconds: Double? = nil
    var submitToPresentP99Milliseconds: Double? = nil
    var configuration: StreamingDisplayConfiguration? = nil
    var latencySampleCount = 0
    var latencyWindowStartMs: Double? = nil
    var latencyWindowEndMs: Double? = nil
    var drawablePresentCount: Int64 = 0
    var lastDrawablePresentAtMs: Double? = nil
    var submitToDrawableP50Ms: Double? = nil
    var submitToDrawableP95Ms: Double? = nil
    var submitToDrawableP99Ms: Double? = nil
    var drawableSampleCount = 0
    var drawableWindowStartMs: Double? = nil
    var drawableWindowEndMs: Double? = nil

    var presentationTimestampSource = "gpuProxy"
    var renderEpoch: UInt64 = 0
    var surfaceRevision: UInt64 = 0
    var presentationActive = true
    var surfaceReady = false
    var sampledAtMs: Double = 0
    var drawableAcquireStartedAtMs: Double? = nil
    var oldestInflightAtMs: Double? = nil
    var resourceCounters = StreamingRenderResourceCounters()
    var firstDisplayedAtMs: Double? = nil
    var lastDisplayedFrameID: UInt64? = nil

    var displayedCount: Int64 {
        presentationTimestampSource == "drawablePresented" ? drawablePresentCount : presentCount
    }
    var lastDisplayedAtMs: Double? {
        presentationTimestampSource == "drawablePresented"
            ? lastDrawablePresentAtMs : lastPresentAtMilliseconds
    }

    static let zero = StreamingDisplayHealthSnapshot(
        submitCount: 0,
        presentCount: 0,
        lastSubmitAtMilliseconds: nil,
        lastPresentAtMilliseconds: nil,
        submitToPresentP50Milliseconds: nil,
        submitToPresentP95Milliseconds: nil,
        displayedFrameDelta: nil,
        mailboxOverwriteCount: 0,
        dropCountByReason: [:],
        currentDrawableBusy: false
    )

    func tracePayload() -> [String: IOSRuntimeTraceValue] {
        [
            "renderEpoch": .integer(Int64(clamping: renderEpoch)),
            "surfaceRevision": .integer(Int64(clamping: surfaceRevision)),
            "presentationTimestampSource": .string(presentationTimestampSource),
            "displayedCount": .integer(displayedCount),
            "sampledAtMonotonicMs": .double(sampledAtMs),
            "resourceCounters": .object(resourceCounters.tracePayload()),
            "submitCount": .integer(submitCount),
            "presentCount": .integer(presentCount),
            "lastSubmitAtMs": lastSubmitAtMilliseconds.map(IOSRuntimeTraceValue.double) ?? .null,
            "lastPresentAtMs": lastPresentAtMilliseconds.map(IOSRuntimeTraceValue.double) ?? .null,
            "submitToPresentP50Ms": submitToPresentP50Milliseconds.map(IOSRuntimeTraceValue.double)
                ?? .null,
            "submitToPresentP95Ms": submitToPresentP95Milliseconds.map(IOSRuntimeTraceValue.double)
                ?? .null,
            "submitToPresentP99Ms": submitToPresentP99Milliseconds.map(IOSRuntimeTraceValue.double)
                ?? .null,
            "displayedFrameDelta": displayedFrameDelta.map(IOSRuntimeTraceValue.integer) ?? .null,
            "mailboxOverwriteCount": .integer(mailboxOverwriteCount),
            "dropCountByReason": .object(dropCountByReason.mapValues(IOSRuntimeTraceValue.integer)),
            "currentDrawableBusy": .bool(currentDrawableBusy),
            "frameIntervalMs": frameIntervalMilliseconds.map(IOSRuntimeTraceValue.double) ?? .null,
            "frameIntervalSource": .string(presentationTimestampSource),
            "presentTimestampSource": .string("commandBufferCompletion"),
            "latencySampleCount": .integer(Int64(latencySampleCount)),
            "latencyWindowStartMonotonicMs": latencyWindowStartMs.map(IOSRuntimeTraceValue.double)
                ?? .null,
            "latencyWindowEndMonotonicMs": latencyWindowEndMs.map(IOSRuntimeTraceValue.double)
                ?? .null,
            "drawablePresentCount": .integer(drawablePresentCount),
            "lastDrawablePresentAtMs": lastDrawablePresentAtMs.map(IOSRuntimeTraceValue.double)
                ?? .null,
            "drawableTimestampSource": .string("drawablePresentedTime"),
            "drawablePresentationObserved": .bool(drawableSampleCount > 0),
            "submitToDrawablePresentP50Ms": submitToDrawableP50Ms.map(IOSRuntimeTraceValue.double)
                ?? .null,
            "submitToDrawablePresentP95Ms": submitToDrawableP95Ms.map(IOSRuntimeTraceValue.double)
                ?? .null,
            "submitToDrawablePresentP99Ms": submitToDrawableP99Ms.map(IOSRuntimeTraceValue.double)
                ?? .null,
            "drawableSampleCount": .integer(Int64(drawableSampleCount)),
            "drawableWindowStartMonotonicMs": drawableWindowStartMs.map(IOSRuntimeTraceValue.double)
                ?? .null,
            "drawableWindowEndMonotonicMs": drawableWindowEndMs.map(IOSRuntimeTraceValue.double)
                ?? .null,
        ]
    }
}

enum StreamingPresentationLifecycleEvent: Sendable {
    case displayPipelineReady(StreamingPresentationTraceContext)
    case firstFrameAccepted(StreamingPresentationTraceContext)
    case firstFrameSubmitted(StreamingPresentationTraceContext)
    case firstFramePresented(StreamingPresentationTraceContext)
    case displayHealth(
        StreamingPresentationTraceContext,
        StreamingDisplayHealthSnapshot
    )

    var context: StreamingPresentationTraceContext {
        switch self {
        case .displayPipelineReady(let context),
            .firstFrameAccepted(let context),
            .firstFrameSubmitted(let context),
            .firstFramePresented(let context),
            .displayHealth(let context, _):
            context
        }
    }
}

struct StreamingDisplayHealthCoalescer {
    static let interval: Duration = .milliseconds(100)
    private var context: StreamingPresentationTraceContext?
    private var lastDelivered: StreamingDisplayHealthSnapshot?
    private(set) var pending: StreamingDisplayHealthSnapshot?

    mutating func offer(
        context: StreamingPresentationTraceContext,
        snapshot: StreamingDisplayHealthSnapshot
    ) -> StreamingPresentationLifecycleEvent? {
        if self.context != context {
            self.context = context
            lastDelivered = snapshot
            pending = nil
            return .displayHealth(context, snapshot)
        }
        pending = snapshot == lastDelivered ? nil : snapshot
        return nil
    }

    mutating func flush() -> StreamingPresentationLifecycleEvent? {
        guard let context, let pending else { return nil }
        self.pending = nil
        lastDelivered = pending
        return .displayHealth(context, pending)
    }

    mutating func reset() {
        context = nil
        pending = nil
        lastDelivered = nil
    }
}

struct StreamingGpuCompletionTiming: Sendable {
    let completedAtMs: Double
    let slotReleasedAtMs: Double
    let mainActorEnteredAtMs: Double
    let feedbackFinishedAtMs: Double
    let telemetryFinishedAtMs: Double

    func tracePayload(recordedAtMs: Double) -> [String: IOSRuntimeTraceValue] {
        [
            "gpuToSlotReleaseMs": .double(max(0, slotReleasedAtMs - completedAtMs)),
            "gpuToMainActorMs": .double(max(0, mainActorEnteredAtMs - completedAtMs)),
            "frameFeedbackWorkMs": .double(max(0, feedbackFinishedAtMs - mainActorEnteredAtMs)),
            "displayTelemetryWorkMs": .double(max(0, telemetryFinishedAtMs - feedbackFinishedAtMs)),
            "mainActorWorkBeforeTraceMs": .double(max(0, recordedAtMs - mainActorEnteredAtMs)),
            "gpuToTraceRecordMs": .double(max(0, recordedAtMs - completedAtMs)),
        ]
    }
}

struct StreamingFrameIngressTiming: Sendable {
    let receivedAtMs: Double
    let drainScheduledAtMs: Double
    let drainStartedAtMs: Double
    let mainActorEnteredAtMs: Double

    func queueTracePayload() -> [String: IOSRuntimeTraceValue] {
        [
            "ingressQueueMs": .double(max(0, mainActorEnteredAtMs - receivedAtMs)),
            "ingressDrainQueueMs": .double(max(0, drainStartedAtMs - drainScheduledAtMs)),
            "ingressDrainWorkBeforeFrameMs": .double(
                max(0, mainActorEnteredAtMs - drainStartedAtMs)),
        ]
    }

    func tracePayload(acceptedAtMs: Double) -> [String: IOSRuntimeTraceValue] {
        var payload = queueTracePayload()
        payload["frameAcceptWorkMs"] = .double(max(0, acceptedAtMs - mainActorEnteredAtMs))
        return payload
    }
}

func streamingPresentationTracePayload(
    _ payload: [String: IOSRuntimeTraceValue],
    context: StreamingPresentationTraceContext?
) -> [String: IOSRuntimeTraceValue] {
    guard let context else { return payload }
    var contextualPayload = payload
    contextualPayload["attemptId"] = .string(context.attemptID)
    contextualPayload["generation"] = .integer(Int64(clamping: context.generation))
    contextualPayload["peerEpoch"] = .integer(Int64(clamping: context.peerEpoch))
    return contextualPayload
}

func streamingPresentationContextDidChange(
    previous: StreamingPresentationTraceContext?,
    next: StreamingPresentationTraceContext?
) -> Bool {
    previous != next
}

enum StreamingVideoFrameDropReason: String {
    case ingressCoalesced
    case unsupportedBufferType
    case unsupportedPixelFormat
    case textureImportFailed
    case drawableUnavailable
    case encoderUnavailable
    case pipelineUnavailable
}

enum StreamingDecodedFrameStorage: @unchecked Sendable {
    case pixelBuffer(CVPixelBuffer)

    #if targetEnvironment(simulator)
        case simulatorI420(StreamingSimulatorI420Frame)
    #endif

    var compatibilityPath: String? {
        switch self {
        case .pixelBuffer:
            nil
        #if targetEnvironment(simulator)
            case .simulatorI420:
                "simulatorI420Upload"
        #endif
        }
    }
}

#if targetEnvironment(simulator)
    struct StreamingSimulatorI420Frame: @unchecked Sendable {
        // mailbox 强引用 WebRTC buffer，确保异步 GPU 上传前 Y/U/V plane 生命周期有效。
        let buffer: any RTCI420BufferProtocol
    }
#endif

struct StreamingDecodedFrameRef: @unchecked Sendable {
    let identifier: UInt64
    let storage: StreamingDecodedFrameStorage
    let width: Int
    let height: Int
    let rotationDegrees: Int
    let isFullRange: Bool
    let colorConfiguration: StreamingVideoColorConfiguration
    let traceContext: StreamingPresentationTraceContext?
    let frameFeedback: StreamingFrameFeedbackTicket?
}

struct StreamingMailboxSubmissionResult {
    let overwroteCandidate: Bool
}

struct StreamingMailboxDrainResult {
    let frame: StreamingDecodedFrameRef?
    let drawableBusy: Bool
}

enum StreamingMailboxRequeuePolicy {
    static func shouldRestoreInflight(hasLatestCandidate: Bool) -> Bool {
        !hasLatestCandidate
    }
}

enum StreamingRenderRetryPolicy {
    static func shouldRequestImmediateTick(
        requeueAsLatest: Bool,
        hasPendingFrame: Bool
    ) -> Bool {
        hasPendingFrame && !requeueAsLatest
    }
}

final class StreamingLatestFrameMailbox: @unchecked Sendable {
    private let lock = NSLock()
    private var inflightCurrent: StreamingDecodedFrameRef?
    private var latestCandidate: StreamingDecodedFrameRef?
    private var mailboxOverwriteCount: Int64 = 0
    private var nextIdentifier: UInt64 = 0

    func reset() {
        lock.lock()
        inflightCurrent = nil
        latestCandidate = nil
        mailboxOverwriteCount = 0
        lock.unlock()
    }

    func makeFrame(
        storage: StreamingDecodedFrameStorage,
        width: Int,
        height: Int,
        rotationDegrees: Int,
        isFullRange: Bool,
        traceContext: StreamingPresentationTraceContext?,
        frameFeedback: StreamingFrameFeedbackTicket? = nil
    ) -> StreamingDecodedFrameRef {
        lock.lock()
        nextIdentifier &+= 1
        let identifier = nextIdentifier
        lock.unlock()
        let colorConfiguration: StreamingVideoColorConfiguration
        switch storage {
        case .pixelBuffer(let pixelBuffer):
            colorConfiguration = .init(
                pixelBuffer: pixelBuffer, width: width, height: height, fullRange: isFullRange)
        #if targetEnvironment(simulator)
            case .simulatorI420:
                colorConfiguration = .init(width: width, height: height, fullRange: isFullRange)
        #endif
        }
        return StreamingDecodedFrameRef(
            identifier: identifier,
            storage: storage,
            width: width,
            height: height,
            rotationDegrees: rotationDegrees,
            isFullRange: isFullRange,
            colorConfiguration: colorConfiguration,
            traceContext: traceContext,
            frameFeedback: frameFeedback
        )
    }

    func submit(_ frame: StreamingDecodedFrameRef) -> StreamingMailboxSubmissionResult {
        lock.lock()
        let overwroteCandidate = latestCandidate != nil
        if overwroteCandidate { mailboxOverwriteCount += 1 }
        latestCandidate = frame
        lock.unlock()
        return StreamingMailboxSubmissionResult(
            overwroteCandidate: overwroteCandidate
        )
    }

    func beginRender() -> StreamingMailboxDrainResult {
        lock.lock()
        defer { lock.unlock() }
        guard inflightCurrent == nil else {
            return StreamingMailboxDrainResult(frame: nil, drawableBusy: true)
        }
        guard let latestCandidate else {
            return StreamingMailboxDrainResult(frame: nil, drawableBusy: false)
        }
        inflightCurrent = latestCandidate
        self.latestCandidate = nil
        return StreamingMailboxDrainResult(frame: latestCandidate, drawableBusy: false)
    }

    func finishRender(
        identifier: UInt64,
        requeueAsLatest: Bool
    ) -> Bool {
        lock.lock()
        defer { lock.unlock() }
        guard let inflightCurrent, inflightCurrent.identifier == identifier else {
            return latestCandidate != nil
        }
        self.inflightCurrent = nil
        if requeueAsLatest {
            if StreamingMailboxRequeuePolicy.shouldRestoreInflight(
                hasLatestCandidate: latestCandidate != nil
            ) {
                latestCandidate = inflightCurrent
            } else {
                mailboxOverwriteCount += 1
            }
        }
        return latestCandidate != nil
    }

    func hasPendingFrame() -> Bool {
        lock.lock()
        let hasPending = latestCandidate != nil
        lock.unlock()
        return hasPending
    }

    func clearCandidates(resetCounters: Bool = false) {
        lock.withLock {
            latestCandidate = nil
            if resetCounters { mailboxOverwriteCount = 0 }
        }
    }

    func retainedFrameCount(including identifier: UInt64?) -> Int {
        lock.withLock {
            let count = (inflightCurrent == nil ? 0 : 1) + (latestCandidate == nil ? 0 : 1)
            guard let identifier, identifier != inflightCurrent?.identifier,
                identifier != latestCandidate?.identifier
            else { return count }
            return count + 1
        }
    }

    func snapshotOverwriteCount() -> Int64 {
        lock.lock()
        let count = mailboxOverwriteCount
        lock.unlock()
        return count
    }

    func snapshotDrawableBusy() -> Bool {
        lock.lock()
        defer { lock.unlock() }
        return inflightCurrent != nil
    }
}

private struct StreamingLatencySample: Sendable {
    let at: Double
    let ms: Double
}

final class StreamingDisplayTelemetry: Sendable {
    private let lock = NSLock()
    private nonisolated(unsafe) var submitCount: Int64 = 0
    private nonisolated(unsafe) var presentCount: Int64 = 0
    private nonisolated(unsafe) var lastSubmitAtMilliseconds: Double?
    private nonisolated(unsafe) var lastPresentAtMilliseconds: Double?
    private nonisolated(unsafe) var frameInterval = StreamingFrameIntervalEstimator()
    private nonisolated(unsafe) var lastPresentDelta: Int64?
    private nonisolated(unsafe) var mailboxOverwriteCount: Int64 = 0
    private nonisolated(unsafe) var dropCountByReason: [String: Int64] = [:]
    private nonisolated(unsafe) var currentDrawableBusy = false
    private nonisolated(unsafe) var pendingSubmissionAtMilliseconds: [UInt64: Double] = [:]
    private nonisolated(unsafe) var submitToPresentSamples = StreamingBoundedRing<
        StreamingLatencySample
    >(capacity: 240)
    private nonisolated(unsafe) var gpuPercentiles: [Double] = []
    private nonisolated(unsafe) var drawableSamples = StreamingBoundedRing<StreamingLatencySample>(
        capacity: 240)
    private nonisolated(unsafe) var drawablePercentiles: [Double] = []
    private nonisolated(unsafe) var drawablePresentCount: Int64 = 0
    private nonisolated(unsafe) var lastDrawablePresentAtMs: Double?
    private nonisolated(unsafe) var configuration: StreamingDisplayConfiguration?

    private nonisolated(unsafe) var lastPercentileAtMs: Double = 0
    private nonisolated(unsafe) var scopeEpoch: UInt64 = 0
    private nonisolated(unsafe) var surfaceRevision: UInt64 = 0
    private nonisolated(unsafe) var active = true
    private nonisolated(unsafe) var surfaceReady = false
    private nonisolated(unsafe) var acquireAtMs: Double?
    private nonisolated(unsafe) var resourceCounters = StreamingRenderResourceCounters()
    private nonisolated(unsafe) var gpuEpisodeCount: Int64 = 0
    private nonisolated(unsafe) var drawableFrameInterval = StreamingFrameIntervalEstimator()
    private nonisolated(unsafe) var firstDisplayedAtMs: Double?
    private nonisolated(unsafe) var lastDisplayedFrameID: UInt64?

    @discardableResult
    func reset(preservePeerCounters: Bool = false) -> StreamingDisplayHealthSnapshot {
        lock.lock()
        if !preservePeerCounters { submitCount = 0 }
        configuration = nil
        if !preservePeerCounters { presentCount = 0 }
        lastSubmitAtMilliseconds = nil
        lastPresentAtMilliseconds = nil
        frameInterval = StreamingFrameIntervalEstimator()
        lastPresentDelta = nil
        if !preservePeerCounters {
            mailboxOverwriteCount = 0
            dropCountByReason = [:]
        }
        currentDrawableBusy = false
        pendingSubmissionAtMilliseconds = [:]
        submitToPresentSamples.removeAll()
        gpuPercentiles = []
        drawableSamples.removeAll()
        drawablePercentiles = []
        drawablePresentCount = 0
        lastDrawablePresentAtMs = nil
        lastPercentileAtMs = 0
        gpuEpisodeCount = 0
        drawableFrameInterval = .init()
        firstDisplayedAtMs = nil
        lastDisplayedFrameID = nil
        acquireAtMs = nil
        let result = makeSnapshotLocked()
        lock.unlock()
        return result
    }

    func recordMailboxOverwrite(totalOverwriteCount: Int64) -> StreamingDisplayHealthSnapshot {
        lock.lock()
        mailboxOverwriteCount = totalOverwriteCount
        let snapshot = makeSnapshotLocked()
        lock.unlock()
        return snapshot
    }

    func recordDrop(
        reason: StreamingVideoFrameDropReason,
        count: Int64 = 1
    ) -> StreamingDisplayHealthSnapshot {
        lock.lock()
        dropCountByReason[reason.rawValue, default: 0] += max(count, 1)
        let snapshot = makeSnapshotLocked()
        lock.unlock()
        return snapshot
    }

    func setDrawableBusy(_ busy: Bool) -> StreamingDisplayHealthSnapshot {
        lock.lock()
        currentDrawableBusy = busy
        let snapshot = makeSnapshotLocked()
        lock.unlock()
        return snapshot
    }

    func recordSubmit(
        identifier: UInt64,
        timestampMilliseconds: Double,
        configuration: StreamingDisplayConfiguration? = nil
    ) -> StreamingDisplayHealthSnapshot {
        lock.lock()
        submitCount += 1
        if let configuration { self.configuration = configuration }
        lastSubmitAtMilliseconds = timestampMilliseconds
        pendingSubmissionAtMilliseconds[identifier] = timestampMilliseconds
        let snapshot = makeSnapshotLocked()
        lock.unlock()
        return snapshot
    }

    func recordPresent(
        identifier: UInt64,
        timestampMilliseconds: Double,
        drawableBusy: Bool? = nil
    ) -> (snapshot: StreamingDisplayHealthSnapshot, isFirstPresent: Bool) {
        lock.lock()
        if let drawableBusy { currentDrawableBusy = drawableBusy }
        presentCount += 1
        #if targetEnvironment(simulator)
            if firstDisplayedAtMs == nil { firstDisplayedAtMs = timestampMilliseconds }
            lastDisplayedFrameID = identifier
        #endif
        lastPresentDelta = 1
        frameInterval.observe(timestampMilliseconds)
        lastPresentAtMilliseconds = timestampMilliseconds
        if let submittedAt = pendingSubmissionAtMilliseconds.removeValue(forKey: identifier) {
            let latency = max(0, timestampMilliseconds - submittedAt)
            submitToPresentSamples.append(.init(at: timestampMilliseconds, ms: latency))
            if submitToPresentSamples.count == 1 { gpuPercentiles = [latency] }
        }
        gpuEpisodeCount += 1
        let isFirstPresent = gpuEpisodeCount == 1
        let snapshot = makeSnapshotLocked()
        lock.unlock()
        return (snapshot, isFirstPresent)
    }

    func recordDrawablePresent(submittedAt: Double, presentedAt: Double, frameID: UInt64? = nil)
        -> StreamingDisplayHealthSnapshot
    {
        lock.lock()
        defer { lock.unlock() }
        guard presentedAt.isFinite, presentedAt >= submittedAt, submittedAt >= 0 else {
            return makeSnapshotLocked()
        }
        drawablePresentCount += 1
        if firstDisplayedAtMs == nil { firstDisplayedAtMs = presentedAt }
        lastDisplayedFrameID = frameID
        lastDrawablePresentAtMs = presentedAt
        drawableSamples.append(.init(at: presentedAt, ms: presentedAt - submittedAt))
        drawableFrameInterval.observe(presentedAt)
        if drawableSamples.count == 1 { drawablePercentiles = [presentedAt - submittedAt] }
        return makeSnapshotLocked()
    }

    func cancelSubmit(identifier: UInt64) {
        lock.lock()
        pendingSubmissionAtMilliseconds.removeValue(forKey: identifier)
        lock.unlock()
    }

    @discardableResult
    func cancelPendingSubmissions() -> StreamingDisplayHealthSnapshot {
        lock.lock()
        defer { lock.unlock() }
        pendingSubmissionAtMilliseconds.removeAll(keepingCapacity: true)
        currentDrawableBusy = false
        return makeSnapshotLocked()
    }

    private func makeSnapshotLocked() -> StreamingDisplayHealthSnapshot {
        let orderedLatencies = gpuPercentiles
        return StreamingDisplayHealthSnapshot(
            submitCount: submitCount,
            presentCount: presentCount,
            lastSubmitAtMilliseconds: lastSubmitAtMilliseconds,
            lastPresentAtMilliseconds: lastPresentAtMilliseconds,
            submitToPresentP50Milliseconds: percentile(orderedLatencies, percentile: 0.50),
            submitToPresentP95Milliseconds: percentile(orderedLatencies, percentile: 0.95),
            displayedFrameDelta: lastPresentDelta,
            mailboxOverwriteCount: mailboxOverwriteCount,
            dropCountByReason: dropCountByReason,
            currentDrawableBusy: currentDrawableBusy,
            frameIntervalMilliseconds: presentationSource == "drawablePresented"
                ? drawableFrameInterval.intervalMilliseconds : frameInterval.intervalMilliseconds,
            submitToPresentP99Milliseconds: percentile(orderedLatencies, percentile: 0.99),
            configuration: configuration,
            latencySampleCount: submitToPresentSamples.count,
            latencyWindowStartMs: submitToPresentSamples.first?.at,
            latencyWindowEndMs: submitToPresentSamples.last?.at,
            drawablePresentCount: drawablePresentCount,
            lastDrawablePresentAtMs: lastDrawablePresentAtMs,
            submitToDrawableP50Ms: percentile(drawablePercentiles, percentile: 0.50),
            submitToDrawableP95Ms: percentile(drawablePercentiles, percentile: 0.95),
            submitToDrawableP99Ms: percentile(drawablePercentiles, percentile: 0.99),
            drawableSampleCount: drawableSamples.count,
            drawableWindowStartMs: drawableSamples.first?.at,
            drawableWindowEndMs: drawableSamples.last?.at,
            presentationTimestampSource: presentationSource, renderEpoch: scopeEpoch,
            surfaceRevision: surfaceRevision, presentationActive: active,
            surfaceReady: surfaceReady,
            sampledAtMs: StreamingFrameFeedbackClock.nowMilliseconds,
            drawableAcquireStartedAtMs: acquireAtMs,
            oldestInflightAtMs: pendingSubmissionAtMilliseconds.values.min(),
            resourceCounters: resourceCounters,
            firstDisplayedAtMs: firstDisplayedAtMs, lastDisplayedFrameID: lastDisplayedFrameID
        )
    }

    private var presentationSource: String {
        #if targetEnvironment(simulator)
            "gpuProxy"
        #else
            "drawablePresented"
        #endif
    }

    func setPresentationScope(
        epoch: UInt64, surfaceRevision: UInt64, active: Bool, surfaceReady: Bool
    ) {
        lock.withLock {
            scopeEpoch = epoch
            self.surfaceRevision = surfaceRevision
            self.active = active
            self.surfaceReady = surfaceReady
        }
    }

    func setDrawableAcquire(_ startedAt: Double?) { lock.withLock { acquireAtMs = startedAt } }
    func setResources(_ counters: StreamingRenderResourceCounters) {
        lock.withLock { resourceCounters = counters }
    }
    func currentSnapshot() -> StreamingDisplayHealthSnapshot {
        lock.withLock { makeSnapshotLocked() }
    }

    func restorePeerCounters(_ snapshot: StreamingDisplayHealthSnapshot) {
        lock.withLock {
            submitCount = max(submitCount, snapshot.submitCount)
            presentCount = max(presentCount, snapshot.presentCount)
        }
    }

    /// 只由 utility DiagnosticsActor 以 1Hz 重算分位数，排序在短锁外完成。
    func snapshot(at now: Double) -> StreamingDisplayHealthSnapshot {
        let batch:
            (epoch: UInt64, gpu: [StreamingLatencySample], drawable: [StreamingLatencySample])? =
                lock.withLock {
                    guard now - lastPercentileAtMs >= 1_000 || lastPercentileAtMs == 0 else {
                        return nil
                    }
                    lastPercentileAtMs = now
                    return (scopeEpoch, submitToPresentSamples.values(), drawableSamples.values())
                }
        if let batch {
            let gpu = batch.gpu.map(\.ms).sorted()
            let drawable = batch.drawable.map(\.ms).sorted()
            lock.withLock {
                if scopeEpoch == batch.epoch {
                    gpuPercentiles = gpu
                    drawablePercentiles = drawable
                }
            }
        }
        return currentSnapshot()
    }

    private func percentile(_ values: [Double], percentile: Double) -> Double? {
        guard !values.isEmpty else { return nil }
        let clamped = max(0, min(1, percentile))
        let index = Int(Double(values.count - 1) * clamped)
        return values[index]
    }
}

struct StreamingVertexUniforms: Sendable {
    var scale: SIMD2<Float>
    var padding: SIMD2<Float> = .zero
    var texCoordTransform: simd_float3x3
}

struct StreamingFragmentUniforms: Sendable {
    var colorMatrix: simd_float3x3
    var colorOffset: SIMD3<Float>
    var padding: Float = 0
    var effects = SIMD4<Float>(1, 1, 1, 0)
}

struct StreamingLatestIngressDelivery<Value>: @unchecked Sendable {
    let value: Value
    let replacedCount: Int64
}

struct StreamingIngressDrainTicket: Equatable, Sendable {
    let identifier: UInt64
    let scheduledAtMs: Double
}

final class StreamingLatestIngressSlot<Value>: @unchecked Sendable {
    private let lock = NSLock()
    private var pendingValue: Value?
    private var scheduledDrain: StreamingIngressDrainTicket?
    private var nextDrainIdentifier: UInt64 = 0
    private var replacedCount: Int64 = 0

    func submit(
        _ value: Value, scheduledAtMs: Double = StreamingFrameFeedbackClock.nowMilliseconds
    ) -> StreamingIngressDrainTicket? {
        lock.lock()
        defer { lock.unlock() }
        if pendingValue != nil { replacedCount += 1 }
        pendingValue = value
        guard scheduledDrain == nil else { return nil }
        return makeDrainTicket(scheduledAtMs: scheduledAtMs)
    }

    func take(_ ticket: StreamingIngressDrainTicket) -> StreamingLatestIngressDelivery<Value>? {
        lock.lock()
        defer { lock.unlock() }
        guard scheduledDrain?.identifier == ticket.identifier, let pendingValue else { return nil }
        let delivery = StreamingLatestIngressDelivery(
            value: pendingValue,
            replacedCount: replacedCount
        )
        self.pendingValue = nil
        replacedCount = 0
        return delivery
    }

    func finishDelivery(
        _ ticket: StreamingIngressDrainTicket,
        scheduledAtMs: Double = StreamingFrameFeedbackClock.nowMilliseconds
    ) -> StreamingIngressDrainTicket? {
        lock.lock()
        defer { lock.unlock() }
        guard scheduledDrain?.identifier == ticket.identifier else { return nil }
        guard pendingValue != nil else {
            scheduledDrain = nil
            return nil
        }
        return makeDrainTicket(scheduledAtMs: scheduledAtMs)
    }

    func drain(
        _ ticket: StreamingIngressDrainTicket,
        consume: (StreamingLatestIngressDelivery<Value>) -> Void
    ) -> StreamingIngressDrainTicket? {
        // 同次处理接续最多两帧，减少补送排队并保留 UI 的执行机会。
        for _ in 0..<2 {
            guard let delivery = take(ticket) else { break }
            consume(delivery)
        }
        return finishDelivery(ticket)
    }

    func takePending() -> StreamingLatestIngressDelivery<Value>? {
        lock.withLock {
            guard let pendingValue else { return nil }
            let result = StreamingLatestIngressDelivery(
                value: pendingValue, replacedCount: replacedCount)
            self.pendingValue = nil
            scheduledDrain = nil
            replacedCount = 0
            return result
        }
    }

    // 调用方持锁；每次续排都换 ticket，隔离 reset 和重复投递的旧任务。
    private func makeDrainTicket(scheduledAtMs: Double) -> StreamingIngressDrainTicket {
        nextDrainIdentifier &+= 1
        let ticket = StreamingIngressDrainTicket(
            identifier: nextDrainIdentifier, scheduledAtMs: scheduledAtMs)
        scheduledDrain = ticket
        return ticket
    }

    func reset() {
        lock.lock()
        pendingValue = nil
        scheduledDrain = nil
        replacedCount = 0
        lock.unlock()
    }
}
