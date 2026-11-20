import Foundation
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
            "frameIntervalSource": .string("metalPresentCompletion"),
        ]
    }
}
private enum StreamingVideoFrameDropReason: String {
    case ingressCoalesced
    case unsupportedBufferType
    case unsupportedPixelFormat
    case textureImportFailed
    case drawableUnavailable
    case encoderUnavailable
    case pipelineUnavailable
}
private final class StreamingDisplayTelemetry {
    private let lock = NSLock()
    private var submitCount: Int64 = 0
    private var presentCount: Int64 = 0
    private var lastSubmitAtMilliseconds: Double?
    private var lastPresentAtMilliseconds: Double?
    private var frameInterval = StreamingFrameIntervalEstimator()
    private var lastReportedPresentCount: Int64 = 0
    private var mailboxOverwriteCount: Int64 = 0
    private var dropCountByReason: [String: Int64] = [:]
    private var currentDrawableBusy = false
    private var pendingSubmissionAtMilliseconds: [UInt64: Double] = [:]
    private var submitToPresentSamples: [Double] = []
    private var configuration: StreamingDisplayConfiguration?

    func reset() -> StreamingDisplayHealthSnapshot {
        lock.lock()
        submitCount = 0
        configuration = nil
        presentCount = 0
        lastSubmitAtMilliseconds = nil
        lastPresentAtMilliseconds = nil
        frameInterval = StreamingFrameIntervalEstimator()
        lastReportedPresentCount = 0
        mailboxOverwriteCount = 0
        dropCountByReason = [:]
        currentDrawableBusy = false
        pendingSubmissionAtMilliseconds = [:]
        submitToPresentSamples = []
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
        frameInterval.observe(timestampMilliseconds)
        lastPresentAtMilliseconds = timestampMilliseconds
        if let submittedAt = pendingSubmissionAtMilliseconds.removeValue(forKey: identifier) {
            let latency = max(0, timestampMilliseconds - submittedAt)
            submitToPresentSamples.append(latency)
            if submitToPresentSamples.count > 240 {
                submitToPresentSamples.removeFirst(submitToPresentSamples.count - 240)
            }
        }
        let isFirstPresent = presentCount == 1
        let snapshot = makeSnapshotLocked()
        lock.unlock()
        return (snapshot, isFirstPresent)
    }

    func cancelSubmit(identifier: UInt64) {
        lock.lock()
        pendingSubmissionAtMilliseconds.removeValue(forKey: identifier)
        lock.unlock()
    }

    private func makeSnapshotLocked() -> StreamingDisplayHealthSnapshot {
        let orderedLatencies = submitToPresentSamples.sorted()
        let displayedFrameDelta = presentCount - lastReportedPresentCount
        lastReportedPresentCount = presentCount
        return StreamingDisplayHealthSnapshot(
            submitCount: submitCount,
            presentCount: presentCount,
            lastSubmitAtMilliseconds: lastSubmitAtMilliseconds,
            lastPresentAtMilliseconds: lastPresentAtMilliseconds,
            submitToPresentP50Milliseconds: percentile(orderedLatencies, percentile: 0.50),
            submitToPresentP95Milliseconds: percentile(orderedLatencies, percentile: 0.95),
            displayedFrameDelta: displayedFrameDelta > 0 ? displayedFrameDelta : nil,
            mailboxOverwriteCount: mailboxOverwriteCount,
            dropCountByReason: dropCountByReason,
            currentDrawableBusy: currentDrawableBusy,
            frameIntervalMilliseconds: frameInterval.intervalMilliseconds,
            submitToPresentP99Milliseconds: percentile(orderedLatencies, percentile: 0.99),
            configuration: configuration
        )
    }

    private func percentile(_ values: [Double], percentile: Double) -> Double? {
        guard !values.isEmpty else { return nil }
        let clamped = max(0, min(1, percentile))
        let index = Int(Double(values.count - 1) * clamped)
        return values[index]
    }
}
@main struct TelemetryAudit {
    static func main() throws {
        let telemetry = StreamingDisplayTelemetry()
        _ = telemetry.recordSubmit(identifier: 1, timestampMilliseconds: 1000)
        let presented = telemetry.recordPresent(identifier: 1, timestampMilliseconds: 1002).snapshot
        let latest = telemetry.setDrawableBusy(false)
        let result: [String: Any] = ["presentSnapshotDelta": presented.displayedFrameDelta as Any? ?? NSNull(), "latestSnapshotDelta": latest.displayedFrameDelta as Any? ?? NSNull(), "latestPresentCount": latest.presentCount, "p95Ms": latest.submitToPresentP95Milliseconds as Any? ?? NSNull()]
        let data = try JSONSerialization.data(withJSONObject: result, options: [.sortedKeys, .prettyPrinted])
        try data.write(to: URL(fileURLWithPath: "/private/tmp/xbxrc-ios-trace-audit-20261120/r1/telemetry-probe.json"))
        print(String(decoding: data, as: UTF8.self))
    }
}
