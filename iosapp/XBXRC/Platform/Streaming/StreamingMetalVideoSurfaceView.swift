import Foundation
@preconcurrency import Metal
import QuartzCore
import UIKit
@preconcurrency import WebRTC

private final class StreamingVideoFrameIngress: NSObject, RTCVideoRenderer, Sendable {
    private let worker: StreamingRenderWorker
    private let lease: StreamingVideoIngressLease
    init(worker: StreamingRenderWorker, lease: StreamingVideoIngressLease) {
        self.worker = worker
        self.lease = lease
    }
    func setSize(_ size: CGSize) { if lease.isValid { worker.setFrameSize(size) } }
    func renderFrame(_ frame: RTCVideoFrame?) { worker.enqueue(frame, lease: lease) }
}

/// MainActor 只配置图层与租约；drawable 与 GPU 工作由唯一 RenderQueue 执行。
@MainActor
final class StreamingMetalVideoSurfaceView: UIView {
    override class var layerClass: AnyClass { CAMetalLayer.self }
    var presentationEventSink: (@MainActor (StreamingPresentationLifecycleEvent) -> Void)? {
        didSet {
            if presentationEventSink == nil { deliveredPipeline = nil }
            if oldValue == nil, presentationEventSink != nil { worker.replayPipelineReady() }
        }
    }
    private let cache: StreamingRenderResourceCache
    private let surfaceFence = StreamingExecutionFence()
    private let fallbackPresentation: StreamingPresentationSession
    private var context: StreamingPresentationTraceContext?
    private var feedback: StreamingFrameFeedbackSession?
    private var presentation: StreamingPresentationSession
    private var contentModePreference: StreamingVideoContentMode = .fit
    private var picture = MobilePicturePreferences()
    private var presentationActive = true
    private var configurationRevision: UInt64 = 0
    private var configurationTask: Task<Void, Never>?
    private var lastPixelSize = CGSize.zero
    private var outputColor: StreamingVideoColorConfiguration?
    private var memoryObserver: NSObjectProtocol?
    private var closed = false
    private var visibilityRevision: UInt64 = 0
    private var deliveredPipeline: (context: StreamingPresentationTraceContext, revision: UInt64)?
    private lazy var worker = StreamingRenderWorker(cache: cache) {
        [weak self] event, scope, visibility in
        Task { @MainActor [weak self] in
            guard let self, !self.closed, self.presentationActive, event.context == self.context,
                visibility == self.visibilityRevision,
                scope.renderEpoch == self.presentation.snapshot?.scope.renderEpoch,
                let sink = self.presentationEventSink
            else { return }
            if case .displayPipelineReady(let context) = event {
                if self.deliveredPipeline?.context == context
                    && self.deliveredPipeline?.revision == visibility
                {
                    return
                }
                self.deliveredPipeline = (context, visibility)
            }
            sink(event)
        }
    } colorSink: { [weak self] color, scope in
        Task { @MainActor [weak self] in
            guard let self, !self.closed, self.context == scope.presentation,
                self.outputColor != color
            else { return }
            self.outputColor = color
            self.requestConfiguration()
        }
    }
    private var ingressLease = StreamingVideoIngressLease()
    private var ingress: StreamingVideoFrameIngress?
    var renderer: RTCVideoRenderer {
        if let ingress { return ingress }
        let value = StreamingVideoFrameIngress(worker: worker, lease: ingressLease)
        ingress = value
        return value
    }
    private func retireIngress() {
        ingressLease.invalidate()
        ingressLease = StreamingVideoIngressLease()
        ingress = nil
    }
    #if targetEnvironment(simulator)
        let rendererName = "metalSimulatorI420Compatibility"
    #else
        let rendererName = "metalZeroCopy"
    #endif

    convenience init(resources: StreamingRenderResourceCache = StreamingRenderResourceCache()) {
        self.init(frame: .zero, resources: resources)
    }

    convenience init(frame: CGRect, device: MTLDevice?) {
        self.init(frame: frame, resources: StreamingRenderResourceCache())
        metalLayer.device = device
    }

    init(frame: CGRect, resources: StreamingRenderResourceCache) {
        cache = resources
        let fallback = StreamingPresentationSession()
        fallbackPresentation = fallback
        presentation = fallback
        super.init(frame: frame)
        let layer = metalLayer
        layer.device = MTLCreateSystemDefaultDevice()
        layer.pixelFormat = .bgra8Unorm
        layer.framebufferOnly = false
        layer.maximumDrawableCount = 3
        layer.allowsNextDrawableTimeout = true
        layer.presentsWithTransaction = false
        backgroundColor = .black
        isOpaque = true
        memoryObserver = NotificationCenter.default.addObserver(
            forName: UIApplication.didReceiveMemoryWarningNotification, object: nil, queue: .main
        ) { [weak self] _ in
            Task { @MainActor [weak self] in self?.worker.trim() }
        }
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    isolated deinit {
        if let memoryObserver { NotificationCenter.default.removeObserver(memoryObserver) }
    }

    private var metalLayer: CAMetalLayer { layer as! CAMetalLayer }

    override func didMoveToWindow() {
        super.didMoveToWindow()
        requestConfiguration()
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        let size = pixelSize
        if size != lastPixelSize {
            lastPixelSize = size
            requestConfiguration()
        }
    }

    func updateStreamHandle(_ handle: StreamingVideoTrackHandle?) {
        let nextPresentation = handle?.presentation ?? fallbackPresentation
        guard
            context != handle?.traceContext || feedback !== handle?.frameFeedback
                || presentation !== nextPresentation
        else { return }
        retireIngress()
        context = handle?.traceContext
        feedback = handle?.frameFeedback
        presentation = nextPresentation
        outputColor = nil
        worker.updateIdentity(context: context, feedback: feedback, presentation: presentation)
        requestConfiguration()
    }

    func updateTraceContext(_ value: StreamingPresentationTraceContext?) {
        guard context != value else { return }
        retireIngress()
        context = value
        worker.updateIdentity(context: context, feedback: feedback, presentation: presentation)
        requestConfiguration()
    }

    func updateFrameFeedbackSession(_ value: StreamingFrameFeedbackSession?) {
        guard feedback !== value else { return }
        feedback = value
        worker.updateIdentity(context: context, feedback: feedback, presentation: presentation)
        requestConfiguration()
    }

    func updateVideoContentMode(_ value: StreamingVideoContentMode) {
        guard contentModePreference != value else { return }
        contentModePreference = value
        requestConfiguration()
    }

    func updatePicture(_ value: MobilePicturePreferences) {
        let normalized = value.normalized()
        guard picture != normalized else { return }
        picture = normalized
        requestConfiguration()
    }

    func setPresentationActive(_ active: Bool) {
        guard presentationActive != active else { return }
        presentationActive = active
        visibilityRevision &+= 1
        deliveredPipeline = nil
        worker.setPresentationActive(active, revision: visibilityRevision)
        presentation.setActive(active)
        requestConfiguration()
    }

    func shutdown() {
        guard !closed else { return }
        closed = true
        ingressLease.invalidate()
        surfaceFence.advance()
        configurationTask?.cancel()
        configurationTask = nil
        presentation.setActive(false)
        presentationEventSink = nil
        worker.shutdown()
        if let memoryObserver { NotificationCenter.default.removeObserver(memoryObserver) }
        memoryObserver = nil
    }

    private var pixelSize: CGSize {
        let scale = window?.screen.scale ?? traitCollection.displayScale
        return CGSize(
            width: max(1, (bounds.width * scale).rounded()),
            height: max(1, (bounds.height * scale).rounded()))
    }

    private func requestConfiguration() {
        guard !closed else { return }
        configurationRevision &+= 1
        surfaceFence.advance()
        presentation.setSurfaceReady(false)
        guard configurationTask == nil else { return }
        configurationTask = Task { @MainActor [weak self] in
            guard let self else { return }
            await self.worker.quiesce()
            guard !Task.isCancelled, !self.closed else { return }
            let revision = self.configurationRevision
            let size = self.pixelSize
            let layer = self.metalLayer
            layer.drawableSize = size
            if let color = self.outputColor { layer.colorspace = color.makeOutputColorSpace() }
            let lease = PresentationSurfaceLease(
                layer: layer, revision: revision, pixelSize: size,
                fence: self.surfaceFence, fenceRevision: self.surfaceFence.current,
                attachedToWindow: self.window != nil)
            self.worker.configure(
                surface: lease, picture: self.picture, mode: self.contentModePreference,
                active: self.presentationActive)
            self.configurationTask = nil
        }
    }

    static func supportsPictureRendering(on device: MTLDevice) -> Bool {
        StreamingRenderResourceCache.supportsPictureRendering(on: device)
    }
}
