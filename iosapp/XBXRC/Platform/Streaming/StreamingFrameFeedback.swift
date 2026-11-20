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

    func tracePayload() -> [String: IOSRuntimeTraceValue] {
        [
            "preventResolutionDrops": .bool(preventResolutionDrops),
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
            "wireDecodeMs": lastWireDecodeDurationMilliseconds.map(IOSRuntimeTraceValue.double) ?? .null,
            "wireDecodedToRenderMs": lastWireDecodedToRenderMilliseconds.map(IOSRuntimeTraceValue.double) ?? .null,
        ]
    }
}

/// 当前 peer 独占的有界观测与发送队列；绘制完成回调和 input 轮询共用单调时钟。
final class StreamingFrameFeedbackSession: @unchecked Sendable {
    let preventResolutionDrops: Bool
    static let maximumDecodeObservations = 128
    static let maximumPendingFrames = 32
    static let maximumFrameAgeMilliseconds: Double = 500

    private struct DecodeObservation {
        let submittedAtMilliseconds: Double
        var decodedAtMilliseconds: Double?
    }

    private let lock = NSLock()
    private var active = true
    private var observations: [UInt32: DecodeObservation] = [:]
    private var pending: [StreamingPresentedFrameFeedback] = []
    private var discardedDecodeCount = 0
    private var discardedFeedbackCount = 0
    private var sentFrameCount = 0
    private var lastDecodeDurationMilliseconds: Double?
    private var lastDecodedToRenderMilliseconds: Double?
    private var lastWireDecodeDurationMilliseconds: Double?
    private var lastWireDecodedToRenderMilliseconds: Double?

    init(preventResolutionDrops: Bool = false) { self.preventResolutionDrops = preventResolutionDrops }

    func decodeStarted(serverDataKey: UInt32, atMilliseconds: Double) {
        guard validTime(atMilliseconds) else { return }
        lock.lock()
        defer { lock.unlock() }
        guard active else { return }
        expire(atMilliseconds: atMilliseconds)
        if observations[serverDataKey] != nil { discardedDecodeCount += 1 }
        if observations.count >= Self.maximumDecodeObservations,
            let oldest = observations.min(by: {
                $0.value.submittedAtMilliseconds < $1.value.submittedAtMilliseconds
            })
        {
            observations.removeValue(forKey: oldest.key)
            discardedDecodeCount += 1
        }
        observations[serverDataKey] = DecodeObservation(submittedAtMilliseconds: atMilliseconds)
    }

    func decodeCompleted(serverDataKey: UInt32, atMilliseconds: Double) {
        guard validTime(atMilliseconds) else { return }
        lock.lock()
        defer { lock.unlock() }
        guard active, var observation = observations[serverDataKey],
            observation.decodedAtMilliseconds == nil,
            atMilliseconds >= observation.submittedAtMilliseconds,
            atMilliseconds - observation.submittedAtMilliseconds <= Self.maximumFrameAgeMilliseconds
        else { return }
        observation.decodedAtMilliseconds = atMilliseconds
        observations[serverDataKey] = observation
        lastDecodeDurationMilliseconds = atMilliseconds - observation.submittedAtMilliseconds
    }

    func decodeFailed(serverDataKey: UInt32) {
        lock.lock()
        defer { lock.unlock() }
        if observations.removeValue(forKey: serverDataKey) != nil { discardedDecodeCount += 1 }
    }

    func takeDecodedTiming(serverDataKey: UInt32, nowMilliseconds: Double)
        -> StreamingFrameDecodeTiming?
    {
        guard validTime(nowMilliseconds) else { return nil }
        lock.lock()
        defer { lock.unlock() }
        guard active else { return nil }
        expire(atMilliseconds: nowMilliseconds)
        guard let observation = observations[serverDataKey],
            let decoded = observation.decodedAtMilliseconds, nowMilliseconds >= decoded
        else { return nil }
        observations.removeValue(forKey: serverDataKey)
        return StreamingFrameDecodeTiming(
            serverDataKey: serverDataKey,
            submittedAtMilliseconds: observation.submittedAtMilliseconds,
            decodedAtMilliseconds: decoded)
    }

    func recordRendered(timing: StreamingFrameDecodeTiming, atMilliseconds: Double) {
        guard validTime(atMilliseconds), validTime(timing.submittedAtMilliseconds),
            validTime(timing.decodedAtMilliseconds),
            timing.decodedAtMilliseconds >= timing.submittedAtMilliseconds,
            atMilliseconds >= timing.decodedAtMilliseconds,
            atMilliseconds - timing.submittedAtMilliseconds <= Self.maximumFrameAgeMilliseconds
        else { return }
        lock.lock()
        defer { lock.unlock() }
        guard active else { return }
        expire(atMilliseconds: atMilliseconds)
        pending.append(
            StreamingPresentedFrameFeedback(timing: timing, renderedAtMilliseconds: atMilliseconds)
        )
        if pending.count > Self.maximumPendingFrames {
            let count = pending.count - Self.maximumPendingFrames
            pending.removeFirst(count)
            discardedFeedbackCount += count
        }
        lastDecodeDurationMilliseconds =
            timing.decodedAtMilliseconds - timing.submittedAtMilliseconds
        lastDecodedToRenderMilliseconds = atMilliseconds - timing.decodedAtMilliseconds
    }

    func pendingFrames(maximumCount: Int, nowMilliseconds: Double)
        -> [StreamingPresentedFrameFeedback]
    {
        guard validTime(nowMilliseconds), maximumCount > 0 else { return [] }
        lock.lock()
        defer { lock.unlock() }
        guard active else { return [] }
        expire(atMilliseconds: nowMilliseconds)
        return Array(
            pending.filter { $0.renderedAtMilliseconds <= nowMilliseconds }.prefix(
                min(maximumCount, 29)))
    }

    func markSent(_ frames: [StreamingPresentedFrameFeedback], wirePayload: Data? = nil) {
        lock.lock()
        defer { lock.unlock() }
        guard active else { return }
        pending.removeAll { frames.contains($0) }
        sentFrameCount += frames.count
        if let frame = frames.last, let wirePayload, wirePayload.count >= 15 + frames.count * 28 {
            let offset = 15 + (frames.count - 1) * 28
            func word(_ position: Int) -> UInt32 {
                (0..<4).reduce(UInt32(0)) { $0 | UInt32(wirePayload[position + $1]) << (8 * $1) }
            }
            if word(offset) == frame.timing.serverDataKey {
                // 诊断读取实际发送字节，时间回绕沿公共wire的u32毫秒事实处理。
                lastWireDecodeDurationMilliseconds = Double(word(offset + 12) &- word(offset + 8))
                lastWireDecodedToRenderMilliseconds = Double(word(offset + 16) &- word(offset + 12))
            }
        }
    }

    func invalidate() {
        lock.lock()
        active = false
        observations.removeAll()
        pending.removeAll()
        lock.unlock()
    }

    func snapshot() -> StreamingFrameFeedbackSnapshot {
        lock.lock()
        defer { lock.unlock() }
        return StreamingFrameFeedbackSnapshot(
            isActive: active, preventResolutionDrops: preventResolutionDrops, decodeObservationCount: observations.count,
            pendingFrameCount: pending.count,
            discardedDecodeCount: discardedDecodeCount,
            discardedFeedbackCount: discardedFeedbackCount,
            sentFrameCount: sentFrameCount,
            lastDecodeDurationMilliseconds: lastDecodeDurationMilliseconds,
            lastDecodedToRenderMilliseconds: lastDecodedToRenderMilliseconds,
            lastWireDecodeDurationMilliseconds: lastWireDecodeDurationMilliseconds,
            lastWireDecodedToRenderMilliseconds: lastWireDecodedToRenderMilliseconds)
    }

    private func expire(atMilliseconds: Double) {
        let beforeDecode = observations.count
        observations = observations.filter {
            atMilliseconds - $0.value.submittedAtMilliseconds <= Self.maximumFrameAgeMilliseconds
        }
        discardedDecodeCount += beforeDecode - observations.count
        let beforePending = pending.count
        pending.removeAll {
            atMilliseconds - $0.timing.submittedAtMilliseconds > Self.maximumFrameAgeMilliseconds
        }
        discardedFeedbackCount += beforePending - pending.count
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

    func release() -> Int { base.release() }

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
