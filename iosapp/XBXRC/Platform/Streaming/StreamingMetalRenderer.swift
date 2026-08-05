import CoreVideo
import Foundation
import Metal
import MetalKit
import QuartzCore
import UIKit
import simd

#if canImport(WebRTC)
@preconcurrency import WebRTC
#endif

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
            "displayedFrameDelta": displayedFrameDelta.map(IOSRuntimeTraceValue.integer) ?? .null,
            "mailboxOverwriteCount": .integer(mailboxOverwriteCount),
            "dropCountByReason": .object(dropCountByReason.mapValues(IOSRuntimeTraceValue.integer)),
            "currentDrawableBusy": .bool(currentDrawableBusy),
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
        case let .displayPipelineReady(context),
             let .firstFrameAccepted(context),
             let .firstFrameSubmitted(context),
             let .firstFramePresented(context),
             let .displayHealth(context, _):
            context
        }
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

#if canImport(WebRTC)
private enum StreamingVideoFrameDropReason: String {
    case ingressCoalesced
    case unsupportedBufferType
    case unsupportedPixelFormat
    case textureImportFailed
    case drawableUnavailable
    case encoderUnavailable
    case pipelineUnavailable
}

private enum StreamingDecodedFrameStorage: @unchecked Sendable {
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
private struct StreamingSimulatorI420Frame: @unchecked Sendable {
    // mailbox 强引用 WebRTC buffer，确保异步 GPU 上传前 Y/U/V plane 生命周期有效。
    let buffer: any RTCI420BufferProtocol
}
#endif

private struct StreamingDecodedFrameRef: @unchecked Sendable {
    let identifier: UInt64
    let storage: StreamingDecodedFrameStorage
    let width: Int
    let height: Int
    let rotationDegrees: Int
    let isFullRange: Bool
    let traceContext: StreamingPresentationTraceContext?
}

private struct StreamingMailboxSubmissionResult {
    let overwroteCandidate: Bool
}

private struct StreamingMailboxDrainResult {
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

private final class StreamingLatestFrameMailbox {
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
        traceContext: StreamingPresentationTraceContext?
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
            traceContext: traceContext
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
}

private final class StreamingDisplayTelemetry {
    private let lock = NSLock()
    private var submitCount: Int64 = 0
    private var presentCount: Int64 = 0
    private var lastSubmitAtMilliseconds: Double?
    private var lastPresentAtMilliseconds: Double?
    private var lastReportedPresentCount: Int64 = 0
    private var mailboxOverwriteCount: Int64 = 0
    private var dropCountByReason: [String: Int64] = [:]
    private var currentDrawableBusy = false
    private var pendingSubmissionAtMilliseconds: [UInt64: Double] = [:]
    private var submitToPresentSamples: [Double] = []

    func reset() -> StreamingDisplayHealthSnapshot {
        lock.lock()
        submitCount = 0
        presentCount = 0
        lastSubmitAtMilliseconds = nil
        lastPresentAtMilliseconds = nil
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
        timestampMilliseconds: Double
    ) -> StreamingDisplayHealthSnapshot {
        lock.lock()
        submitCount += 1
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
            currentDrawableBusy: currentDrawableBusy
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
    float y = lumaTexture.sample(textureSampler, in.texCoord).r;
    float2 uv = chromaTexture.sample(textureSampler, in.texCoord).rg;
    float3 yuv = float3(y, uv.x, uv.y) + uniforms.colorOffset;
    float3 rgb = uniforms.colorMatrix * yuv;
    return float4(rgb, 1.0);
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
    float y = lumaTexture.sample(textureSampler, in.texCoord).r;
    float u = uTexture.sample(textureSampler, in.texCoord).r;
    float v = vTexture.sample(textureSampler, in.texCoord).r;
    float3 yuv = float3(y, u, v) + uniforms.colorOffset;
    float3 rgb = uniforms.colorMatrix * yuv;
    return float4(rgb, 1.0);
}

fragment float4 streamingBGRAFragment(
    VertexOut in [[stage_in]],
    texture2d<float> videoTexture [[texture(0)]]
) {
    constexpr sampler textureSampler(
        coord::normalized,
        address::clamp_to_edge,
        filter::linear
    );
    return videoTexture.sample(textureSampler, in.texCoord);
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

    func reset() {
        lock.lock()
        pendingValue = nil
        deliveryScheduled = false
        replacedCount = 0
        lock.unlock()
    }
}

private final class StreamingVideoFrameIngress: NSObject, RTCVideoRenderer, @unchecked Sendable {
    private let frameSlot = StreamingLatestIngressSlot<RTCVideoFrame>()
    @MainActor var frameSink: (@MainActor (RTCVideoFrame, Int64) -> Void)?
    @MainActor var sizeSink: (@MainActor (CGSize) -> Void)?

    func setSize(_ size: CGSize) {
        guard size.width > 0, size.height > 0 else { return }
        Task { @MainActor [weak self] in
            self?.sizeSink?(size)
        }
    }

    func renderFrame(_ frame: RTCVideoFrame?) {
        guard let frame else { return }
        if frameSlot.submit(frame) { scheduleDrain() }
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
        guard let delivery = frameSlot.take() else {
            _ = frameSlot.finishDelivery()
            return
        }
        frameSink?(delivery.value, delivery.replacedCount)
        if frameSlot.finishDelivery() { scheduleDrain() }
    }
}

final class StreamingMetalVideoSurfaceView: MTKView, MTKViewDelegate, @unchecked Sendable {
    @MainActor var presentationEventSink: (@MainActor (StreamingPresentationLifecycleEvent) -> Void)?
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
    private let stateLock = NSLock()
    private let commandQueue: MTLCommandQueue?
    private let nv12PipelineState: MTLRenderPipelineState?
    private let i420PipelineState: MTLRenderPipelineState?
    private let bgraPipelineState: MTLRenderPipelineState?
    private var textureCache: CVMetalTextureCache?
    private var traceContext: StreamingPresentationTraceContext?
    private var lastReportedSurfaceSize: CGSize = .zero
    private var lastReportedFrameSize: CGSize = .zero
    private var didReportDisplayPipelineReady = false
    private var didReportFirstFrameAccepted = false
    private var didReportFirstFrameSubmitted = false
    private var didReportFirstFramePresented = false

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
        frameIngress.frameSink = { [weak self] frame, replacedCount in
            self?.consumeFrame(frame, ingressReplacedCount: replacedCount)
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
        scheduler.setActive(window != nil)
        if window != nil { scheduler.requestTick() }
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        reportSurfaceSizeIfNeeded()
    }

    func updateTraceContext(_ context: StreamingPresentationTraceContext?) {
        stateLock.lock()
        guard streamingPresentationContextDidChange(previous: traceContext, next: context) else {
            stateLock.unlock()
            return
        }
        traceContext = context
        lastReportedSurfaceSize = .zero
        lastReportedFrameSize = .zero
        didReportDisplayPipelineReady = false
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

    func shutdown() {
        frameIngress.reset()
        frameIngress.frameSink = nil
        frameIngress.sizeSink = nil
        scheduler.invalidate()
    }

    private func consumeFrameSize(_ size: CGSize) {
        guard size.width > 0, size.height > 0 else { return }
        reportDisplayPipelineReadyIfNeeded(frameSize: size)
    }

    private func consumeFrame(_ frame: RTCVideoFrame, ingressReplacedCount: Int64) {
        let ingressContext = currentTraceContext()
        if ingressReplacedCount > 0 {
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
            let supportedFormat = pixelFormat == kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange
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

        let decodedFrame = mailbox.makeFrame(
            storage: decodedStorage,
            width: Int(frame.width),
            height: Int(frame.height),
            rotationDegrees: Int(frame.rotation.rawValue),
            isFullRange: isFullRange,
            traceContext: context
        )
        let submission = mailbox.submit(decodedFrame)
        if let context {
            var acceptedPayload: [String: IOSRuntimeTraceValue] = [
                "frameWidth": .integer(Int64(frame.width)),
                "frameHeight": .integer(Int64(frame.height)),
                "rotationDegrees": .integer(Int64(decodedFrame.rotationDegrees)),
            ]
            if let compatibilityPath {
                acceptedPayload["compatibilityPath"] = .string(compatibilityPath)
            }
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
            if !didReportFirstFrameAcceptedForCurrentContext() {
                emitPresentationEvent(.firstFrameAccepted(context))
            }
            if submission.overwroteCandidate {
                IOSRuntimeTrace.state(
                    domain: "ios-streaming",
                    event: "mailboxOverwrite",
                    payload: streamingPresentationTracePayload(
                        [
                            "mailboxOverwriteCount": .integer(mailbox.snapshotOverwriteCount()),
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
        let drain = mailbox.beginRender()
        if drain.drawableBusy { return }
        guard let frame = drain.frame else { return }
        guard let traceContext = frame.traceContext else {
            _ = mailbox.finishRender(identifier: frame.identifier, requeueAsLatest: false)
            return
        }
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

        guard let renderResources = makeRenderResources(for: frame) else {
            handleSkippedPresent(
                frame: frame,
                reason: .textureImportFailed,
                requeueAsLatest: false
            )
            return
        }

        guard let commandBuffer = commandQueue.makeCommandBuffer(),
              let encoder = commandBuffer.makeRenderCommandEncoder(descriptor: descriptor)
        else {
            handleSkippedPresent(
                frame: frame,
                reason: .encoderUnavailable,
                requeueAsLatest: false
            )
            return
        }

        emitPresentationEvent(.displayHealth(traceContext, telemetry.setDrawableBusy(true)))
        let submitTimestamp = Date().timeIntervalSince1970 * 1_000
        let submitSnapshot = telemetry.recordSubmit(
            identifier: frame.identifier,
            timestampMilliseconds: submitTimestamp
        )
        emitPresentationEvent(.displayHealth(traceContext, submitSnapshot))
        var submitPayload: [String: IOSRuntimeTraceValue] = [
            "submitCount": .integer(submitSnapshot.submitCount),
            "frameWidth": .integer(Int64(frame.width)),
            "frameHeight": .integer(Int64(frame.height)),
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
        encoder.setRenderPipelineState(renderResources.pipelineState)
        encoder.setVertexBytes(
            &vertexUniforms,
            length: MemoryLayout<StreamingVertexUniforms>.stride,
            index: 0
        )
        encoder.setFragmentTexture(renderResources.primaryTexture, index: 0)
        if let secondaryTexture = renderResources.secondaryTexture {
            var fragmentUniforms = renderResources.fragmentUniforms
            encoder.setFragmentTexture(secondaryTexture, index: 1)
            encoder.setFragmentBytes(
                &fragmentUniforms,
                length: MemoryLayout<StreamingFragmentUniforms>.stride,
                index: 0
            )
        }
        if let tertiaryTexture = renderResources.tertiaryTexture {
            encoder.setFragmentTexture(tertiaryTexture, index: 2)
        }
        encoder.drawPrimitives(type: .triangleStrip, vertexStart: 0, vertexCount: 4)
        encoder.endEncoding()

        commandBuffer.addCompletedHandler { [weak self] completedCommandBuffer in
            let completedSuccessfully = completedCommandBuffer.status == .completed
            Task { @MainActor in
                self?.handleCompletedCommandBuffer(
                    completedSuccessfully: completedSuccessfully,
                    frame: frame,
                    context: traceContext
                )
            }
        }
        commandBuffer.present(drawable)
        commandBuffer.commit()
    }

    private func handleCompletedCommandBuffer(
        completedSuccessfully: Bool,
        frame: StreamingDecodedFrameRef,
        context: StreamingPresentationTraceContext
    ) {
        guard currentTraceContext() == context else { return }
        guard completedSuccessfully else {
            telemetry.cancelSubmit(identifier: frame.identifier)
            handleSkippedPresent(
                frame: frame,
                reason: .drawableUnavailable,
                requeueAsLatest: true
            )
            return
        }
        let hasPending = handlePresentedFrame(frame, context: context)
        let health = telemetry.setDrawableBusy(false)
        emitPresentationEvent(.displayHealth(context, health))
        if hasPending { scheduler.requestTick() }
    }

    @discardableResult
    private func handlePresentedFrame(
        _ frame: StreamingDecodedFrameRef,
        context: StreamingPresentationTraceContext
    ) -> Bool {
        let presentTimestamp = Date().timeIntervalSince1970 * 1_000
        let hasPending = mailbox.finishRender(identifier: frame.identifier, requeueAsLatest: false)
        let present = telemetry.recordPresent(
            identifier: frame.identifier,
            timestampMilliseconds: presentTimestamp
        )
        emitPresentationEvent(.displayHealth(context, present.snapshot))
        var presentPayload: [String: IOSRuntimeTraceValue] = [
            "presentCount": .integer(present.snapshot.presentCount),
            "frameWidth": .integer(Int64(frame.width)),
            "frameHeight": .integer(Int64(frame.height)),
        ]
        if let compatibilityPath = frame.storage.compatibilityPath {
            presentPayload["compatibilityPath"] = .string(compatibilityPath)
        }
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
                "presentCount": .integer(present.snapshot.presentCount),
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
        return hasPending
    }

    private func handleSkippedPresent(
        frame: StreamingDecodedFrameRef,
        reason: StreamingVideoFrameDropReason,
        requeueAsLatest: Bool
    ) {
        let context = frame.traceContext
        let hasPending = mailbox.finishRender(identifier: frame.identifier, requeueAsLatest: requeueAsLatest)
        recordDrop(reason: reason, context: context)
        if let context {
            var skippedPayload: [String: IOSRuntimeTraceValue] = [
                "reason": .string(reason.rawValue),
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
            emitPresentationEvent(.displayHealth(context, telemetry.setDrawableBusy(false)))
        }
        if StreamingRenderRetryPolicy.shouldRequestImmediateTick(
            requeueAsLatest: requeueAsLatest,
            hasPendingFrame: hasPending
        ) {
            scheduler.requestTick()
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
        let fragmentUniforms = makeFragmentUniforms(fullRange: frame.isFullRange)
        switch frame.storage {
        case let .pixelBuffer(pixelBuffer):
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
        case let .simulatorI420(i420Frame):
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
        let width = isPlanar
            ? max(CVPixelBufferGetWidthOfPlane(pixelBuffer, planeIndex), 1)
            : max(CVPixelBufferGetWidth(pixelBuffer), 1)
        let height = isPlanar
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
        let frameAspect = frameWidth / frameHeight
        let drawableAspect = drawableWidth / drawableHeight
        if frameAspect > drawableAspect {
            return SIMD2<Float>(1, Float(drawableAspect / frameAspect))
        } else {
            return SIMD2<Float>(Float(frameAspect / drawableAspect), 1)
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
            hasPipeline = device != nil
                && commandQueue != nil
                && (nv12PipelineState != nil || bgraPipelineState != nil)
                && textureCache != nil
        case .simulatorI420:
            hasPipeline = device != nil
                && commandQueue != nil
                && i420PipelineState != nil
        case nil:
            return
        }
        #else
        let hasPipeline = device != nil
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
        if let frameSize, frameSize.width > 0, frameSize.height > 0, frameSize != lastReportedFrameSize {
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
            emitPresentationEvent(.displayPipelineReady(context))
        }

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

    private func emitPresentationEvent(_ event: StreamingPresentationLifecycleEvent) {
        Task { @MainActor [weak self] in
            self?.presentationEventSink?(event)
        }
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
            guard let vertex, let nv12Fragment, let i420Fragment, let bgraFragment else { return nil }

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
        displayLink?.isPaused = !active
    }

    func requestTick() {
        guard isActive else { return }
        target?.draw()
    }

    func invalidate() {
        isActive = false
        displayLink?.invalidate()
        displayLink = nil
        target = nil
    }

    @objc
    private func handleDisplayTick() {
        guard isActive else { return }
        target?.draw()
    }
}
#endif
