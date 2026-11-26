import CoreVideo
import Foundation
@preconcurrency import Metal
import QuartzCore
@preconcurrency import WebRTC
import simd

#if !targetEnvironment(simulator)
    @preconcurrency import MetalFX
#endif

private struct StreamingWorkerVideoFrame: @unchecked Sendable {
    let frame: RTCVideoFrame
    let receivedAtMs: Double
    let epoch: UInt64
    let lease: StreamingVideoIngressLease?
}

private struct StreamingGeometryKey: Equatable {
    let source: SIMD2<Int>
    let output: SIMD2<Int>
    let rotation: Int
    let mode: StreamingVideoContentMode
}

/// GPU 完成票据只持有当前帧与本次编码使用的 SDK 资源。
private struct StreamingGpuFrameResources: @unchecked Sendable {
    let imported: ImportedFrameResources
    let effects: [AnyObject]
}

final class StreamingRenderWorker: Sendable {
    let renderFence = StreamingExecutionFence()
    private let cache: StreamingRenderResourceCache
    private let mailbox = StreamingLatestFrameMailbox()
    private let frameSlot = StreamingLatestIngressSlot<StreamingWorkerVideoFrame>()
    private let telemetry = StreamingDisplayTelemetry()
    private let diagnostics = StreamingDiagnosticsActor()
    private let eventSink:
        @Sendable (StreamingPresentationLifecycleEvent, StreamExecutionScope, UInt64) -> Void
    private let colorSink:
        @Sendable (StreamingVideoColorConfiguration, StreamExecutionScope) -> Void
    // 以下可变状态仅在 cache.queue 上访问，SDK 完成回调使用独立票据与短锁事实入口。
    private nonisolated(unsafe) var scope = StreamExecutionScope()
    private nonisolated(unsafe) var localRenderEpoch: UInt64 = 0
    private nonisolated(unsafe) var surface: PresentationSurfaceLease?
    private nonisolated(unsafe) var presentation = StreamingPresentationSession()
    private nonisolated(unsafe) var feedback: StreamingFrameFeedbackSession?
    private nonisolated(unsafe) var active = true
    private nonisolated(unsafe) var visibilityRevision: UInt64 = 0
    private nonisolated(unsafe) var stopped = false
    private nonisolated(unsafe) var pipelines: StreamingRenderPipelines?
    private nonisolated(unsafe) var textureCache: CVMetalTextureCache?
    private nonisolated(unsafe) var imported: ImportedFrameResources?
    private nonisolated(unsafe) var picture = MobilePicturePreferences()
    private nonisolated(unsafe) var effectRevision: UInt64 = 0
    private nonisolated(unsafe) var contentMode: StreamingVideoContentMode = .fit
    private nonisolated(unsafe) var resources = StreamingRenderResourceCounters()
    private nonisolated(unsafe) var renderScheduled = false
    private nonisolated(unsafe) var retry: DispatchSourceTimer?
    private nonisolated(unsafe) var pipelineReadyDelivered = false
    private nonisolated(unsafe) var firstAccepted = false
    private nonisolated(unsafe) var firstSubmitted = false
    private nonisolated(unsafe) var geometry:
        (key: StreamingGeometryKey, value: StreamingVertexUniforms)?
    private nonisolated(unsafe) var color:
        (
            key: StreamingVideoColorConfiguration, picture: MobilePicturePreferences,
            value: StreamingFragmentUniforms
        )?
    private nonisolated(unsafe) var finalPass = MTLRenderPassDescriptor()
    private nonisolated(unsafe) var effectsPass = MTLRenderPassDescriptor()
    #if targetEnvironment(simulator)
        private nonisolated(unsafe) var simulatorTextures:
            (size: SIMD4<Int>, textures: [MTLTexture])?
    #else
        private nonisolated(unsafe) var scalerResources:
            (
                size: SIMD4<Int>, revision: UInt64, scaler: any MTLFXSpatialScaler,
                input: MTLTexture, output: MTLTexture
            )?
    #endif

    init(
        cache: StreamingRenderResourceCache,
        eventSink:
            @escaping @Sendable (StreamingPresentationLifecycleEvent, StreamExecutionScope, UInt64)
            -> Void,
        colorSink:
            @escaping @Sendable (StreamingVideoColorConfiguration, StreamExecutionScope) -> Void
    ) {
        self.cache = cache
        self.eventSink = eventSink
        self.colorSink = colorSink
    }

    func updateIdentity(
        context: StreamingPresentationTraceContext?, feedback: StreamingFrameFeedbackSession?,
        presentation: StreamingPresentationSession
    ) {
        let epoch = renderFence.advance()
        frameSlot.reset()
        cache.queue.async { [self] in
            let previousContext = scope.presentation
            let samePeer =
                previousContext != nil && previousContext?.peerEpoch == context?.peerEpoch
                && previousContext?.attemptID == context?.attemptID
                && previousContext?.generation == context?.generation
            let previous = presentation.snapshot
            scope.presentation = context
            localRenderEpoch = epoch
            scope.renderEpoch = presentation.reserveRenderEpoch()
            self.feedback = feedback
            self.presentation = presentation
            mailbox.clearCandidates(resetCounters: !samePeer)
            imported = nil
            pipelineReadyDelivered = false
            firstAccepted = false
            firstSubmitted = false
            telemetry.reset(preservePeerCounters: samePeer)
            if let previous, previous.scope.presentation?.peerEpoch == context?.peerEpoch,
                previous.scope.presentation?.attemptID == context?.attemptID
            {
                telemetry.restorePeerCounters(previous.health)
            }
            bindObservation()
            deliverPipelineReady()
            requestRender()
        }
    }

    func configure(
        surface: PresentationSurfaceLease, picture: MobilePicturePreferences,
        mode: StreamingVideoContentMode, active: Bool
    ) {
        cache.queue.async { [self] in
            guard surface.isValid, !stopped else { return }
            self.surface = surface
            scope.surfaceRevision = surface.revision
            scope.configurationRevision &+= 1
            if self.picture != picture { effectRevision &+= 1 }
            self.picture = picture
            contentMode = mode
            self.active = active
            if !active {
                mailbox.clearCandidates()
                frameSlot.reset()
                imported = nil
                pipelineReadyDelivered = false
                firstAccepted = false
                firstSubmitted = false
            }
            if pipelines == nil, let device = surface.layer.device {
                pipelines = cache.resources(on: device)
            }
            if textureCache == nil, let device = pipelines?.device {
                CVMetalTextureCacheCreate(kCFAllocatorDefault, nil, device, nil, &textureCache)
            }
            resources.pipelineLoadCount = cache.counts.loaded
            resources.pipelineCreateCount = cache.counts.created
            bindObservation()
            deliverPipelineReady()
            requestRender()
        }
    }

    func setPresentationActive(_ active: Bool, revision: UInt64) {
        let epoch = active ? nil : renderFence.advance()
        cache.queue.async { [self] in
            self.active = active
            visibilityRevision = revision
            if !active {
                localRenderEpoch = epoch ?? localRenderEpoch
                scope.renderEpoch = presentation.reserveRenderEpoch()
                frameSlot.reset()
                mailbox.clearCandidates()
                imported = nil
                pipelineReadyDelivered = false
                firstAccepted = false
                firstSubmitted = false
                telemetry.reset(preservePeerCounters: true)
                bindObservation()
            } else {
                presentation.setActive(true)
                deliverPipelineReady()
            }
        }
    }

    func replayPipelineReady() {
        cache.queue.async { [self] in
            pipelineReadyDelivered = false
            deliverPipelineReady()
        }
    }

    func quiesce() async {
        await withCheckedContinuation { continuation in
            cache.queue.async { [self] in
                active = false
                retry?.cancel()
                retry = nil
                presentation.setActive(false)
                continuation.resume()
            }
        }
    }

    func shutdown() {
        renderFence.advance()
        frameSlot.reset()
        cache.queue.async { [self] in
            stopped = true
            active = false
            retry?.cancel()
            retry = nil
            mailbox.clearCandidates()
            imported = nil
            releaseDynamicResources()
            presentation.setActive(false)
            Task(priority: .utility) { await diagnostics.stop() }
        }
    }

    func trim() {
        cache.queue.async { [self] in
            resources.cacheTrimCount += 1
            imported = nil
            releaseDynamicResources()
            if let textureCache { CVMetalTextureCacheFlush(textureCache, 0) }
            cache.trim()
            reportResources()
        }
    }

    func enqueue(_ frame: RTCVideoFrame?, lease: StreamingVideoIngressLease? = nil) {
        guard let frame, lease?.isValid ?? true else { return }
        let queued = StreamingWorkerVideoFrame(
            frame: frame,
            receivedAtMs: StreamingFrameFeedbackClock.nowMilliseconds, epoch: renderFence.current,
            lease: lease)
        if let ticket = frameSlot.submit(queued, scheduledAtMs: queued.receivedAtMs) {
            cache.queue.async { [self] in drain(ticket) }
        }
    }

    func setFrameSize(_ size: CGSize) {
        cache.queue.async { [self] in
            if size.width > 0, size.height > 0 { deliverPipelineReady() }
        }
    }

    private func drain(_ ticket: StreamingIngressDrainTicket) {
        if let next = frameSlot.drain(
            ticket,
            consume: { [self] delivery in accept(delivery.value, replaced: delivery.replacedCount) }
        ) {
            cache.queue.async { [self] in drain(next) }
        }
        requestRender()
    }

    private func accept(_ queued: StreamingWorkerVideoFrame, replaced: Int64) {
        dispatchPrecondition(condition: .onQueue(cache.queue))
        guard queued.lease?.isValid ?? true, renderFence.accepts(queued.epoch),
            queued.epoch == localRenderEpoch,
            !stopped, let context = scope.presentation
        else { return }
        deliverPipelineReady()
        guard active else { return }
        let frame = queued.frame
        let storage: StreamingDecodedFrameStorage
        let fullRange: Bool
        if let cv = frame.buffer as? RTCCVPixelBuffer {
            let format = CVPixelBufferGetPixelFormatType(cv.pixelBuffer)
            guard
                [
                    kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange,
                    kCVPixelFormatType_420YpCbCr8BiPlanarFullRange, kCVPixelFormatType_32BGRA,
                ].contains(format)
            else {
                drop(.unsupportedPixelFormat, context: context)
                return
            }
            storage = .pixelBuffer(cv.pixelBuffer)
            fullRange = format == kCVPixelFormatType_420YpCbCr8BiPlanarFullRange
        } else {
            #if targetEnvironment(simulator)
                storage = .simulatorI420(.init(buffer: frame.buffer.toI420()))
                fullRange = false
            #else
                drop(.unsupportedBufferType, context: context)
                return
            #endif
        }
        let timing = feedback?.takeDecodedTiming(
            serverDataKey: UInt32(bitPattern: frame.timeStamp),
            nowMilliseconds: StreamingFrameFeedbackClock.nowMilliseconds)
        let decoded = mailbox.makeFrame(
            storage: storage, width: Int(frame.width), height: Int(frame.height),
            rotationDegrees: Int(frame.rotation.rawValue), isFullRange: fullRange,
            traceContext: context,
            frameFeedback: timing.flatMap { timing in
                feedback.map { .init(session: $0, timing: timing) }
            })
        _ = mailbox.submit(decoded)
        if replaced > 0 { telemetry.recordDrop(reason: .ingressCoalesced, count: replaced) }
        telemetry.recordMailboxOverwrite(totalOverwriteCount: mailbox.snapshotOverwriteCount())
        if IOSRuntimeTrace.shouldRecordPresentationFrame(
            counter: Int64(clamping: decoded.identifier), firstFrame: !firstAccepted)
        {
            presentation.record(
                .init(
                    event: "videoFrameAcceptedToMailbox", context: context,
                    frameID: decoded.identifier, atMs: queued.receivedAtMs))
        }
        if !firstAccepted {
            firstAccepted = true
            eventSink(.firstFrameAccepted(context), scope, visibilityRevision)
        }
        reportResources()
    }

    private func requestRender() {
        dispatchPrecondition(condition: .onQueue(cache.queue))
        guard !renderScheduled, active, !stopped, mailbox.hasPendingFrame(),
            !mailbox.snapshotDrawableBusy()
        else { return }
        renderScheduled = true
        cache.queue.async { [self] in
            renderScheduled = false
            autoreleasepool { renderOneFrame() }
        }
    }

    private func renderOneFrame() {
        guard active, !stopped, let surface, surface.isDrawableReady, let pipelines,
            mailbox.hasPendingFrame(), !mailbox.snapshotDrawableBusy(),
            renderFence.accepts(localRenderEpoch)
        else { return }
        let currentScope = scope
        let currentLocalEpoch = localRenderEpoch
        let currentVisibility = visibilityRevision
        let startedAt = StreamingFrameFeedbackClock.nowMilliseconds
        telemetry.setDrawableAcquire(startedAt)
        presentation.publish(scope: currentScope, health: telemetry.currentSnapshot())
        // drawable 等待期间 SDK 持续覆盖入口；获得 drawable 后才选取最新帧。
        let drawable = surface.layer.nextDrawable()
        telemetry.setDrawableAcquire(nil)
        guard surface.isValid, renderFence.accepts(currentLocalEpoch), active else { return }
        if let delivery = frameSlot.takePending() {
            accept(delivery.value, replaced: delivery.replacedCount)
        }
        guard let drawable else {
            if let context = scope.presentation { drop(.drawableUnavailable, context: context) }
            scheduleRetry()
            return
        }
        let acquireMs = StreamingFrameFeedbackClock.nowMilliseconds - startedAt
        guard let frame = mailbox.beginRender().frame, let context = frame.traceContext else {
            return
        }
        guard let imported = importFrame(frame),
            let commandBuffer = pipelines.commandQueue.makeCommandBuffer()
        else {
            finishSkipped(frame, .textureImportFailed, requeue: false)
            return
        }
        let uniforms = makeUniforms(frame: frame, output: drawable.texture)
        let effect = encodeSuperResolution(
            frame: frame, imported: imported, uniforms: uniforms.fragment,
            commandBuffer: commandBuffer)
        finalPass.colorAttachments[0].texture = drawable.texture
        finalPass.colorAttachments[0].loadAction = .clear
        finalPass.colorAttachments[0].storeAction = .store
        finalPass.colorAttachments[0].clearColor = MTLClearColorMake(0, 0, 0, 1)
        guard let encoder = commandBuffer.makeRenderCommandEncoder(descriptor: finalPass) else {
            finalPass.colorAttachments[0].texture = nil
            finishSkipped(frame, .encoderUnavailable, requeue: false)
            return
        }
        finalPass.colorAttachments[0].texture = nil
        var vertex = uniforms.vertex
        var fragment =
            effect.texture == nil
            ? uniforms.fragment
            : StreamingFragmentUniforms(colorMatrix: matrix_identity_float3x3, colorOffset: .zero)
        encoder.setRenderPipelineState(effect.texture == nil ? imported.pipeline : pipelines.bgra)
        encoder.setVertexBytes(
            &vertex, length: MemoryLayout<StreamingVertexUniforms>.stride, index: 0)
        encoder.setFragmentBytes(
            &fragment, length: MemoryLayout<StreamingFragmentUniforms>.stride, index: 0)
        encoder.setFragmentTexture(effect.texture ?? imported.primary, index: 0)
        if effect.texture == nil {
            encoder.setFragmentTexture(imported.secondary, index: 1)
            encoder.setFragmentTexture(imported.tertiary, index: 2)
        }
        encoder.drawPrimitives(type: .triangleStrip, vertexStart: 0, vertexCount: 4)
        encoder.endEncoding()
        guard surface.isValid, renderFence.accepts(currentLocalEpoch) else {
            finishSkipped(frame, .drawableUnavailable, requeue: false)
            return
        }
        let submittedAt = StreamingFrameFeedbackClock.nowMilliseconds
        let health = telemetry.recordSubmit(
            identifier: frame.identifier, timestampMilliseconds: submittedAt,
            configuration: .init(
                sourceWidth: frame.width, sourceHeight: frame.height,
                outputWidth: Int(Float(drawable.texture.width) * abs(vertex.scale.x)),
                outputHeight: Int(Float(drawable.texture.height) * abs(vertex.scale.y)),
                superResolutionActive: effect.texture != nil, sharpness: picture.sharpness))
        telemetry.setDrawableBusy(true)
        presentation.publish(scope: currentScope, health: health)
        if IOSRuntimeTrace.shouldRecordPresentationFrame(
            counter: health.submitCount, firstFrame: !firstSubmitted)
        {
            presentation.record(
                .init(
                    event: "displaySubmit", context: context, frameID: frame.identifier,
                    atMs: submittedAt, count: health.submitCount, durationMs: acquireMs))
        }
        if !firstSubmitted {
            firstSubmitted = true
            eventSink(.firstFrameSubmitted(context), scope, visibilityRevision)
        }
        let ticket = StreamingGpuFrameResources(imported: imported, effects: effect.retained)
        let session = presentation
        let eventSink = eventSink
        let renderFence = renderFence
        commandBuffer.addCompletedHandler { [weak self, mailbox, telemetry, ticket] buffer in
            let completedAt = StreamingFrameFeedbackClock.nowMilliseconds
            let current =
                renderFence.accepts(currentLocalEpoch) && surface.isValid
                && session.snapshot?.active == true && session.snapshot?.scope == currentScope
            let succeeded = buffer.status == .completed
            _ = mailbox.finishRender(
                identifier: frame.identifier, requeueAsLatest: !succeeded && current)
            // 无论作用域是否已退役，当前票据都在 GPU completion 后释放。
            withExtendedLifetime(ticket) {
                if current, succeeded {
                    if let feedback = frame.frameFeedback {
                        feedback.session.recordRendered(
                            timing: feedback.timing, atMilliseconds: completedAt)
                    }
                    let result = telemetry.recordPresent(
                        identifier: frame.identifier, timestampMilliseconds: completedAt,
                        drawableBusy: mailbox.snapshotDrawableBusy())
                    session.publish(scope: currentScope, health: result.snapshot)
                    if IOSRuntimeTrace.shouldRecordPresentationFrame(
                        counter: result.snapshot.presentCount, firstFrame: result.isFirstPresent)
                    {
                        session.record(
                            .init(
                                event: "displayPresent", context: context,
                                frameID: frame.identifier,
                                atMs: completedAt, count: result.snapshot.presentCount,
                                timestampSource: "commandBufferCompletion"))
                    }
                    #if targetEnvironment(simulator)
                        if result.isFirstPresent {
                            eventSink(
                                .firstFramePresented(context), currentScope, currentVisibility)
                        }
                    #endif
                } else if current {
                    telemetry.cancelSubmit(identifier: frame.identifier)
                    telemetry.recordDrop(reason: .drawableUnavailable)
                }
            }
            let completedFrameID = frame.identifier
            self?.cache.queue.async { [weak self] in
                if succeeded, self?.imported?.frameID == completedFrameID { self?.imported = nil }
                self?.reportResources()
                self?.requestRender()
            }
        }
        #if !targetEnvironment(simulator)
            drawable.addPresentedHandler { [telemetry] drawable in
                let presentedAt = drawable.presentedTime * 1_000
                guard presentedAt > 0, renderFence.accepts(currentLocalEpoch), surface.isValid,
                    session.snapshot?.scope == currentScope, session.snapshot?.active == true
                else { return }
                let snapshot = telemetry.recordDrawablePresent(
                    submittedAt: submittedAt, presentedAt: presentedAt, frameID: frame.identifier)
                session.publish(scope: currentScope, health: snapshot)
                if snapshot.drawablePresentCount == 1 {
                    eventSink(.firstFramePresented(context), currentScope, currentVisibility)
                }
            }
        #endif
        commandBuffer.present(drawable)
        commandBuffer.commit()
        reportResources()
    }

    private func bindObservation() {
        let boundScope = scope
        telemetry.setPresentationScope(
            epoch: boundScope.renderEpoch,
            surfaceRevision: boundScope.surfaceRevision, active: active,
            surfaceReady: surface?.isDrawableReady == true)
        let boundSession = presentation
        presentation.bind(
            scope: boundScope, active: active, surfaceReady: surface?.isDrawableReady == true
        ) { [weak self] expected, actionID in
            await self?.resetPresentation(
                expected: expected, actionID: actionID, session: boundSession) ?? .superseded
        }
        presentation.publish(scope: boundScope, health: telemetry.currentSnapshot())
        let presentation = presentation
        let eventSink = eventSink
        let visibility = visibilityRevision
        Task(priority: .utility) { [diagnostics, telemetry] in
            await diagnostics.start(
                telemetry: telemetry, session: presentation, scope: boundScope,
                eventSink: { eventSink($0, boundScope, visibility) })
        }
    }

    private func resetPresentation(
        expected: StreamExecutionScope, actionID: UInt64,
        session: StreamingPresentationSession
    ) async -> StreamingPresentationResetOutcome {
        guard session.snapshot?.scope == expected else { return .superseded }
        let epoch = renderFence.advance()
        frameSlot.reset()
        return await withCheckedContinuation { continuation in
            cache.queue.async { [self] in
                guard !stopped, active, scope == expected, presentation === session,
                    session.acceptsReset(actionID),
                    let surface, surface.isValid
                else {
                    if !stopped, renderFence.accepts(epoch) {
                        localRenderEpoch = epoch
                        scope.renderEpoch = presentation.reserveRenderEpoch()
                        mailbox.clearCandidates()
                        imported = nil
                        bindObservation()
                    }
                    continuation.resume(returning: .superseded)
                    return
                }
                localRenderEpoch = epoch
                scope.renderEpoch = presentation.reserveRenderEpoch()
                mailbox.clearCandidates()
                imported = nil
                releaseDynamicResources()
                if let device = pipelines?.device {
                    textureCache = nil
                    CVMetalTextureCacheCreate(kCFAllocatorDefault, nil, device, nil, &textureCache)
                }
                resources.cacheResetCount += 1
                telemetry.reset(preservePeerCounters: true)
                firstAccepted = false
                firstSubmitted = false
                bindObservation()
                IOSRuntimeTrace.state(
                    domain: "ios-streaming", event: "presentationResetAcknowledged",
                    payload: [
                        "actionId": .integer(Int64(clamping: actionID)),
                        "renderEpoch": .integer(Int64(clamping: epoch)),
                    ],
                    dimension: .recovery, importance: .key,
                    operationID: scope.presentation?.attemptID)
                continuation.resume(
                    returning: textureCache != nil && pipelines != nil ? .acknowledged : .failed)
            }
        }
    }

    private func deliverPipelineReady() {
        guard active, pipelines != nil, !pipelineReadyDelivered, let context = scope.presentation
        else { return }
        pipelineReadyDelivered = true
        eventSink(.displayPipelineReady(context), scope, visibilityRevision)
        IOSRuntimeTrace.state(
            domain: "ios-streaming", event: "displayPipelineReady",
            payload: streamingPresentationTracePayload(
                ["renderer": .string("metalRenderQueue")], context: context),
            dimension: .presentation, importance: .essential, operationID: context.attemptID)
    }

    private func scheduleRetry() {
        guard active, !stopped else { return }
        if retry == nil {
            let timer = DispatchSource.makeTimerSource(queue: cache.queue)
            timer.setEventHandler { [weak self] in self?.requestRender() }
            retry = timer
            timer.resume()
        }
        let delay = telemetry.currentSnapshot().frameIntervalMilliseconds ?? 16.7
        retry?.schedule(
            deadline: .now() + .nanoseconds(Int(max(8, min(33, delay)) * 1_000_000)),
            leeway: .milliseconds(1))
    }

    private func finishSkipped(
        _ frame: StreamingDecodedFrameRef, _ reason: StreamingVideoFrameDropReason, requeue: Bool
    ) {
        _ = mailbox.finishRender(identifier: frame.identifier, requeueAsLatest: requeue)
        if !requeue, imported?.frameID == frame.identifier { imported = nil }
        if let context = frame.traceContext { drop(reason, context: context) }
        if requeue { scheduleRetry() } else { requestRender() }
    }

    private func drop(
        _ reason: StreamingVideoFrameDropReason, context: StreamingPresentationTraceContext
    ) {
        let snapshot = telemetry.recordDrop(reason: reason)
        presentation.publish(scope: scope, health: snapshot)
        presentation.record(
            .init(
                event: "displayPresentSkipped", context: context, frameID: 0,
                atMs: StreamingFrameFeedbackClock.nowMilliseconds, reason: reason.rawValue))
    }

    private func reportResources() {
        resources.applicationHeldFrameCount = Int64(
            mailbox.retainedFrameCount(including: imported?.frameID))
        resources.activeGpuTextureBytes =
            mailbox.snapshotDrawableBusy() ? (imported?.textureBytes ?? 0) : 0
        telemetry.setResources(resources)
    }

    private func releaseDynamicResources() {
        geometry = nil
        color = nil
        #if targetEnvironment(simulator)
            simulatorTextures = nil
        #else
            scalerResources = nil
        #endif
    }

    private func importFrame(_ frame: StreamingDecodedFrameRef) -> ImportedFrameResources? {
        if let imported, imported.frameID == frame.identifier,
            imported.renderEpoch == scope.renderEpoch
        {
            resources.sameFrameTextureReuseCount += 1
            return imported
        }
        guard let pipelines else { return nil }
        var refs: [CVMetalTexture] = []
        let textures: [MTLTexture]
        let pipeline: MTLRenderPipelineState
        switch frame.storage {
        case .pixelBuffer(let buffer):
            let bgra = CVPixelBufferGetPixelFormatType(buffer) == kCVPixelFormatType_32BGRA
            let formats: [MTLPixelFormat] = bgra ? [.bgra8Unorm] : [.r8Unorm, .rg8Unorm]
            var importedTextures: [MTLTexture] = []
            for (plane, format) in formats.enumerated() {
                guard let textureCache else { return nil }
                let planar = CVPixelBufferIsPlanar(buffer)
                let width =
                    planar
                    ? CVPixelBufferGetWidthOfPlane(buffer, plane) : CVPixelBufferGetWidth(buffer)
                let height =
                    planar
                    ? CVPixelBufferGetHeightOfPlane(buffer, plane) : CVPixelBufferGetHeight(buffer)
                var ref: CVMetalTexture?
                let status = CVMetalTextureCacheCreateTextureFromImage(
                    kCFAllocatorDefault, textureCache,
                    buffer, nil, format, width, height, plane, &ref)
                guard status == kCVReturnSuccess, let ref,
                    let texture = CVMetalTextureGetTexture(ref)
                else { return nil }
                refs.append(ref)
                importedTextures.append(texture)
                resources.textureImportCount += 1
            }
            textures = importedTextures
            pipeline = bgra ? pipelines.bgra : pipelines.nv12
        #if targetEnvironment(simulator)
            case .simulatorI420(let value):
                let buffer = value.buffer
                let size = SIMD4<Int>(
                    Int(buffer.width), Int(buffer.height), Int(buffer.chromaWidth),
                    Int(buffer.chromaHeight))
                if simulatorTextures?.size != size {
                    var created: [MTLTexture] = []
                    for index in 0..<3 {
                        let descriptor = MTLTextureDescriptor.texture2DDescriptor(
                            pixelFormat: .r8Unorm,
                            width: index == 0 ? size.x : size.z,
                            height: index == 0 ? size.y : size.w, mipmapped: false)
                        descriptor.usage = .shaderRead
                        descriptor.storageMode = .shared
                        guard let texture = pipelines.device.makeTexture(descriptor: descriptor)
                        else { return nil }
                        created.append(texture)
                    }
                    simulatorTextures = (size, created)
                    resources.intermediateTextureCreateCount += 3
                }
                guard let group = simulatorTextures else { return nil }
                let planes = [
                    (buffer.dataY, Int(buffer.strideY), size.x, size.y),
                    (buffer.dataU, Int(buffer.strideU), size.z, size.w),
                    (buffer.dataV, Int(buffer.strideV), size.z, size.w),
                ]
                for (index, plane) in planes.enumerated() {
                    guard plane.1 >= plane.2 else { return nil }
                    group.textures[index].replace(
                        region: MTLRegionMake2D(0, 0, plane.2, plane.3),
                        mipmapLevel: 0, withBytes: UnsafeRawPointer(plane.0), bytesPerRow: plane.1)
                }
                textures = group.textures
                pipeline = pipelines.i420
        #endif
        }
        let result = ImportedFrameResources(
            frameID: frame.identifier, renderEpoch: scope.renderEpoch,
            storage: frame.storage, textureRefs: refs, primary: textures[0],
            secondary: textures.count > 1 ? textures[1] : nil,
            tertiary: textures.count > 2 ? textures[2] : nil, pipeline: pipeline)
        imported = result
        return result
    }

    private func makeUniforms(frame: StreamingDecodedFrameRef, output: MTLTexture)
        -> (vertex: StreamingVertexUniforms, fragment: StreamingFragmentUniforms)
    {
        let key = StreamingGeometryKey(
            source: .init(frame.width, frame.height), output: .init(output.width, output.height),
            rotation: frame.rotationDegrees, mode: contentMode)
        if geometry?.key != key {
            geometry = (
                key,
                .init(
                    scale: normalizedRenderScale(
                        for: frame, drawableWidth: Double(output.width),
                        drawableHeight: Double(output.height)),
                    texCoordTransform: textureCoordinateTransform(
                        rotationDegrees: frame.rotationDegrees))
            )
            resources.uniformBuildCount += 1
        }
        if color?.key != frame.colorConfiguration || color?.picture != picture {
            let conversion = frame.colorConfiguration.conversion
            var value = StreamingFragmentUniforms(
                colorMatrix: conversion.matrix, colorOffset: conversion.offset)
            if !picture.compatibilityMode {
                value.effects = .init(
                    Float(picture.saturation / 100), Float(picture.contrast / 100),
                    Float(picture.brightness / 100),
                    picture.sharpness == 0 ? 0 : picture.sharpness == 1 ? 0.25 : 0.65)
            }
            if color?.key != frame.colorConfiguration { colorSink(frame.colorConfiguration, scope) }
            color = (frame.colorConfiguration, picture, value)
            resources.uniformBuildCount += 1
        }
        return (geometry!.value, color!.value)
    }

    private func encodeSuperResolution(
        frame: StreamingDecodedFrameRef, imported: ImportedFrameResources,
        uniforms: StreamingFragmentUniforms, commandBuffer: MTLCommandBuffer
    )
        -> (texture: MTLTexture?, retained: [AnyObject])
    {
        #if targetEnvironment(simulator)
            return (nil, [])
        #else
            guard picture.superResolution, !picture.compatibilityMode, let pipelines,
                MTLFXSpatialScalerDescriptor.supportsDevice(pipelines.device), let surface
            else { return (nil, []) }
            let rotated = frame.rotationDegrees == 90 || frame.rotationDegrees == 270
            let extent = normalizedRenderScale(
                for: frame, drawableWidth: surface.pixelSize.width,
                drawableHeight: surface.pixelSize.height)
            let scale = min(
                2,
                max(
                    surface.pixelSize.width * Double(abs(extent.x))
                        / Double(rotated ? frame.height : frame.width),
                    surface.pixelSize.height * Double(abs(extent.y))
                        / Double(rotated ? frame.width : frame.height)))
            guard scale > 1.01 else { return (nil, []) }
            let size = SIMD4<Int>(
                frame.width, frame.height, Int(Double(frame.width) * scale),
                Int(Double(frame.height) * scale))
            if scalerResources?.size != size || scalerResources?.revision != effectRevision {
                let descriptor = MTLFXSpatialScalerDescriptor()
                descriptor.inputWidth = size.x
                descriptor.inputHeight = size.y
                descriptor.outputWidth = size.z
                descriptor.outputHeight = size.w
                descriptor.colorTextureFormat = .bgra8Unorm
                descriptor.outputTextureFormat = .bgra8Unorm
                descriptor.colorProcessingMode = .perceptual
                guard let scaler = descriptor.makeSpatialScaler(device: pipelines.device) else {
                    return (nil, [])
                }
                func texture(width: Int, height: Int, usage: MTLTextureUsage) -> MTLTexture? {
                    let descriptor = MTLTextureDescriptor.texture2DDescriptor(
                        pixelFormat: .bgra8Unorm, width: width, height: height, mipmapped: false)
                    descriptor.storageMode = .private
                    descriptor.usage = usage
                    return pipelines.device.makeTexture(descriptor: descriptor)
                }
                guard
                    let input = texture(
                        width: size.x, height: size.y,
                        usage: scaler.colorTextureUsage.union([.renderTarget, .shaderRead])),
                    let output = texture(
                        width: size.z, height: size.w,
                        usage: scaler.outputTextureUsage.union(.shaderRead))
                else { return (nil, []) }
                scalerResources = (size, effectRevision, scaler, input, output)
                resources.intermediateTextureCreateCount += 2
            }
            guard let group = scalerResources else { return (nil, []) }
            effectsPass.colorAttachments[0].texture = group.input
            effectsPass.colorAttachments[0].loadAction = .clear
            effectsPass.colorAttachments[0].storeAction = .store
            guard let encoder = commandBuffer.makeRenderCommandEncoder(descriptor: effectsPass)
            else {
                effectsPass.colorAttachments[0].texture = nil
                return (nil, [])
            }
            effectsPass.colorAttachments[0].texture = nil
            var vertex = StreamingVertexUniforms(
                scale: .init(1, 1), texCoordTransform: matrix_identity_float3x3)
            var fragment = uniforms
            encoder.setRenderPipelineState(imported.pipeline)
            encoder.setVertexBytes(
                &vertex, length: MemoryLayout<StreamingVertexUniforms>.stride, index: 0)
            encoder.setFragmentBytes(
                &fragment, length: MemoryLayout<StreamingFragmentUniforms>.stride, index: 0)
            encoder.setFragmentTexture(imported.primary, index: 0)
            encoder.setFragmentTexture(imported.secondary, index: 1)
            encoder.setFragmentTexture(imported.tertiary, index: 2)
            encoder.drawPrimitives(type: .triangleStrip, vertexStart: 0, vertexCount: 4)
            encoder.endEncoding()
            group.scaler.colorTexture = group.input
            group.scaler.outputTexture = group.output
            group.scaler.inputContentWidth = size.x
            group.scaler.inputContentHeight = size.y
            group.scaler.encode(commandBuffer: commandBuffer)
            return (
                group.output,
                [group.scaler as AnyObject, group.input as AnyObject, group.output as AnyObject]
            )
        #endif
    }

    private func normalizedRenderScale(
        for frame: StreamingDecodedFrameRef, drawableWidth: Double, drawableHeight: Double
    ) -> SIMD2<Float> {
        let rotated = frame.rotationDegrees == 90 || frame.rotationDegrees == 270
        let width = Double(rotated ? frame.height : frame.width)
        let height = Double(rotated ? frame.width : frame.height)
        guard width > 0, height > 0, drawableWidth > 0, drawableHeight > 0 else {
            return .init(1, 1)
        }
        var aspect = width / height
        let output = drawableWidth / drawableHeight
        switch contentMode {
        case .stretch: return .init(1, 1)
        case .ratio16x9: aspect = 16 / 9
        case .ratio16x10: aspect = 16 / 10
        case .ratio18x9: aspect = 18 / 9
        case .ratio21x9: aspect = 21 / 9
        case .ratio4x3: aspect = 4 / 3
        default: break
        }
        if contentMode == .fill {
            return aspect > output
                ? .init(Float(aspect / output), 1) : .init(1, Float(output / aspect))
        }
        return aspect > output ? .init(1, Float(output / aspect)) : .init(Float(aspect / output), 1)
    }

    private func textureCoordinateTransform(rotationDegrees: Int) -> simd_float3x3 {
        switch rotationDegrees {
        case 90: return .init(rows: [.init(0, -1, 1), .init(1, 0, 0), .init(0, 0, 1)])
        case 180: return .init(rows: [.init(-1, 0, 1), .init(0, -1, 1), .init(0, 0, 1)])
        case 270: return .init(rows: [.init(0, 1, 0), .init(-1, 0, 1), .init(0, 0, 1)])
        default: return matrix_identity_float3x3
        }
    }
}
