import SwiftUI
import UIKit

#if canImport(WebRTC) && canImport(MetalKit)
@preconcurrency import WebRTC
#endif

struct StreamingPlayerView: View {
    @ObservedObject var store: StreamingFeatureStore
    @State private var controlsRevealToken = 0
    @State private var controlsRevealSource: StreamingControlsRevealSource = .singleTap

    var body: some View {
        ZStack {
            Color.black.ignoresSafeArea()
            StreamingVideoSurface(
                track: store.videoTrack,
                contentMode: store.videoContentMode,
                presentationEventSink: store.handlePresentationEvent,
                controlsRevealSink: revealControls
            )
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
            if store.state == .playing {
                StreamingControlsOverlay(
                    store: store,
                    revealToken: controlsRevealToken,
                    revealSource: controlsRevealSource
                )
            } else {
                VStack {
                    HStack {
                        Spacer()
                        Button { store.stop() } label: {
                            Image(systemName: "xmark")
                                .font(.headline.bold())
                                .foregroundStyle(.white)
                                .frame(width: 44, height: 44)
                                .glassEffect(.regular.interactive(), in: Circle())
                        }
                        .accessibilityLabel("结束串流")
                    }
                    Spacer()
                }
                .padding()
            }
        }
        .statusBarHidden(true)
    }

    private func revealControls(_ source: StreamingControlsRevealSource) {
        controlsRevealSource = source
        controlsRevealToken &+= 1
        IOSRuntimeTrace.event(
            domain: "ios-streaming",
            event: "streamControlsGestureRecognized",
            payload: ["source": .string(source.rawValue)],
            dimension: .input,
            importance: .debug
        )
    }
}

#if canImport(WebRTC) && canImport(MetalKit)
private typealias StreamingVideoSurfaceView = StreamingMetalVideoSurfaceView

private struct StreamingVideoSurface: UIViewRepresentable {
    let track: StreamingVideoTrackHandle?
    let contentMode: StreamingVideoContentMode
    let presentationEventSink: @MainActor (StreamingPresentationLifecycleEvent) -> Void
    let controlsRevealSink: @MainActor (StreamingControlsRevealSource) -> Void

    func makeCoordinator() -> Coordinator { Coordinator() }

    func makeUIView(context: Context) -> StreamingVideoSurfaceView {
        let view = StreamingVideoSurfaceView()
        context.coordinator.view = view
        view.presentationEventSink = presentationEventSink
        view.updateVideoContentMode(contentMode)
        context.coordinator.installGestures(on: view, revealSink: controlsRevealSink)
        context.coordinator.update(track: track)
        return view
    }

    func updateUIView(_ uiView: StreamingVideoSurfaceView, context: Context) {
        context.coordinator.view = uiView
        uiView.presentationEventSink = presentationEventSink
        uiView.updateVideoContentMode(contentMode)
        context.coordinator.revealSink = controlsRevealSink
        context.coordinator.update(track: track)
    }

    static func dismantleUIView(_ uiView: StreamingVideoSurfaceView, coordinator: Coordinator) {
        coordinator.detach()
    }

    @MainActor
    final class Coordinator {
        weak var view: StreamingVideoSurfaceView?
        private weak var currentTrack: RTCVideoTrack?
        var revealSink: (@MainActor (StreamingControlsRevealSource) -> Void)?
        private var inwardGesture: StreamingInwardGestureRecognizer?
        private var tapGesture: UITapGestureRecognizer?

        func installGestures(
            on view: StreamingVideoSurfaceView,
            revealSink: @escaping @MainActor (StreamingControlsRevealSource) -> Void
        ) {
            self.revealSink = revealSink
            let inward = StreamingInwardGestureRecognizer(target: self, action: #selector(inward))
            inward.cancelsTouchesInView = false
            view.addGestureRecognizer(inward)
            inwardGesture = inward

            let tap = UITapGestureRecognizer(target: self, action: #selector(tapped))
            tap.cancelsTouchesInView = false
            view.addGestureRecognizer(tap)
            tapGesture = tap
        }

        @objc private func inward() {
            guard inwardGesture?.state == .recognized else { return }
            revealSink?(.twoFingerInward)
        }

        @objc private func tapped() {
            guard tapGesture?.state == .recognized else { return }
            revealSink?(.singleTap)
        }

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
                    payload: streamingPresentationTracePayload(
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
                    payload: streamingPresentationTracePayload(
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
            view?.shutdown()
            if let inwardGesture { view?.removeGestureRecognizer(inwardGesture) }
            if let tapGesture { view?.removeGestureRecognizer(tapGesture) }
            revealSink = nil
            view = nil
        }
    }
}
#else
private struct StreamingVideoSurface: View {
    let track: StreamingVideoTrackHandle?
    let contentMode: StreamingVideoContentMode
    let presentationEventSink: @MainActor (StreamingPresentationLifecycleEvent) -> Void
    let controlsRevealSink: @MainActor (StreamingControlsRevealSource) -> Void
    var body: some View { Color.black }
}
#endif
