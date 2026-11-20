import Foundation
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
            "frameIntervalSource": .string("commandBufferCompletion"),
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
enum StreamingVideoFrameDropReason: String {
    case ingressCoalesced
    case unsupportedBufferType
    case unsupportedPixelFormat
    case textureImportFailed
    case drawableUnavailable
    case encoderUnavailable
    case pipelineUnavailable
}
final class StreamingDisplayTelemetry {
    private let lock = NSLock()
    private var submitCount: Int64 = 0
    private var presentCount: Int64 = 0
    private var lastSubmitAtMilliseconds: Double?
    private var lastPresentAtMilliseconds: Double?
    private var frameInterval = StreamingFrameIntervalEstimator()
    private var lastPresentDelta: Int64?
    private var mailboxOverwriteCount: Int64 = 0
    private var dropCountByReason: [String: Int64] = [:]
    private var currentDrawableBusy = false
    private var pendingSubmissionAtMilliseconds: [UInt64: Double] = [:]
    private var submitToPresentSamples: [(at: Double, ms: Double)] = []
    private var gpuPercentiles: [Double] = []
    private var drawableSamples: [(at: Double, ms: Double)] = []
    private var drawablePercentiles: [Double] = []
    private var drawablePresentCount: Int64 = 0
    private var lastDrawablePresentAtMs: Double?
    private var configuration: StreamingDisplayConfiguration?

    func reset() -> StreamingDisplayHealthSnapshot {
        lock.lock()
        submitCount = 0
        configuration = nil
        presentCount = 0
        lastSubmitAtMilliseconds = nil
        lastPresentAtMilliseconds = nil
        frameInterval = StreamingFrameIntervalEstimator()
        lastPresentDelta = nil
        mailboxOverwriteCount = 0
        dropCountByReason = [:]
        currentDrawableBusy = false
        pendingSubmissionAtMilliseconds = [:]
        submitToPresentSamples = []
        gpuPercentiles = []
        drawableSamples = []
        drawablePercentiles = []
        drawablePresentCount = 0
        lastDrawablePresentAtMs = nil
        lock.unlock()
        return .zero
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
        timestampMilliseconds: Double
    ) -> (snapshot: StreamingDisplayHealthSnapshot, isFirstPresent: Bool) {
        lock.lock()
        presentCount += 1
        lastPresentDelta = 1
        frameInterval.observe(timestampMilliseconds)
        lastPresentAtMilliseconds = timestampMilliseconds
        if let submittedAt = pendingSubmissionAtMilliseconds.removeValue(forKey: identifier) {
            let latency = max(0, timestampMilliseconds - submittedAt)
            submitToPresentSamples.append((timestampMilliseconds, latency))
            if submitToPresentSamples.count > 240 {
                submitToPresentSamples.removeFirst(submitToPresentSamples.count - 240)
            }
        }
        gpuPercentiles = submitToPresentSamples.map(\.ms).sorted()
        let isFirstPresent = presentCount == 1
        let snapshot = makeSnapshotLocked()
        lock.unlock()
        return (snapshot, isFirstPresent)
    }

    func recordDrawablePresent(submittedAt: Double, presentedAt: Double)
        -> StreamingDisplayHealthSnapshot
    {
        lock.lock()
        defer { lock.unlock() }
        guard presentedAt.isFinite, presentedAt >= submittedAt, submittedAt >= 0 else {
            return makeSnapshotLocked()
        }
        drawablePresentCount += 1
        lastDrawablePresentAtMs = presentedAt
        drawableSamples.append((presentedAt, presentedAt - submittedAt))
        if drawableSamples.count > 240 { drawableSamples.removeFirst(drawableSamples.count - 240) }
        drawablePercentiles = drawableSamples.map(\.ms).sorted()
        return makeSnapshotLocked()
    }

    func cancelSubmit(identifier: UInt64) {
        lock.lock()
        pendingSubmissionAtMilliseconds.removeValue(forKey: identifier)
        lock.unlock()
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
            frameIntervalMilliseconds: frameInterval.intervalMilliseconds,
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
            drawableWindowEndMs: drawableSamples.last?.at
        )
    }

    private func percentile(_ values: [Double], percentile: Double) -> Double? {
        guard !values.isEmpty else { return nil }
        let clamped = max(0, min(1, percentile))
        let index = Int(Double(values.count - 1) * clamped)
        return values[index]
    }
}
struct StreamingVideoPacketStatistics: Sendable {
    private(set) var received: UInt64 = 0
    private(set) var lost: Int64 = 0
    private var allLossObserved = true
    private var hasVideo = false
    var observedLoss: Int64? { hasVideo && allLossObserved ? lost : nil }

    mutating func observe(kind: String?, received: UInt64, lost: Int64?) {
        guard kind == "video" else { return }
        hasVideo = true
        self.received += received
        if let lost { self.lost += lost } else { allLossObserved = false }
    }
}
