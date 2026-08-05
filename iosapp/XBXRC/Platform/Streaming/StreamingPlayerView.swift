import SwiftUI
import UIKit

#if canImport(WebRTC) && canImport(MetalKit)
@preconcurrency import WebRTC
#endif

struct StreamingPlayerView: View {
    @ObservedObject var store: StreamingFeatureStore

    var body: some View {
        ZStack {
            Color.black.ignoresSafeArea()
            StreamingVideoSurface(
                track: store.videoTrack,
                presentationEventSink: store.handlePresentationEvent
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

#if canImport(WebRTC) && canImport(MetalKit)
private typealias StreamingVideoSurfaceView = StreamingMetalVideoSurfaceView

private struct StreamingVideoSurface: UIViewRepresentable {
    let track: StreamingVideoTrackHandle?
    let presentationEventSink: @MainActor (StreamingPresentationLifecycleEvent) -> Void

    func makeCoordinator() -> Coordinator { Coordinator() }

    func makeUIView(context: Context) -> StreamingVideoSurfaceView {
        let view = StreamingVideoSurfaceView()
        context.coordinator.view = view
        view.presentationEventSink = presentationEventSink
        context.coordinator.update(track: track)
        return view
    }

    func updateUIView(_ uiView: StreamingVideoSurfaceView, context: Context) {
        context.coordinator.view = uiView
        uiView.presentationEventSink = presentationEventSink
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
            view = nil
        }
    }
}
#else
private struct StreamingVideoSurface: View {
    let track: StreamingVideoTrackHandle?
    let presentationEventSink: @MainActor (StreamingPresentationLifecycleEvent) -> Void
    var body: some View { Color.black }
}
#endif
