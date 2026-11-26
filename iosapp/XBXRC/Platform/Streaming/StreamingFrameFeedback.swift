import Foundation
import QuartzCore
@preconcurrency import WebRTC

enum StreamingFrameFeedbackClock {
    static var nowMilliseconds: Double { CACurrentMediaTime() * 1_000 }
}

struct StreamingFrameDecodeTiming: Equatable, Sendable {
    let serverDataKey: UInt32
    let submittedAtMilliseconds: Double
    let decodedAtMilliseconds: Double
}

struct StreamingPresentedFrameFeedback: Equatable, Sendable {
    let timing: StreamingFrameDecodeTiming
    let renderedAtMilliseconds: Double

    var metadata: XboxStreamFrameFeedback {
        // SDK 的可观测接收边界是完整编码帧进入解码器，原始网络到包事实由 RTCP 统计保留。
        XboxStreamFrameFeedback(
            serverDataKey: timing.serverDataKey,
            arrivalTimeMs: timing.submittedAtMilliseconds,
            submittedTimeMs: timing.submittedAtMilliseconds,
            decodedTimeMs: timing.decodedAtMilliseconds,
            renderedTimeMs: renderedAtMilliseconds)
    }
}

struct StreamingFrameFeedbackTicket: Sendable {
    let session: StreamingFrameFeedbackSession
    let timing: StreamingFrameDecodeTiming
}

struct StreamingFrameFeedbackSnapshot: Equatable, Sendable {
    let isActive: Bool
    let preventResolutionDrops: Bool
    let decodeObservationCount: Int
    let pendingFrameCount: Int
    let discardedDecodeCount: Int
    let discardedFeedbackCount: Int
    let sentFrameCount: Int
    let lastDecodeDurationMilliseconds: Double?
    let lastDecodedToRenderMilliseconds: Double?
    let lastWireDecodeDurationMilliseconds: Double?
    let lastWireDecodedToRenderMilliseconds: Double?
    var decoderCreateCount = 0
    var decoderReleaseCount = 0
    var decoderImplementation: String?

    func tracePayload() -> [String: IOSRuntimeTraceValue] {
        [
            "preventResolutionDrops": .bool(preventResolutionDrops),
            "decoderCreateCount": .integer(Int64(decoderCreateCount)),
            "decoderReleaseCount": .integer(Int64(decoderReleaseCount)),
            "decoderImplementation": decoderImplementation.map(IOSRuntimeTraceValue.string)
                ?? .null,
            "arrivalSource": .string("encodedFrameIngress"),
            "renderSource": .string("commandBufferCompletion"),
            "decodeObservationCount": .integer(Int64(decodeObservationCount)),
            "pendingFrameCount": .integer(Int64(pendingFrameCount)),
            "discardedDecodeCount": .integer(Int64(discardedDecodeCount)),
            "discardedFeedbackCount": .integer(Int64(discardedFeedbackCount)),
            "sentFrameCount": .integer(Int64(sentFrameCount)),
            "rawDecodeMs": lastDecodeDurationMilliseconds.map(IOSRuntimeTraceValue.double) ?? .null,
            "rawDecodedToRenderMs": lastDecodedToRenderMilliseconds.map(
                IOSRuntimeTraceValue.double) ?? .null,
            "wireDecodeMs": lastWireDecodeDurationMilliseconds.map(IOSRuntimeTraceValue.double)
                ?? .null,
            "wireDecodedToRenderMs": lastWireDecodedToRenderMilliseconds.map(
                IOSRuntimeTraceValue.double) ?? .null,
        ]
    }
}

/// 当前 peer 独占固定 timing 槽位和 feedback ring；SDK 回调只更新有界事实。
final class StreamingFrameFeedbackSession: @unchecked Sendable {
    let preventResolutionDrops: Bool
    static let maximumDecodeObservations = 128
    static let maximumPendingFrames = 32
    static let maximumFrameAgeMilliseconds: Double = 500

    private struct DecodeObservation {
        let key: UInt32
        let submittedAtMilliseconds: Double
        var decodedAtMilliseconds: Double?
    }

    private let lock = NSLock()
    private var active = true
    private var slots: [DecodeObservation?] = Array(
        repeating: nil, count: maximumDecodeObservations)
    private var indices: [UInt32: Int] = [:]
    private var freeSlots = Array((0..<maximumDecodeObservations).reversed())
    private var pending = StreamingBoundedRing<StreamingPresentedFrameFeedback>(
        capacity: maximumPendingFrames)
    private var discardedDecodeCount = 0
    private var discardedFeedbackCount = 0
    private var sentFrameCount = 0
    private var decoderCreateCount = 0
    private var decoderReleaseCount = 0
    private var lastDecoderImplementation: String?
    private var lastDecodeDurationMilliseconds: Double?
    private var lastDecodedToRenderMilliseconds: Double?
    private var lastWireDecodeDurationMilliseconds: Double?
    private var lastWireDecodedToRenderMilliseconds: Double?
    private var availableHandler: (@Sendable () -> Void)?

    init(preventResolutionDrops: Bool = false) {
        self.preventResolutionDrops = preventResolutionDrops
        indices.reserveCapacity(Self.maximumDecodeObservations)
        freeSlots.reserveCapacity(Self.maximumDecodeObservations)
    }

    func setAvailableHandler(_ handler: (@Sendable () -> Void)?) {
        lock.withLock { availableHandler = handler }
    }

    func recordDecoderCreated(implementation: String) {
        lock.withLock {
            decoderCreateCount += 1
            lastDecoderImplementation = implementation
        }
    }

    func recordDecoderReleased() { lock.withLock { decoderReleaseCount += 1 } }

    func decodeStarted(serverDataKey: UInt32, atMilliseconds: Double) {
        guard validTime(atMilliseconds) else { return }
        lock.withLock {
            guard active else { return }
            expire(atMilliseconds: atMilliseconds)
            if removeObservation(serverDataKey) != nil { discardedDecodeCount += 1 }
            if freeSlots.isEmpty {
                var oldest: Int?
                for index in slots.indices where slots[index] != nil {
                    if oldest == nil
                        || slots[index]!.submittedAtMilliseconds
                            < slots[oldest!]!.submittedAtMilliseconds
                    {
                        oldest = index
                    }
                }
                if let oldest {
                    _ = removeObservation(slots[oldest]!.key)
                    discardedDecodeCount += 1
                }
            }
            guard let index = freeSlots.popLast() else { return }
            indices[serverDataKey] = index
            slots[index] = DecodeObservation(
                key: serverDataKey, submittedAtMilliseconds: atMilliseconds)
        }
    }

    func decodeCompleted(serverDataKey: UInt32, atMilliseconds: Double) {
        guard validTime(atMilliseconds) else { return }
        lock.withLock {
            guard active, let index = indices[serverDataKey], var observation = slots[index],
                observation.decodedAtMilliseconds == nil,
                atMilliseconds >= observation.submittedAtMilliseconds,
                atMilliseconds - observation.submittedAtMilliseconds
                    <= Self.maximumFrameAgeMilliseconds
            else { return }
            observation.decodedAtMilliseconds = atMilliseconds
            slots[index] = observation
            lastDecodeDurationMilliseconds = atMilliseconds - observation.submittedAtMilliseconds
        }
    }

    func decodeFailed(serverDataKey: UInt32) {
        lock.withLock { if removeObservation(serverDataKey) != nil { discardedDecodeCount += 1 } }
    }

    func takeDecodedTiming(serverDataKey: UInt32, nowMilliseconds: Double)
        -> StreamingFrameDecodeTiming?
    {
        guard validTime(nowMilliseconds) else { return nil }
        return lock.withLock {
            guard active else { return nil }
            expire(atMilliseconds: nowMilliseconds)
            guard let index = indices[serverDataKey], let observation = slots[index],
                let decoded = observation.decodedAtMilliseconds, nowMilliseconds >= decoded
            else { return nil }
            _ = removeObservation(serverDataKey)
            return .init(
                serverDataKey: serverDataKey,
                submittedAtMilliseconds: observation.submittedAtMilliseconds,
                decodedAtMilliseconds: decoded)
        }
    }

    func recordRendered(timing: StreamingFrameDecodeTiming, atMilliseconds: Double) {
        guard validTime(atMilliseconds), validTime(timing.submittedAtMilliseconds),
            validTime(timing.decodedAtMilliseconds),
            timing.decodedAtMilliseconds >= timing.submittedAtMilliseconds,
            atMilliseconds >= timing.decodedAtMilliseconds,
            atMilliseconds - timing.submittedAtMilliseconds <= Self.maximumFrameAgeMilliseconds
        else { return }
        let handler = lock.withLock { () -> (@Sendable () -> Void)? in
            guard active else { return nil }
            expire(atMilliseconds: atMilliseconds)
            if pending.append(.init(timing: timing, renderedAtMilliseconds: atMilliseconds)) != nil
            {
                discardedFeedbackCount += 1
            }
            lastDecodeDurationMilliseconds =
                timing.decodedAtMilliseconds - timing.submittedAtMilliseconds
            lastDecodedToRenderMilliseconds = atMilliseconds - timing.decodedAtMilliseconds
            return availableHandler
        }
        handler?()
    }

    func copyPendingFrames(
        into result: inout [StreamingPresentedFrameFeedback], maximumCount: Int,
        nowMilliseconds: Double
    ) {
        result.removeAll(keepingCapacity: true)
        guard validTime(nowMilliseconds), maximumCount > 0 else { return }
        lock.withLock {
            guard active else { return }
            expire(atMilliseconds: nowMilliseconds)
            result.reserveCapacity(min(maximumCount, 29))
            pending.forEach(maximumCount: min(maximumCount, 29)) {
                if $0.renderedAtMilliseconds <= nowMilliseconds { result.append($0) }
            }
        }
    }

    func pendingFrames(maximumCount: Int, nowMilliseconds: Double)
        -> [StreamingPresentedFrameFeedback]
    {
        var result: [StreamingPresentedFrameFeedback] = []
        copyPendingFrames(
            into: &result, maximumCount: maximumCount, nowMilliseconds: nowMilliseconds)
        return result
    }

    func markSent(_ frames: [StreamingPresentedFrameFeedback], wirePayload: Data? = nil) {
        lock.withLock {
            guard active else { return }
            for _ in 0..<pending.count {
                guard let value = pending.removeFirst() else { break }
                if !frames.contains(value) { pending.append(value) }
            }
            sentFrameCount += frames.count
            if let frame = frames.last, let wirePayload, wirePayload.count >= 15 + frames.count * 28
            {
                let offset = 15 + (frames.count - 1) * 28
                func word(_ position: Int) -> UInt32 {
                    (0..<4).reduce(UInt32(0)) {
                        $0 | UInt32(wirePayload[position + $1]) << (8 * $1)
                    }
                }
                if word(offset) == frame.timing.serverDataKey {
                    lastWireDecodeDurationMilliseconds = Double(
                        word(offset + 12) &- word(offset + 8))
                    lastWireDecodedToRenderMilliseconds = Double(
                        word(offset + 16) &- word(offset + 12))
                }
            }
        }
    }

    func invalidate() {
        lock.withLock {
            active = false
            availableHandler = nil
            for index in slots.indices { slots[index] = nil }
            indices.removeAll(keepingCapacity: true)
            pending.removeAll()
        }
    }

    func snapshot() -> StreamingFrameFeedbackSnapshot {
        lock.withLock {
            var result = StreamingFrameFeedbackSnapshot(
                isActive: active, preventResolutionDrops: preventResolutionDrops,
                decodeObservationCount: indices.count, pendingFrameCount: pending.count,
                discardedDecodeCount: discardedDecodeCount,
                discardedFeedbackCount: discardedFeedbackCount,
                sentFrameCount: sentFrameCount,
                lastDecodeDurationMilliseconds: lastDecodeDurationMilliseconds,
                lastDecodedToRenderMilliseconds: lastDecodedToRenderMilliseconds,
                lastWireDecodeDurationMilliseconds: lastWireDecodeDurationMilliseconds,
                lastWireDecodedToRenderMilliseconds: lastWireDecodedToRenderMilliseconds)
            result.decoderCreateCount = decoderCreateCount
            result.decoderReleaseCount = decoderReleaseCount
            result.decoderImplementation = lastDecoderImplementation
            return result
        }
    }

    private func removeObservation(_ key: UInt32) -> DecodeObservation? {
        guard let index = indices.removeValue(forKey: key) else { return nil }
        let value = slots[index]
        slots[index] = nil
        freeSlots.append(index)
        return value
    }

    private func expire(atMilliseconds: Double) {
        for index in slots.indices {
            if let value = slots[index],
                atMilliseconds - value.submittedAtMilliseconds > Self.maximumFrameAgeMilliseconds
            {
                _ = removeObservation(value.key)
                discardedDecodeCount += 1
            }
        }
        for _ in 0..<pending.count {
            guard let value = pending.removeFirst() else { break }
            if atMilliseconds - value.timing.submittedAtMilliseconds
                > Self.maximumFrameAgeMilliseconds
            {
                discardedFeedbackCount += 1
            } else {
                pending.append(value)
            }
        }
    }

    private func validTime(_ time: Double) -> Bool { time.isFinite && time >= 0 }
}

/// 转发同一个默认解码器，持续观测当前 peer 的提交和解码回调。
final class StreamingFeedbackVideoDecoderFactory: NSObject, RTCVideoDecoderFactory,
    @unchecked Sendable
{
    private let base = RTCDefaultVideoDecoderFactory()
    private let feedback: StreamingFrameFeedbackSession

    init(feedback: StreamingFrameFeedbackSession) { self.feedback = feedback }

    func supportedCodecs() -> [RTCVideoCodecInfo] { base.supportedCodecs() }

    func createDecoder(_ info: RTCVideoCodecInfo) -> (any RTCVideoDecoder)? {
        guard let decoder = base.createDecoder(info) else { return nil }
        feedback.recordDecoderCreated(implementation: decoder.implementationName())
        return StreamingFeedbackVideoDecoder(base: decoder, feedback: feedback)
    }
}

final class StreamingFeedbackVideoDecoder: NSObject, RTCVideoDecoder, @unchecked Sendable {
    private let base: any RTCVideoDecoder
    private let feedback: StreamingFrameFeedbackSession

    init(base: any RTCVideoDecoder, feedback: StreamingFrameFeedbackSession) {
        self.base = base
        self.feedback = feedback
    }

    func setCallback(_ callback: @escaping RTCVideoDecoderCallback) {
        base.setCallback { [weak self] frame in
            self?.feedback.decodeCompleted(
                serverDataKey: UInt32(bitPattern: frame.timeStamp),
                atMilliseconds: StreamingFrameFeedbackClock.nowMilliseconds)
            callback(frame)
        }
    }

    func startDecode(withNumberOfCores numberOfCores: Int32) -> Int {
        base.startDecode(withNumberOfCores: numberOfCores)
    }

    func release() -> Int {
        let result = base.release()
        feedback.recordDecoderReleased()
        return result
    }

    func decode(
        _ encodedImage: RTCEncodedImage, missingFrames: Bool,
        codecSpecificInfo info: (any RTCCodecSpecificInfo)?, renderTimeMs: Int64
    ) -> Int {
        feedback.decodeStarted(
            serverDataKey: encodedImage.timeStamp,
            atMilliseconds: StreamingFrameFeedbackClock.nowMilliseconds)
        let result = base.decode(
            encodedImage, missingFrames: missingFrames, codecSpecificInfo: info,
            renderTimeMs: renderTimeMs)
        if result != 0 { feedback.decodeFailed(serverDataKey: encodedImage.timeStamp) }
        return result
    }

    func implementationName() -> String { base.implementationName() }
}
