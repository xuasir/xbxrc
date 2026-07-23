import SwiftUI
import UIKit

#if canImport(WebRTC)
@preconcurrency import WebRTC
#endif

struct StreamingPlayerView: View {
    @ObservedObject var store: StreamingFeatureStore

    var body: some View {
        ZStack {
            Color.black.ignoresSafeArea()
            StreamingVideoSurface(track: store.videoTrack)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .ignoresSafeArea()
            if store.state != .playing {
                VStack(spacing: 16) {
                    if case .failed = store.state {
                        Image(systemName: "exclamationmark.triangle.fill")
                            .font(.largeTitle)
                            .foregroundStyle(.orange)
                    } else {
                        ProgressView().controlSize(.large).tint(.white)
                    }
                    Text(store.state.statusText)
                        .font(.headline)
                        .foregroundStyle(.white)
                        .multilineTextAlignment(.center)
                        .padding(.horizontal, 28)
                }
            }
            VStack {
                HStack {
                    Spacer()
                    Button { store.stop() } label: {
                        Image(systemName: "xmark")
                            .font(.headline.bold())
                            .foregroundStyle(.white)
                            .frame(width: 44, height: 44)
                            .background(.black.opacity(0.52), in: Circle())
                    }
                    .accessibilityLabel("结束串流")
                }
                Spacer()
            }
            .padding()
        }
        .statusBarHidden(true)
    }
}

#if canImport(WebRTC)
private func presentationTracePayload(
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

private struct StreamingVideoSurface: UIViewRepresentable {
    let track: StreamingVideoTrackHandle?

    func makeCoordinator() -> Coordinator { Coordinator() }

    func makeUIView(context: Context) -> StreamingVideoSurfaceView {
        let view = StreamingVideoSurfaceView()
        context.coordinator.view = view
        context.coordinator.update(track: track)
        return view
    }

    func updateUIView(_ uiView: StreamingVideoSurfaceView, context: Context) {
        context.coordinator.view = uiView
        context.coordinator.update(track: track)
    }

    static func dismantleUIView(_ uiView: StreamingVideoSurfaceView, coordinator: Coordinator) {
        coordinator.detach()
    }

    @MainActor
    final class Coordinator {
        weak var view: StreamingVideoSurfaceView?
        private weak var currentTrack: RTCVideoTrack?

        func update(track handle: StreamingVideoTrackHandle?) {
            let nextTrack = handle?.rawValue as? RTCVideoTrack
            let traceContext = handle?.traceContext
            view?.updateTraceContext(traceContext)
            guard nextTrack !== currentTrack else { return }
            if let currentTrack, let view {
                currentTrack.remove(view.renderer)
            }
            currentTrack = nextTrack
            if let nextTrack, let view {
                nextTrack.add(view.renderer)
                IOSRuntimeTrace.state(
                    domain: "ios-streaming",
                    event: "videoSurfaceAttached",
                    payload: presentationTracePayload(
                        ["renderer": .string(view.rendererName)],
                        context: traceContext
                    ),
                    dimension: .presentation,
                    importance: .key,
                    operationID: traceContext?.attemptID
                )
            } else if handle != nil {
                IOSRuntimeTrace.state(
                    domain: "ios-streaming",
                    event: "videoSurfaceTrackRejected",
                    payload: presentationTracePayload(
                        ["reason": .string("trackTypeMismatch")],
                        context: traceContext
                    ),
                    dimension: .presentation,
                    importance: .key,
                    operationID: traceContext?.attemptID
                )
            }
        }

        func detach() {
            if let currentTrack, let view { currentTrack.remove(view.renderer) }
            currentTrack = nil
            view?.updateTraceContext(nil)
            view = nil
        }
    }
}

private final class StreamingVideoSurfaceView: UIView, @preconcurrency RTCVideoViewDelegate {
    let renderer: RTCVideoRenderer
    let rendererName: String
    private var lastReportedSize: CGSize = .zero
    private var traceContext: StreamingPresentationTraceContext?

    override init(frame: CGRect) {
        let videoView = RTCMTLVideoView(frame: .zero)
        videoView.videoContentMode = .scaleAspectFit
        videoView.isEnabled = true
        rendererName = "metal"
        renderer = videoView
        super.init(frame: frame)

        backgroundColor = .black
        isOpaque = true
        videoView.backgroundColor = .black
        videoView.contentMode = .scaleAspectFit
        videoView.translatesAutoresizingMaskIntoConstraints = false
        videoView.delegate = self
        addSubview(videoView)
        NSLayoutConstraint.activate([
            videoView.leadingAnchor.constraint(equalTo: leadingAnchor),
            videoView.trailingAnchor.constraint(equalTo: trailingAnchor),
            videoView.topAnchor.constraint(equalTo: topAnchor),
            videoView.bottomAnchor.constraint(equalTo: bottomAnchor),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    override func layoutSubviews() {
        super.layoutSubviews()
        reportSurfaceSizeIfNeeded()
    }

    func updateTraceContext(_ context: StreamingPresentationTraceContext?) {
        guard context != traceContext else { return }
        traceContext = context
        lastReportedSize = .zero
        reportSurfaceSizeIfNeeded()
    }

    private func reportSurfaceSizeIfNeeded() {
        guard traceContext != nil, bounds.width > 0, bounds.height > 0,
              bounds.size != lastReportedSize
        else { return }
        lastReportedSize = bounds.size
        IOSRuntimeTrace.state(
            domain: "ios-streaming",
            event: "videoSurfaceSized",
            payload: presentationTracePayload(
                [
                    "width": .double(bounds.width),
                    "height": .double(bounds.height),
                    "renderer": .string(rendererName),
                ],
                context: traceContext
            ),
            dimension: .presentation,
            importance: .key,
            operationID: traceContext?.attemptID
        )
    }

    func videoView(_: RTCVideoRenderer, didChangeVideoSize size: CGSize) {
        guard traceContext != nil, size.width > 0, size.height > 0 else { return }
        IOSRuntimeTrace.state(
            domain: "ios-streaming",
            event: "videoSurfaceRendererReady",
            payload: presentationTracePayload(
                [
                    "frameWidth": .double(size.width),
                    "frameHeight": .double(size.height),
                    "renderer": .string(rendererName),
                ],
                context: traceContext
            ),
            dimension: .presentation,
            importance: .key,
            operationID: traceContext?.attemptID
        )
    }
}
#else
private struct StreamingVideoSurface: View {
    let track: StreamingVideoTrackHandle?
    var body: some View { Color.black }
}
#endif
