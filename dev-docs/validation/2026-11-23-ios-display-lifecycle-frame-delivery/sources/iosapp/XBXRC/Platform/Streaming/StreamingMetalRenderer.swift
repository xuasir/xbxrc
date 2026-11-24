import CoreVideo
import Foundation
import Metal
import MetalKit
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
    let mainActorEnteredAtMs: Double

    func tracePayload(acceptedAtMs: Double) -> [String: IOSRuntimeTraceValue] {
        [
            "ingressQueueMs": .double(max(0, mainActorEnteredAtMs - receivedAtMs)),
            "frameAcceptWorkMs": .double(max(0, acceptedAtMs - mainActorEnteredAtMs)),
        ]
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
        return StreamingDecodedFrameRef(
            identifier: identifier,
            storage: storage,
            width: width,
            height: height,
            rotationDegrees: rotationDegrees,
            isFullRange: isFullRange,
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
        timestampMilliseconds: Double,
        drawableBusy: Bool? = nil
    ) -> (snapshot: StreamingDisplayHealthSnapshot, isFirstPresent: Bool) {
        lock.lock()
        if let drawableBusy { currentDrawableBusy = drawableBusy }
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

private struct StreamingVertexUniforms {
    var scale: SIMD2<Float>
    var padding: SIMD2<Float> = .zero
    var texCoordTransform: simd_float3x3
}

private struct StreamingFragmentUniforms {
    var colorMatrix: simd_float3x3
    var colorOffset: SIMD3<Float>
    var padding: Float = 0
    var effects = SIMD4<Float>(1, 1, 1, 0)
}

private let streamingMetalShaderSource = """
    #include <metal_stdlib>
    using namespace metal;

    struct VertexUniforms {
        float2 scale;
        float2 padding;
        float3x3 texCoordTransform;
    };

    struct FragmentUniforms {
        float3x3 colorMatrix;
        float3 colorOffset;
        float padding;
        float4 effects;
    };

    struct VertexOut {
        float4 position [[position]];
        float2 texCoord;
    };

    vertex VertexOut streamingVertex(
        uint vertexID [[vertex_id]],
        constant VertexUniforms& uniforms [[buffer(0)]]
    ) {
        constexpr float2 positions[4] = {
            float2(-1.0, -1.0),
            float2(1.0, -1.0),
            float2(-1.0, 1.0),
            float2(1.0, 1.0)
        };
        constexpr float2 texCoords[4] = {
            float2(0.0, 1.0),
            float2(1.0, 1.0),
            float2(0.0, 0.0),
            float2(1.0, 0.0)
        };
        VertexOut out;
        out.position = float4(positions[vertexID] * uniforms.scale, 0.0, 1.0);
        float3 transformed = uniforms.texCoordTransform * float3(texCoords[vertexID], 1.0);
        out.texCoord = transformed.xy;
        return out;
    }

    float3 adjustedColor(float3 rgb, constant FragmentUniforms& uniforms) {
        float luma = dot(rgb, float3(0.2126, 0.7152, 0.0722));
        rgb = mix(float3(luma), rgb, uniforms.effects.x);
        rgb = (rgb - 0.5) * uniforms.effects.y + 0.5;
        return saturate(rgb * uniforms.effects.z);
    }

    float sharpenedLuma(texture2d<float> texture, float2 uv, float strength) {
        constexpr sampler s(coord::normalized, address::clamp_to_edge, filter::linear);
        float center = texture.sample(s, uv).r;
        if (strength <= 0.0) return center;
        float2 step = 1.0 / float2(texture.get_width(), texture.get_height());
        float4 neighbors = float4(texture.sample(s, uv + float2(step.x, 0)).r,
            texture.sample(s, uv - float2(step.x, 0)).r,
            texture.sample(s, uv + float2(0, step.y)).r,
            texture.sample(s, uv - float2(0, step.y)).r);
        float lo = min(center, min(min(neighbors.x, neighbors.y), min(neighbors.z, neighbors.w)));
        float hi = max(center, max(max(neighbors.x, neighbors.y), max(neighbors.z, neighbors.w)));
        return clamp(center + (center - dot(neighbors, float4(0.25))) * strength, lo, hi);
    }

    fragment float4 streamingNV12Fragment(
        VertexOut in [[stage_in]],
        texture2d<float> lumaTexture [[texture(0)]],
        texture2d<float> chromaTexture [[texture(1)]],
        constant FragmentUniforms& uniforms [[buffer(0)]]
    ) {
        constexpr sampler textureSampler(
            coord::normalized,
            address::clamp_to_edge,
            filter::linear
        );
        float y = sharpenedLuma(lumaTexture, in.texCoord, uniforms.effects.w);
        float2 uv = chromaTexture.sample(textureSampler, in.texCoord).rg;
        float3 yuv = float3(y, uv.x, uv.y) + uniforms.colorOffset;
        float3 rgb = uniforms.colorMatrix * yuv;
        return float4(adjustedColor(rgb, uniforms), 1.0);
    }

    fragment float4 streamingI420Fragment(
        VertexOut in [[stage_in]],
        texture2d<float> lumaTexture [[texture(0)]],
        texture2d<float> uTexture [[texture(1)]],
        texture2d<float> vTexture [[texture(2)]],
        constant FragmentUniforms& uniforms [[buffer(0)]]
    ) {
        constexpr sampler textureSampler(
            coord::normalized,
            address::clamp_to_edge,
            filter::linear
        );
        float y = sharpenedLuma(lumaTexture, in.texCoord, uniforms.effects.w);
        float u = uTexture.sample(textureSampler, in.texCoord).r;
        float v = vTexture.sample(textureSampler, in.texCoord).r;
        float3 yuv = float3(y, u, v) + uniforms.colorOffset;
        float3 rgb = uniforms.colorMatrix * yuv;
        return float4(adjustedColor(rgb, uniforms), 1.0);
    }

    fragment float4 streamingBGRAFragment(
        VertexOut in [[stage_in]],
        texture2d<float> videoTexture [[texture(0)]],
        constant FragmentUniforms& uniforms [[buffer(0)]]
    ) {
        constexpr sampler textureSampler(
            coord::normalized,
            address::clamp_to_edge,
            filter::linear
        );
        float3 rgb = videoTexture.sample(textureSampler, in.texCoord).rgb;
        if (uniforms.effects.w > 0.0) {
            float2 step = 1.0 / float2(videoTexture.get_width(), videoTexture.get_height());
            float3 a = videoTexture.sample(textureSampler, in.texCoord + float2(step.x, 0)).rgb;
            float3 b = videoTexture.sample(textureSampler, in.texCoord - float2(step.x, 0)).rgb;
            float3 c = videoTexture.sample(textureSampler, in.texCoord + float2(0, step.y)).rgb;
            float3 d = videoTexture.sample(textureSampler, in.texCoord - float2(0, step.y)).rgb;
            float3 lo = min(rgb, min(min(a, b), min(c, d)));
            float3 hi = max(rgb, max(max(a, b), max(c, d)));
            rgb = clamp(rgb + (rgb - (a + b + c + d) * 0.25) * uniforms.effects.w, lo, hi);
        }
        return float4(adjustedColor(rgb, uniforms), 1.0);
    }
    """

struct StreamingLatestIngressDelivery<Value>: @unchecked Sendable {
    let value: Value
    let replacedCount: Int64
}

final class StreamingLatestIngressSlot<Value>: @unchecked Sendable {
    private let lock = NSLock()
    private var pendingValue: Value?
    private var deliveryScheduled = false
    private var replacedCount: Int64 = 0

    func submit(_ value: Value) -> Bool {
        lock.lock()
        if pendingValue != nil { replacedCount += 1 }
        pendingValue = value
        let shouldSchedule = !deliveryScheduled
        deliveryScheduled = true
        lock.unlock()
        return shouldSchedule
    }

    func take() -> StreamingLatestIngressDelivery<Value>? {
        lock.lock()
        defer { lock.unlock() }
        guard let pendingValue else { return nil }
        let delivery = StreamingLatestIngressDelivery(
            value: pendingValue,
            replacedCount: replacedCount
        )
        self.pendingValue = nil
        replacedCount = 0
        return delivery
    }

    func finishDelivery() -> Bool {
        lock.lock()
        defer { lock.unlock() }
        guard pendingValue == nil else { return true }
        deliveryScheduled = false
        return false
    }

    func drain(consume: (StreamingLatestIngressDelivery<Value>) -> Void) -> Bool {
        // 同次处理接续最多两帧，减少补送排队并保留 UI 的执行机会。
        for _ in 0..<2 {
            guard let delivery = take() else { break }
            consume(delivery)
        }
        return finishDelivery()
    }

    func reset() {
        lock.lock()
        pendingValue = nil
        deliveryScheduled = false
        replacedCount = 0
        lock.unlock()
    }
}

private struct StreamingQueuedVideoFrame: @unchecked Sendable {
    let frame: RTCVideoFrame
    let receivedAtMs: Double
}

private final class StreamingVideoFrameIngress: NSObject, RTCVideoRenderer, @unchecked Sendable {
    private let frameSlot = StreamingLatestIngressSlot<StreamingQueuedVideoFrame>()
    @MainActor var frameSink: (@MainActor (RTCVideoFrame, Int64, Double) -> Void)?
    @MainActor var sizeSink: (@MainActor (CGSize) -> Void)?

    func setSize(_ size: CGSize) {
        guard size.width > 0, size.height > 0 else { return }
        Task { @MainActor [weak self] in
            self?.sizeSink?(size)
        }
    }

    func renderFrame(_ frame: RTCVideoFrame?) {
        guard let frame else { return }
        let queued = StreamingQueuedVideoFrame(
            frame: frame, receivedAtMs: StreamingFrameFeedbackClock.nowMilliseconds)
        if frameSlot.submit(queued) { scheduleDrain() }
    }

    func reset() {
        frameSlot.reset()
    }

    private func scheduleDrain() {
        Task { @MainActor [weak self] in
            self?.drainLatestFrame()
        }
    }

    @MainActor
    private func drainLatestFrame() {
        let hasPending = frameSlot.drain { delivery in
            frameSink?(
                delivery.value.frame, delivery.replacedCount, delivery.value.receivedAtMs)
        }
        if hasPending { scheduleDrain() }
    }
}

final class StreamingMetalVideoSurfaceView: MTKView, MTKViewDelegate, @unchecked Sendable {
    @MainActor var presentationEventSink:
        (@MainActor (StreamingPresentationLifecycleEvent) -> Void)?
    {
        didSet {
            if presentationEventSink == nil { didDeliverDisplayPipelineReady = false }
            deliverDisplayPipelineReadyIfNeeded(reason: "sinkAttached")
        }
    }
    private let frameIngress = StreamingVideoFrameIngress()

    var renderer: RTCVideoRenderer { frameIngress }

    #if targetEnvironment(simulator)
        let rendererName = "metalSimulatorI420Compatibility"
    #else
        let rendererName = "metalZeroCopy"
    #endif

    private let mailbox = StreamingLatestFrameMailbox()
    private let telemetry = StreamingDisplayTelemetry()
    private let scheduler = StreamingMetalRenderScheduler()
    private var healthCoalescer = StreamingDisplayHealthCoalescer()
    private var healthReportTask: Task<Void, Never>?
    private let stateLock = NSLock()
    private let commandQueue: MTLCommandQueue?
    private let nv12PipelineState: MTLRenderPipelineState?
    private let i420PipelineState: MTLRenderPipelineState?
    private let bgraPipelineState: MTLRenderPipelineState?
    private var textureCache: CVMetalTextureCache?
    private var traceContext: StreamingPresentationTraceContext?
    private var frameFeedbackSession: StreamingFrameFeedbackSession?
    private var lastReportedSurfaceSize: CGSize = .zero
    private var lastReportedFrameSize: CGSize = .zero
    private var didReportDisplayPipelineReady = false
    private var didDeliverDisplayPipelineReady = false
    private var didReportFirstFrameAccepted = false
    private var didReportFirstFrameSubmitted = false
    private var didReportFirstFramePresented = false
    private var videoContentMode: StreamingVideoContentMode = .fit
    private var picture = MobilePicturePreferences()
    private var presentationActive = true
    private var presentationRevision: UInt64 = 0
    #if !targetEnvironment(simulator)
        private var superResolutionResources:
            (
                size: SIMD4<Int>, scaler: any MTLFXSpatialScaler,
                input: any MTLTexture, output: any MTLTexture
            )?
    #endif

    convenience init() {
        self.init(frame: .zero, device: MTLCreateSystemDefaultDevice())
    }

    override init(frame frameRect: CGRect, device: MTLDevice?) {
        let compiledPipelines = Self.makePipelineStates(device: device)
        commandQueue = device?.makeCommandQueue()
        nv12PipelineState = compiledPipelines?.nv12
        i420PipelineState = compiledPipelines?.i420
        bgraPipelineState = compiledPipelines?.bgra
        super.init(frame: frameRect, device: device)
        scheduler.attach(self)
        frameIngress.frameSink = { [weak self] frame, replacedCount, receivedAtMs in
            self?.consumeFrame(
                frame, ingressReplacedCount: replacedCount, receivedAtMs: receivedAtMs)
        }
        frameIngress.sizeSink = { [weak self] size in
            self?.consumeFrameSize(size)
        }

        framebufferOnly = false
        isPaused = true
        enableSetNeedsDisplay = false
        autoResizeDrawable = true
        colorPixelFormat = .bgra8Unorm
        preferredFramesPerSecond = 60
        delegate = self
        backgroundColor = .black
        isOpaque = true

        if let device {
            var cache: CVMetalTextureCache?
            CVMetalTextureCacheCreate(kCFAllocatorDefault, nil, device, nil, &cache)
            textureCache = cache
        }
    }

    @available(*, unavailable)
    required init(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    override func didMoveToWindow() {
        super.didMoveToWindow()
        scheduler.setActive(presentationActive && window != nil)
        if presentationActive && window != nil { scheduler.requestTick() }
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        reportSurfaceSizeIfNeeded()
    }

    func updateTraceContext(_ context: StreamingPresentationTraceContext?) {
        if currentTraceContext() != context { flushDisplayHealth(reset: true) }
        stateLock.lock()
        guard streamingPresentationContextDidChange(previous: traceContext, next: context) else {
            stateLock.unlock()
            return
        }
        traceContext = context
        presentationRevision &+= 1
        lastReportedSurfaceSize = .zero
        lastReportedFrameSize = .zero
        didReportDisplayPipelineReady = false
        didDeliverDisplayPipelineReady = false
        didReportFirstFrameAccepted = false
        didReportFirstFrameSubmitted = false
        didReportFirstFramePresented = false
        let healthSnapshot = telemetry.reset()
        mailbox.reset()
        frameIngress.reset()
        stateLock.unlock()
        if let context {
            emitPresentationEvent(.displayHealth(context, healthSnapshot))
        }
        reportSurfaceSizeIfNeeded()
        #if !targetEnvironment(simulator)
            reportDisplayPipelineReadyIfNeeded(frameSize: nil)
        #endif
    }

    func updateFrameFeedbackSession(_ session: StreamingFrameFeedbackSession?) {
        stateLock.lock()
        frameFeedbackSession = session
        stateLock.unlock()
    }

    func setPresentationActive(_ active: Bool) {
        guard presentationActive != active else { return }
        flushDisplayHealth(reset: true)
        presentationActive = active
        presentationRevision &+= 1
        didDeliverDisplayPipelineReady = false
        mailbox.reset()
        telemetry.cancelPendingSubmissions()
        frameIngress.reset()
        scheduler.setActive(active && window != nil)
        if active {
            deliverDisplayPipelineReadyIfNeeded(reason: "foreground")
            scheduler.requestTick()
        }
    }

    func shutdown() {
        flushDisplayHealth(reset: true)
        presentationActive = false
        presentationRevision &+= 1
        updateFrameFeedbackSession(nil)
        mailbox.reset()
        telemetry.cancelPendingSubmissions()
        frameIngress.reset()
        frameIngress.frameSink = nil
        frameIngress.sizeSink = nil
        scheduler.invalidate()
        #if !targetEnvironment(simulator)
            superResolutionResources = nil
        #endif
    }

    func updateVideoContentMode(_ mode: StreamingVideoContentMode) {
        stateLock.lock()
        videoContentMode = mode
        stateLock.unlock()
        scheduler.requestTick()
    }

    func updatePicture(_ value: MobilePicturePreferences) {
        stateLock.lock()
        picture = value.normalized()
        stateLock.unlock()
        scheduler.requestTick()
    }

    private func consumeFrameSize(_ size: CGSize) {
        guard size.width > 0, size.height > 0 else { return }
        reportDisplayPipelineReadyIfNeeded(frameSize: size)
    }

    private func consumeFrame(
        _ frame: RTCVideoFrame, ingressReplacedCount: Int64, receivedAtMs: Double
    ) {
        let ingressTiming = StreamingFrameIngressTiming(
            receivedAtMs: receivedAtMs,
            mainActorEnteredAtMs: StreamingFrameFeedbackClock.nowMilliseconds)
        // 后台首帧只补齐管线能力事实；后续后台帧在入口释放。
        guard presentationActive || !didReportDisplayPipelineReady else { return }
        let ingressContext = currentTraceContext()
        if presentationActive, ingressReplacedCount > 0 {
            recordDrop(
                reason: .ingressCoalesced,
                count: ingressReplacedCount,
                context: ingressContext
            )
        }
        let decodedStorage: StreamingDecodedFrameStorage
        let isFullRange: Bool
        let compatibilityPath: String?
        if let cvBuffer = frame.buffer as? RTCCVPixelBuffer {
            let pixelBuffer = cvBuffer.pixelBuffer
            let pixelFormat = CVPixelBufferGetPixelFormatType(pixelBuffer)
            let supportedFormat =
                pixelFormat == kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange
                || pixelFormat == kCVPixelFormatType_420YpCbCr8BiPlanarFullRange
                || pixelFormat == kCVPixelFormatType_32BGRA
            guard supportedFormat else {
                recordDrop(reason: .unsupportedPixelFormat, context: currentTraceContext())
                return
            }
            decodedStorage = .pixelBuffer(pixelBuffer)
            isFullRange = pixelFormat == kCVPixelFormatType_420YpCbCr8BiPlanarFullRange
            compatibilityPath = nil
        } else {
            #if targetEnvironment(simulator)
                // Simulator 常回退到软件 I420 解码，直接按 stride 上传三平面以控制每帧 CPU 开销。
                let i420Buffer = frame.buffer.toI420()
                decodedStorage = .simulatorI420(StreamingSimulatorI420Frame(buffer: i420Buffer))
                isFullRange = false
                compatibilityPath = "simulatorI420Upload"
            #else
                recordDrop(reason: .unsupportedBufferType, context: currentTraceContext())
                return
            #endif
        }

        let context = currentTraceContext()
        reportDisplayPipelineReadyIfNeeded(
            frameSize: CGSize(
                width: CGFloat(frame.width),
                height: CGFloat(frame.height)
            ),
            storage: decodedStorage
        )
        guard presentationActive else { return }

        stateLock.lock()
        let feedbackSession = frameFeedbackSession
        stateLock.unlock()
        let feedbackTicket: StreamingFrameFeedbackTicket?
        if let feedbackSession,
            let timing = feedbackSession.takeDecodedTiming(
                serverDataKey: UInt32(bitPattern: frame.timeStamp),
                nowMilliseconds: StreamingFrameFeedbackClock.nowMilliseconds)
        {
            feedbackTicket = StreamingFrameFeedbackTicket(session: feedbackSession, timing: timing)
        } else {
            feedbackTicket = nil
        }
        let decodedFrame = mailbox.makeFrame(
            storage: decodedStorage,
            width: Int(frame.width),
            height: Int(frame.height),
            rotationDegrees: Int(frame.rotation.rawValue),
            isFullRange: isFullRange,
            traceContext: context,
            frameFeedback: feedbackTicket
        )
        let submission = mailbox.submit(decodedFrame)
        if let context {
            let firstAccepted = !didReportFirstFrameAcceptedForCurrentContext()
            var acceptedPayload: [String: IOSRuntimeTraceValue] = [
                "firstFrame": .bool(firstAccepted),
                "frameId": .integer(Int64(clamping: decodedFrame.identifier)),
                "frameWidth": .integer(Int64(frame.width)),
                "frameHeight": .integer(Int64(frame.height)),
                "rotationDegrees": .integer(Int64(decodedFrame.rotationDegrees)),
            ]
            if let compatibilityPath {
                acceptedPayload["compatibilityPath"] = .string(compatibilityPath)
            }
            acceptedPayload.merge(
                ingressTiming.tracePayload(
                    acceptedAtMs: StreamingFrameFeedbackClock.nowMilliseconds)
            ) { _, new in new }
            IOSRuntimeTrace.state(
                domain: "ios-streaming",
                event: "videoFrameAcceptedToMailbox",
                payload: streamingPresentationTracePayload(
                    acceptedPayload,
                    context: context
                ),
                dimension: .presentation,
                importance: .key,
                operationID: context.attemptID
            )
            if firstAccepted {
                emitPresentationEvent(.firstFrameAccepted(context))
            }
            if submission.overwroteCandidate {
                IOSRuntimeTrace.state(
                    domain: "ios-streaming",
                    event: "mailboxOverwrite",
                    payload: streamingPresentationTracePayload(
                        [
                            "mailboxOverwriteCount": .integer(mailbox.snapshotOverwriteCount())
                        ],
                        context: context
                    ),
                    dimension: .presentation,
                    importance: .key,
                    operationID: context.attemptID
                )
                emitPresentationEvent(
                    .displayHealth(
                        context,
                        telemetry.recordMailboxOverwrite(
                            totalOverwriteCount: mailbox.snapshotOverwriteCount()
                        )
                    )
                )
            }
        }
        scheduler.requestTick()
    }

    func mtkView(_: MTKView, drawableSizeWillChange _: CGSize) {
        reportSurfaceSizeIfNeeded()
    }

    func draw(in _: MTKView) {
        guard presentationActive else { return }
        let drawStartedAtMs = StreamingFrameFeedbackClock.nowMilliseconds
        let drain = mailbox.beginRender()
        if drain.drawableBusy { return }
        guard let frame = drain.frame else { return }
        guard let traceContext = frame.traceContext else {
            _ = mailbox.finishRender(identifier: frame.identifier, requeueAsLatest: false)
            return
        }
        let drawableAcquireStartedAtMs = StreamingFrameFeedbackClock.nowMilliseconds
        guard let commandQueue,
            let descriptor = currentRenderPassDescriptor,
            let drawable = currentDrawable
        else {
            handleSkippedPresent(
                frame: frame,
                reason: .drawableUnavailable,
                requeueAsLatest: true
            )
            return
        }
        let drawableAcquiredAtMs = StreamingFrameFeedbackClock.nowMilliseconds

        guard let renderResources = makeRenderResources(for: frame) else {
            handleSkippedPresent(
                frame: frame,
                reason: .textureImportFailed,
                requeueAsLatest: false
            )
            return
        }

        guard let commandBuffer = commandQueue.makeCommandBuffer() else {
            handleSkippedPresent(frame: frame, reason: .encoderUnavailable, requeueAsLatest: false)
            return
        }
        let scaledTexture = encodeSuperResolution(
            frame: frame,
            pipelineState: renderResources.pipelineState, primary: renderResources.primaryTexture,
            secondary: renderResources.secondaryTexture, tertiary: renderResources.tertiaryTexture,
            uniforms: renderResources.fragmentUniforms, commandBuffer: commandBuffer)
        guard let encoder = commandBuffer.makeRenderCommandEncoder(descriptor: descriptor) else {
            handleSkippedPresent(
                frame: frame,
                reason: .encoderUnavailable,
                requeueAsLatest: false
            )
            return
        }

        emitPresentationEvent(.displayHealth(traceContext, telemetry.setDrawableBusy(true)))
        let submitTimestamp = StreamingFrameFeedbackClock.nowMilliseconds
        let submitSnapshot = telemetry.recordSubmit(
            identifier: frame.identifier,
            timestampMilliseconds: submitTimestamp,
            configuration: StreamingDisplayConfiguration(
                sourceWidth: frame.width, sourceHeight: frame.height,
                outputWidth: Int(
                    Float(drawable.texture.width) * abs(renderResources.vertexUniforms.scale.x)),
                outputHeight: Int(
                    Float(drawable.texture.height) * abs(renderResources.vertexUniforms.scale.y)),
                superResolutionActive: scaledTexture != nil,
                sharpness: renderResources.fragmentUniforms.effects.w <= 0
                    ? 0
                    : renderResources.fragmentUniforms.effects.w <= 0.25 ? 1 : 3)
        )
        emitPresentationEvent(.displayHealth(traceContext, submitSnapshot))
        var submitPayload: [String: IOSRuntimeTraceValue] = [
            "submitCount": .integer(submitSnapshot.submitCount),
            "frameId": .integer(Int64(clamping: frame.identifier)),
            "submittedAtMonotonicMs": .double(submitTimestamp),
            "frameWidth": .integer(Int64(frame.width)),
            "frameHeight": .integer(Int64(frame.height)),
            "drawToSubmitMs": .double(max(0, submitTimestamp - drawStartedAtMs)),
            "drawableAcquireMs": .double(
                max(0, drawableAcquiredAtMs - drawableAcquireStartedAtMs)),
        ]
        if let compatibilityPath = frame.storage.compatibilityPath {
            submitPayload["compatibilityPath"] = .string(compatibilityPath)
        }
        IOSRuntimeTrace.state(
            domain: "ios-streaming",
            event: "displaySubmit",
            payload: streamingPresentationTracePayload(
                submitPayload,
                context: traceContext
            ),
            dimension: .presentation,
            importance: .key,
            operationID: traceContext.attemptID
        )
        if !didReportFirstFrameSubmittedForCurrentContext() {
            emitPresentationEvent(.firstFrameSubmitted(traceContext))
        }

        var vertexUniforms = renderResources.vertexUniforms
        var fragmentUniforms =
            scaledTexture == nil
            ? renderResources.fragmentUniforms
            : StreamingFragmentUniforms(
                colorMatrix: matrix_identity_float3x3, colorOffset: .zero)
        encoder.setRenderPipelineState(
            scaledTexture == nil ? renderResources.pipelineState : bgraPipelineState!)
        encoder.setVertexBytes(
            &vertexUniforms, length: MemoryLayout<StreamingVertexUniforms>.stride, index: 0)
        encoder.setFragmentBytes(
            &fragmentUniforms, length: MemoryLayout<StreamingFragmentUniforms>.stride, index: 0)
        encoder.setFragmentTexture(scaledTexture ?? renderResources.primaryTexture, index: 0)
        if scaledTexture == nil {
            encoder.setFragmentTexture(renderResources.secondaryTexture, index: 1)
            encoder.setFragmentTexture(renderResources.tertiaryTexture, index: 2)
        }
        encoder.drawPrimitives(type: .triangleStrip, vertexStart: 0, vertexCount: 4)
        encoder.endEncoding()

        let revision = presentationRevision
        commandBuffer.addCompletedHandler { [weak self, mailbox] completedCommandBuffer in
            let completedSuccessfully = completedCommandBuffer.status == .completed
            let renderedAtMilliseconds = StreamingFrameFeedbackClock.nowMilliseconds
            // GPU 已完成后立即释放帧槽；唯一 frameId 隔离 reset 后的迟到回调。
            if completedSuccessfully {
                _ = mailbox.finishRender(identifier: frame.identifier, requeueAsLatest: false)
            }
            let slotReleasedAtMs = StreamingFrameFeedbackClock.nowMilliseconds
            Task { @MainActor in
                self?.handleCompletedCommandBuffer(
                    completedSuccessfully: completedSuccessfully,
                    frame: frame,
                    context: traceContext,
                    revision: revision,
                    renderedAtMilliseconds: renderedAtMilliseconds,
                    slotReleasedAtMs: slotReleasedAtMs
                )
            }
        }
        #if !targetEnvironment(simulator)
            // 真机分别观测系统 drawable 呈现与 GPU 完成，模拟器沿用 GPU 完成观测。
            drawable.addPresentedHandler { [weak self] presentedDrawable in
                let presentedAt = presentedDrawable.presentedTime * 1_000
                guard presentedAt > 0 else { return }
                Task { @MainActor in
                    guard let self, self.presentationActive,
                        self.presentationRevision == revision,
                        self.currentTraceContext() == traceContext
                    else { return }
                    let snapshot = self.telemetry.recordDrawablePresent(
                        submittedAt: submitTimestamp, presentedAt: presentedAt)
                    self.emitPresentationEvent(.displayHealth(traceContext, snapshot))
                }
            }
        #endif
        commandBuffer.present(drawable)
        commandBuffer.commit()
    }

    private func handleCompletedCommandBuffer(
        completedSuccessfully: Bool,
        frame: StreamingDecodedFrameRef,
        context: StreamingPresentationTraceContext,
        revision: UInt64,
        renderedAtMilliseconds: Double,
        slotReleasedAtMs: Double
    ) {
        let mainActorEnteredAtMs = StreamingFrameFeedbackClock.nowMilliseconds
        guard presentationActive, presentationRevision == revision,
            currentTraceContext() == context
        else { return }
        guard completedSuccessfully else {
            telemetry.cancelSubmit(identifier: frame.identifier)
            handleSkippedPresent(
                frame: frame,
                reason: .drawableUnavailable,
                requeueAsLatest: true
            )
            return
        }
        if let feedback = frame.frameFeedback {
            feedback.session.recordRendered(
                timing: feedback.timing, atMilliseconds: renderedAtMilliseconds)
        }
        let feedbackFinishedAtMs = StreamingFrameFeedbackClock.nowMilliseconds
        let present = telemetry.recordPresent(
            identifier: frame.identifier,
            timestampMilliseconds: renderedAtMilliseconds,
            drawableBusy: mailbox.snapshotDrawableBusy()
        )
        emitPresentationEvent(.displayHealth(context, present.snapshot))
        let timing = StreamingGpuCompletionTiming(
            completedAtMs: renderedAtMilliseconds, slotReleasedAtMs: slotReleasedAtMs,
            mainActorEnteredAtMs: mainActorEnteredAtMs, feedbackFinishedAtMs: feedbackFinishedAtMs,
            telemetryFinishedAtMs: StreamingFrameFeedbackClock.nowMilliseconds)
        var presentPayload: [String: IOSRuntimeTraceValue] = [
            "presentCount": .integer(present.snapshot.presentCount),
            "frameId": .integer(Int64(clamping: frame.identifier)),
            "presentedAtMonotonicMs": .double(renderedAtMilliseconds),
            "presentTimestampSource": .string("commandBufferCompletion"),
            "frameWidth": .integer(Int64(frame.width)),
            "frameHeight": .integer(Int64(frame.height)),
        ]
        if let compatibilityPath = frame.storage.compatibilityPath {
            presentPayload["compatibilityPath"] = .string(compatibilityPath)
        }
        presentPayload.merge(
            timing.tracePayload(recordedAtMs: StreamingFrameFeedbackClock.nowMilliseconds)
        ) { _, new in new }
        IOSRuntimeTrace.state(
            domain: "ios-streaming",
            event: "displayPresent",
            payload: streamingPresentationTracePayload(
                presentPayload,
                context: context
            ),
            dimension: .presentation,
            importance: .key,
            operationID: context.attemptID
        )
        if present.isFirstPresent, !didReportFirstFramePresentedForCurrentContext() {
            var firstPresentPayload: [String: IOSRuntimeTraceValue] = [
                "presentCount": .integer(present.snapshot.presentCount)
            ]
            if let compatibilityPath = frame.storage.compatibilityPath {
                firstPresentPayload["compatibilityPath"] = .string(compatibilityPath)
            }
            IOSRuntimeTrace.state(
                domain: "ios-streaming",
                event: "displayFirstPresent",
                payload: streamingPresentationTracePayload(
                    firstPresentPayload,
                    context: context
                ),
                dimension: .presentation,
                importance: .essential,
                operationID: context.attemptID
            )
            emitPresentationEvent(.firstFramePresented(context))
        }
        if mailbox.hasPendingFrame() { scheduler.requestTick() }
    }

    private func handleSkippedPresent(
        frame: StreamingDecodedFrameRef,
        reason: StreamingVideoFrameDropReason,
        requeueAsLatest: Bool
    ) {
        let context = frame.traceContext
        let hasPending = mailbox.finishRender(
            identifier: frame.identifier, requeueAsLatest: requeueAsLatest)
        recordDrop(reason: reason, context: context)
        if let context {
            var skippedPayload: [String: IOSRuntimeTraceValue] = [
                "reason": .string(reason.rawValue)
            ]
            if let compatibilityPath = frame.storage.compatibilityPath {
                skippedPayload["compatibilityPath"] = .string(compatibilityPath)
            }
            IOSRuntimeTrace.state(
                domain: "ios-streaming",
                event: "displayPresentSkipped",
                payload: streamingPresentationTracePayload(
                    skippedPayload,
                    context: context
                ),
                dimension: .presentation,
                importance: .key,
                operationID: context.attemptID
            )
            emitPresentationEvent(
                .displayHealth(context, telemetry.setDrawableBusy(mailbox.snapshotDrawableBusy())))
        }
        if StreamingRenderRetryPolicy.shouldRequestImmediateTick(
            requeueAsLatest: requeueAsLatest,
            hasPendingFrame: hasPending
        ) {
            scheduler.requestTick()
        } else {
            scheduler.scheduleDisplayTickIfNeeded()
        }
    }

    private func makeRenderResources(
        for frame: StreamingDecodedFrameRef
    ) -> (
        pipelineState: MTLRenderPipelineState,
        primaryTexture: MTLTexture,
        secondaryTexture: MTLTexture?,
        tertiaryTexture: MTLTexture?,
        vertexUniforms: StreamingVertexUniforms,
        fragmentUniforms: StreamingFragmentUniforms
    )? {
        let drawableWidth = max(drawableSize.width, 1)
        let drawableHeight = max(drawableSize.height, 1)
        let renderScale = normalizedRenderScale(
            for: frame,
            drawableWidth: Double(drawableWidth),
            drawableHeight: Double(drawableHeight)
        )

        let vertexUniforms = StreamingVertexUniforms(
            scale: renderScale,
            texCoordTransform: textureCoordinateTransform(rotationDegrees: frame.rotationDegrees)
        )
        var fragmentUniforms = makeFragmentUniforms(fullRange: frame.isFullRange)
        stateLock.lock()
        let selectedPicture = picture
        stateLock.unlock()
        if !selectedPicture.compatibilityMode {
            fragmentUniforms.effects = SIMD4<Float>(
                Float(selectedPicture.saturation / 100),
                Float(selectedPicture.contrast / 100), Float(selectedPicture.brightness / 100),
                selectedPicture.sharpness == 0 ? 0 : selectedPicture.sharpness == 1 ? 0.25 : 0.65)
        }
        switch frame.storage {
        case .pixelBuffer(let pixelBuffer):
            let pixelFormat = CVPixelBufferGetPixelFormatType(pixelBuffer)
            if pixelFormat == kCVPixelFormatType_32BGRA {
                guard let bgraPipelineState,
                    let texture = makeTexture(
                        pixelBuffer: pixelBuffer,
                        pixelFormat: .bgra8Unorm,
                        planeIndex: 0
                    )
                else { return nil }
                return (
                    pipelineState: bgraPipelineState,
                    primaryTexture: texture,
                    secondaryTexture: nil,
                    tertiaryTexture: nil,
                    vertexUniforms: vertexUniforms,
                    fragmentUniforms: fragmentUniforms
                )
            }

            guard let nv12PipelineState,
                let lumaTexture = makeTexture(
                    pixelBuffer: pixelBuffer,
                    pixelFormat: .r8Unorm,
                    planeIndex: 0
                ),
                let chromaTexture = makeTexture(
                    pixelBuffer: pixelBuffer,
                    pixelFormat: .rg8Unorm,
                    planeIndex: 1
                )
            else { return nil }
            return (
                pipelineState: nv12PipelineState,
                primaryTexture: lumaTexture,
                secondaryTexture: chromaTexture,
                tertiaryTexture: nil,
                vertexUniforms: vertexUniforms,
                fragmentUniforms: fragmentUniforms
            )

        #if targetEnvironment(simulator)
            case .simulatorI420(let i420Frame):
                let i420Buffer = i420Frame.buffer
                guard let i420PipelineState,
                    let lumaTexture = makeSimulatorI420Texture(
                        bytes: UnsafeRawPointer(i420Buffer.dataY),
                        bytesPerRow: Int(i420Buffer.strideY),
                        width: Int(i420Buffer.width),
                        height: Int(i420Buffer.height)
                    ),
                    let uTexture = makeSimulatorI420Texture(
                        bytes: UnsafeRawPointer(i420Buffer.dataU),
                        bytesPerRow: Int(i420Buffer.strideU),
                        width: Int(i420Buffer.chromaWidth),
                        height: Int(i420Buffer.chromaHeight)
                    ),
                    let vTexture = makeSimulatorI420Texture(
                        bytes: UnsafeRawPointer(i420Buffer.dataV),
                        bytesPerRow: Int(i420Buffer.strideV),
                        width: Int(i420Buffer.chromaWidth),
                        height: Int(i420Buffer.chromaHeight)
                    )
                else { return nil }
                return (
                    pipelineState: i420PipelineState,
                    primaryTexture: lumaTexture,
                    secondaryTexture: uTexture,
                    tertiaryTexture: vTexture,
                    vertexUniforms: vertexUniforms,
                    fragmentUniforms: fragmentUniforms
                )
        #endif
        }
    }

    #if targetEnvironment(simulator)
        private func makeSimulatorI420Texture(
            bytes: UnsafeRawPointer,
            bytesPerRow: Int,
            width: Int,
            height: Int
        ) -> MTLTexture? {
            guard width > 0, height > 0, bytesPerRow >= width, let device else {
                return nil
            }
            let descriptor = MTLTextureDescriptor.texture2DDescriptor(
                pixelFormat: .r8Unorm,
                width: width,
                height: height,
                mipmapped: false
            )
            descriptor.usage = .shaderRead
            descriptor.storageMode = .shared
            guard let texture = device.makeTexture(descriptor: descriptor) else { return nil }
            texture.replace(
                region: MTLRegionMake2D(0, 0, width, height),
                mipmapLevel: 0,
                withBytes: bytes,
                bytesPerRow: bytesPerRow
            )
            return texture
        }
    #endif

    private func makeTexture(
        pixelBuffer: CVPixelBuffer,
        pixelFormat: MTLPixelFormat,
        planeIndex: Int
    ) -> MTLTexture? {
        guard let textureCache else { return nil }
        let isPlanar = CVPixelBufferIsPlanar(pixelBuffer)
        let width =
            isPlanar
            ? max(CVPixelBufferGetWidthOfPlane(pixelBuffer, planeIndex), 1)
            : max(CVPixelBufferGetWidth(pixelBuffer), 1)
        let height =
            isPlanar
            ? max(CVPixelBufferGetHeightOfPlane(pixelBuffer, planeIndex), 1)
            : max(CVPixelBufferGetHeight(pixelBuffer), 1)
        var textureRef: CVMetalTexture?
        let status = CVMetalTextureCacheCreateTextureFromImage(
            kCFAllocatorDefault,
            textureCache,
            pixelBuffer,
            nil,
            pixelFormat,
            width,
            height,
            planeIndex,
            &textureRef
        )
        guard status == kCVReturnSuccess,
            let textureRef,
            let texture = CVMetalTextureGetTexture(textureRef)
        else { return nil }
        return texture
    }

    /// 原生超分作为同一 commandBuffer 的可选效果，沿原有 mailbox 与 present 终态。
    private func encodeSuperResolution(
        frame: StreamingDecodedFrameRef, pipelineState: MTLRenderPipelineState,
        primary: MTLTexture, secondary: MTLTexture?, tertiary: MTLTexture?,
        uniforms: StreamingFragmentUniforms, commandBuffer: MTLCommandBuffer
    ) -> MTLTexture? {
        #if targetEnvironment(simulator)
            return nil
        #else
            stateLock.lock()
            let selectedPicture = picture
            stateLock.unlock()
            guard selectedPicture.superResolution, !selectedPicture.compatibilityMode,
                let device, bgraPipelineState != nil,
                MTLFXSpatialScalerDescriptor.supportsDevice(device)
            else { return nil }
            let rotated = frame.rotationDegrees == 90 || frame.rotationDegrees == 270
            let width = Double(rotated ? frame.height : frame.width)
            let height = Double(rotated ? frame.width : frame.height)
            guard width > 0, height > 0 else { return nil }
            let extent = normalizedRenderScale(
                for: frame,
                drawableWidth: max(drawableSize.width, 1),
                drawableHeight: max(drawableSize.height, 1))
            let scale = min(
                2,
                max(
                    drawableSize.width * Double(abs(extent.x)) / width,
                    drawableSize.height * Double(abs(extent.y)) / height))
            guard scale > 1.01 else { return nil }
            let outputWidth = Int(Double(frame.width) * scale)
            let outputHeight = Int(Double(frame.height) * scale)
            let size = SIMD4<Int>(frame.width, frame.height, outputWidth, outputHeight)
            if superResolutionResources?.size != size {
                let descriptor = MTLFXSpatialScalerDescriptor()
                descriptor.inputWidth = frame.width
                descriptor.inputHeight = frame.height
                descriptor.outputWidth = outputWidth
                descriptor.outputHeight = outputHeight
                descriptor.colorTextureFormat = .bgra8Unorm
                descriptor.outputTextureFormat = .bgra8Unorm
                descriptor.colorProcessingMode = .perceptual
                guard let scaler = descriptor.makeSpatialScaler(device: device) else { return nil }
                let inputDescriptor = MTLTextureDescriptor.texture2DDescriptor(
                    pixelFormat: .bgra8Unorm,
                    width: frame.width, height: frame.height, mipmapped: false)
                inputDescriptor.usage = scaler.colorTextureUsage.union([.renderTarget, .shaderRead])
                inputDescriptor.storageMode = .private
                let outputDescriptor = MTLTextureDescriptor.texture2DDescriptor(
                    pixelFormat: .bgra8Unorm,
                    width: outputWidth, height: outputHeight, mipmapped: false)
                outputDescriptor.usage = scaler.outputTextureUsage.union(.shaderRead)
                outputDescriptor.storageMode = .private
                guard let input = device.makeTexture(descriptor: inputDescriptor),
                    let output = device.makeTexture(descriptor: outputDescriptor)
                else { return nil }
                superResolutionResources = (size, scaler, input, output)
                IOSRuntimeTrace.state(
                    domain: "ios-streaming", event: "spatialScalerConfigured",
                    payload: [
                        "inputWidth": .integer(Int64(frame.width)),
                        "inputHeight": .integer(Int64(frame.height)),
                        "outputWidth": .integer(Int64(outputWidth)),
                        "outputHeight": .integer(Int64(outputHeight)),
                    ],
                    dimension: .presentation, importance: .key,
                    operationID: frame.traceContext?.attemptID)
            }
            guard let resources = superResolutionResources else { return nil }
            let pass = MTLRenderPassDescriptor()
            pass.colorAttachments[0].texture = resources.input
            pass.colorAttachments[0].loadAction = .clear
            pass.colorAttachments[0].storeAction = .store
            guard let encoder = commandBuffer.makeRenderCommandEncoder(descriptor: pass) else {
                return nil
            }
            var vertex = StreamingVertexUniforms(
                scale: SIMD2<Float>(1, 1), texCoordTransform: matrix_identity_float3x3)
            var fragment = uniforms
            encoder.setRenderPipelineState(pipelineState)
            encoder.setVertexBytes(
                &vertex, length: MemoryLayout<StreamingVertexUniforms>.stride, index: 0)
            encoder.setFragmentBytes(
                &fragment, length: MemoryLayout<StreamingFragmentUniforms>.stride, index: 0)
            encoder.setFragmentTexture(primary, index: 0)
            encoder.setFragmentTexture(secondary, index: 1)
            encoder.setFragmentTexture(tertiary, index: 2)
            encoder.drawPrimitives(type: .triangleStrip, vertexStart: 0, vertexCount: 4)
            encoder.endEncoding()
            resources.scaler.colorTexture = resources.input
            resources.scaler.outputTexture = resources.output
            resources.scaler.inputContentWidth = frame.width
            resources.scaler.inputContentHeight = frame.height
            resources.scaler.encode(commandBuffer: commandBuffer)
            return resources.output
        #endif
    }

    private func normalizedRenderScale(
        for frame: StreamingDecodedFrameRef,
        drawableWidth: Double,
        drawableHeight: Double
    ) -> SIMD2<Float> {
        let rotated = frame.rotationDegrees == 90 || frame.rotationDegrees == 270
        let frameWidth = Double(rotated ? frame.height : frame.width)
        let frameHeight = Double(rotated ? frame.width : frame.height)
        guard frameWidth > 0, frameHeight > 0 else {
            return SIMD2<Float>(1, 1)
        }
        var frameAspect = frameWidth / frameHeight
        let drawableAspect = drawableWidth / drawableHeight
        stateLock.lock()
        let contentMode = videoContentMode
        stateLock.unlock()
        switch contentMode {
        case .stretch: return SIMD2<Float>(1, 1)
        case .ratio16x9: frameAspect = 16.0 / 9.0
        case .ratio16x10: frameAspect = 16.0 / 10.0
        case .ratio18x9: frameAspect = 18.0 / 9.0
        case .ratio21x9: frameAspect = 21.0 / 9.0
        case .ratio4x3: frameAspect = 4.0 / 3.0
        case .fit, .fill: break
        }
        switch contentMode {
        case .fit, .ratio16x9, .ratio16x10, .ratio18x9, .ratio21x9, .ratio4x3, .stretch:
            if frameAspect > drawableAspect {
                return SIMD2<Float>(1, Float(drawableAspect / frameAspect))
            }
            return SIMD2<Float>(Float(frameAspect / drawableAspect), 1)
        case .fill:
            if frameAspect > drawableAspect {
                return SIMD2<Float>(Float(frameAspect / drawableAspect), 1)
            }
            return SIMD2<Float>(1, Float(drawableAspect / frameAspect))
        }
    }

    private func textureCoordinateTransform(rotationDegrees: Int) -> simd_float3x3 {
        switch rotationDegrees {
        case 90:
            return simd_float3x3(rows: [
                SIMD3<Float>(0, -1, 1),
                SIMD3<Float>(1, 0, 0),
                SIMD3<Float>(0, 0, 1),
            ])
        case 180:
            return simd_float3x3(rows: [
                SIMD3<Float>(-1, 0, 1),
                SIMD3<Float>(0, -1, 1),
                SIMD3<Float>(0, 0, 1),
            ])
        case 270:
            return simd_float3x3(rows: [
                SIMD3<Float>(0, 1, 0),
                SIMD3<Float>(-1, 0, 1),
                SIMD3<Float>(0, 0, 1),
            ])
        default:
            return matrix_identity_float3x3
        }
    }

    private func makeFragmentUniforms(fullRange: Bool) -> StreamingFragmentUniforms {
        if fullRange {
            return StreamingFragmentUniforms(
                colorMatrix: simd_float3x3(rows: [
                    SIMD3<Float>(1.0, 0.0, 1.402),
                    SIMD3<Float>(1.0, -0.344136, -0.714136),
                    SIMD3<Float>(1.0, 1.772, 0.0),
                ]),
                colorOffset: SIMD3<Float>(0, -0.5, -0.5)
            )
        }
        return StreamingFragmentUniforms(
            colorMatrix: simd_float3x3(rows: [
                SIMD3<Float>(1.164383, 0.0, 1.596027),
                SIMD3<Float>(1.164383, -0.391762, -0.812968),
                SIMD3<Float>(1.164383, 2.017232, 0.0),
            ]),
            colorOffset: SIMD3<Float>(-16.0 / 255.0, -0.5, -0.5)
        )
    }

    private func recordDrop(
        reason: StreamingVideoFrameDropReason,
        count: Int64 = 1,
        context: StreamingPresentationTraceContext?
    ) {
        guard let context else { return }
        IOSRuntimeTrace.state(
            domain: "ios-streaming",
            event: "videoFrameDropped",
            payload: streamingPresentationTracePayload(
                [
                    "count": .integer(max(count, 1)),
                    "reason": .string(reason.rawValue),
                ],
                context: context
            ),
            dimension: .presentation,
            importance: .key,
            operationID: context.attemptID
        )
        emitPresentationEvent(
            .displayHealth(
                context,
                telemetry.recordDrop(reason: reason, count: count)
            )
        )
    }

    private func reportSurfaceSizeIfNeeded() {
        guard bounds.width > 0, bounds.height > 0 else { return }
        let context = currentTraceContext()
        guard let context else { return }

        stateLock.lock()
        let shouldReport = bounds.size != lastReportedSurfaceSize
        if shouldReport { lastReportedSurfaceSize = bounds.size }
        stateLock.unlock()
        guard shouldReport else { return }

        IOSRuntimeTrace.state(
            domain: "ios-streaming",
            event: "videoSurfaceSized",
            payload: streamingPresentationTracePayload(
                [
                    "width": .double(bounds.width),
                    "height": .double(bounds.height),
                    "renderer": .string(rendererName),
                ],
                context: context
            ),
            dimension: .presentation,
            importance: .key,
            operationID: context.attemptID
        )
    }

    private func reportDisplayPipelineReadyIfNeeded(
        frameSize: CGSize?,
        storage: StreamingDecodedFrameStorage? = nil
    ) {
        let context = currentTraceContext()
        guard let context else { return }
        #if targetEnvironment(simulator)
            let hasPipeline: Bool
            switch storage {
            case .pixelBuffer:
                hasPipeline =
                    device != nil
                    && commandQueue != nil
                    && (nv12PipelineState != nil || bgraPipelineState != nil)
                    && textureCache != nil
            case .simulatorI420:
                hasPipeline =
                    device != nil
                    && commandQueue != nil
                    && i420PipelineState != nil
            case nil:
                return
            }
        #else
            let hasPipeline =
                device != nil
                && commandQueue != nil
                && (nv12PipelineState != nil || bgraPipelineState != nil)
                && textureCache != nil
        #endif
        guard hasPipeline else {
            recordDrop(reason: .pipelineUnavailable, context: context)
            return
        }

        var shouldReportPipelineReady = false
        var shouldReportRendererReady = false
        stateLock.lock()
        if !didReportDisplayPipelineReady {
            didReportDisplayPipelineReady = true
            shouldReportPipelineReady = true
        }
        if let frameSize, frameSize.width > 0, frameSize.height > 0,
            frameSize != lastReportedFrameSize
        {
            lastReportedFrameSize = frameSize
            shouldReportRendererReady = true
        }
        stateLock.unlock()

        if shouldReportPipelineReady {
            IOSRuntimeTrace.state(
                domain: "ios-streaming",
                event: "displayPipelineReady",
                payload: streamingPresentationTracePayload(
                    ["renderer": .string(rendererName)],
                    context: context
                ),
                dimension: .presentation,
                importance: .essential,
                operationID: context.attemptID
            )
        }
        deliverDisplayPipelineReadyIfNeeded(reason: "observed")

        if shouldReportRendererReady {
            IOSRuntimeTrace.state(
                domain: "ios-streaming",
                event: "videoSurfaceRendererReady",
                payload: streamingPresentationTracePayload(
                    [
                        "frameHeight": .double(Double(frameSize?.height ?? 0)),
                        "frameWidth": .double(Double(frameSize?.width ?? 0)),
                        "renderer": .string(rendererName),
                    ],
                    context: context
                ),
                dimension: .presentation,
                importance: .key,
                operationID: context.attemptID
            )
        }
    }

    private func deliverDisplayPipelineReadyIfNeeded(reason: String) {
        guard presentationActive, let context = currentTraceContext(),
            let presentationEventSink
        else { return }
        stateLock.lock()
        let shouldDeliver = didReportDisplayPipelineReady && !didDeliverDisplayPipelineReady
        if shouldDeliver { didDeliverDisplayPipelineReady = true }
        stateLock.unlock()
        guard shouldDeliver else { return }
        IOSRuntimeTrace.state(
            domain: "ios-streaming", event: "displayPipelineReadyDelivered",
            payload: streamingPresentationTracePayload(
                [
                    "reason": .string(reason),
                    "presentationRevision": .integer(Int64(clamping: presentationRevision)),
                ], context: context),
            dimension: .presentation, importance: .essential, operationID: context.attemptID)
        presentationEventSink(.displayPipelineReady(context))
    }

    fileprivate var hasPendingVideoFrame: Bool { mailbox.hasPendingFrame() }

    private func didReportFirstFrameAcceptedForCurrentContext() -> Bool {
        stateLock.lock()
        defer { stateLock.unlock() }
        if didReportFirstFrameAccepted { return true }
        didReportFirstFrameAccepted = true
        return false
    }

    private func didReportFirstFrameSubmittedForCurrentContext() -> Bool {
        stateLock.lock()
        defer { stateLock.unlock() }
        if didReportFirstFrameSubmitted { return true }
        didReportFirstFrameSubmitted = true
        return false
    }

    private func didReportFirstFramePresentedForCurrentContext() -> Bool {
        stateLock.lock()
        defer { stateLock.unlock() }
        if didReportFirstFramePresented { return true }
        didReportFirstFramePresented = true
        return false
    }

    private func currentTraceContext() -> StreamingPresentationTraceContext? {
        stateLock.lock()
        let context = traceContext
        stateLock.unlock()
        return context
    }

    @MainActor
    private func emitPresentationEvent(_ event: StreamingPresentationLifecycleEvent) {
        guard presentationActive, currentTraceContext() == event.context else { return }
        switch event {
        case .displayHealth(let context, let snapshot):
            if let immediate = healthCoalescer.offer(context: context, snapshot: snapshot) {
                presentationEventSink?(immediate)
            }
            guard healthCoalescer.pending != nil, healthReportTask == nil else { return }
            healthReportTask = Task { @MainActor [weak self] in
                do { try await Task.sleep(for: StreamingDisplayHealthCoalescer.interval) } catch {
                    return
                }
                self?.flushDisplayHealth()
            }
        case .firstFramePresented:
            flushDisplayHealth()
            presentationEventSink?(event)
        default:
            presentationEventSink?(event)
        }
    }

    @MainActor
    private func flushDisplayHealth(reset: Bool = false) {
        healthReportTask?.cancel()
        healthReportTask = nil
        if let event = healthCoalescer.flush() { presentationEventSink?(event) }
        if reset { healthCoalescer.reset() }
    }

    static func supportsPictureRendering(on device: MTLDevice) -> Bool {
        makePipelineStates(device: device) != nil
    }

    private static func makePipelineStates(
        device: MTLDevice?
    ) -> (
        nv12: MTLRenderPipelineState,
        i420: MTLRenderPipelineState,
        bgra: MTLRenderPipelineState
    )? {
        guard let device else { return nil }
        do {
            let library = try device.makeLibrary(source: streamingMetalShaderSource, options: nil)
            let vertex = library.makeFunction(name: "streamingVertex")
            let nv12Fragment = library.makeFunction(name: "streamingNV12Fragment")
            let i420Fragment = library.makeFunction(name: "streamingI420Fragment")
            let bgraFragment = library.makeFunction(name: "streamingBGRAFragment")
            guard let vertex, let nv12Fragment, let i420Fragment, let bgraFragment else {
                return nil
            }

            let nv12Descriptor = MTLRenderPipelineDescriptor()
            nv12Descriptor.vertexFunction = vertex
            nv12Descriptor.fragmentFunction = nv12Fragment
            nv12Descriptor.colorAttachments[0].pixelFormat = .bgra8Unorm

            let i420Descriptor = MTLRenderPipelineDescriptor()
            i420Descriptor.vertexFunction = vertex
            i420Descriptor.fragmentFunction = i420Fragment
            i420Descriptor.colorAttachments[0].pixelFormat = .bgra8Unorm

            let bgraDescriptor = MTLRenderPipelineDescriptor()
            bgraDescriptor.vertexFunction = vertex
            bgraDescriptor.fragmentFunction = bgraFragment
            bgraDescriptor.colorAttachments[0].pixelFormat = .bgra8Unorm

            return (
                try device.makeRenderPipelineState(descriptor: nv12Descriptor),
                try device.makeRenderPipelineState(descriptor: i420Descriptor),
                try device.makeRenderPipelineState(descriptor: bgraDescriptor)
            )
        } catch {
            return nil
        }
    }
}

@MainActor
private final class StreamingMetalRenderScheduler: NSObject {
    private weak var target: StreamingMetalVideoSurfaceView?
    private var displayLink: CADisplayLink?
    private var isActive = false

    func attach(_ target: StreamingMetalVideoSurfaceView) {
        self.target = target
        let displayLink = CADisplayLink(target: self, selector: #selector(handleDisplayTick))
        displayLink.preferredFramesPerSecond = 60
        displayLink.add(to: .main, forMode: .common)
        displayLink.isPaused = true
        self.displayLink = displayLink
    }

    func setActive(_ active: Bool) {
        isActive = active
        scheduleDisplayTickIfNeeded()
    }

    func requestTick() {
        guard isActive, let target else { return }
        if target.hasPendingVideoFrame { target.draw() }
        scheduleDisplayTickIfNeeded()
    }

    func scheduleDisplayTickIfNeeded() {
        displayLink?.isPaused = !(isActive && target?.hasPendingVideoFrame == true)
    }

    func invalidate() {
        isActive = false
        displayLink?.invalidate()
        displayLink = nil
        target = nil
    }

    @objc
    private func handleDisplayTick() {
        requestTick()
    }
}
