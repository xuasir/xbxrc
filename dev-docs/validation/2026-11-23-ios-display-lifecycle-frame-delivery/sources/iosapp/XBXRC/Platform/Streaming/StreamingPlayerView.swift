import SwiftUI
import UIKit
@preconcurrency import WebRTC

struct StreamingPlayerView: View {
    let store: StreamingFeatureStore
    @State private var playerState: StreamingFeatureState
    @State private var track: StreamingVideoTrackHandle?
    @State private var picture: MobilePicturePreferences
    @State private var contentMode: StreamingVideoContentMode
    @State private var presentationActive: Bool
    @State private var startupProgress: StreamingStartupProgress?
    @EnvironmentObject private var settings: AppSettingsStore
    @Environment(\.locale) private var locale
    @State private var controlsRevealToken = 0
    @State private var controlsRevealSource: StreamingControlsRevealSource = .twoFingerInward

    init(store: StreamingFeatureStore) {
        self.store = store
        _playerState = State(initialValue: store.state)
        _track = State(initialValue: store.videoTrack)
        _picture = State(initialValue: store.picture)
        _contentMode = State(initialValue: store.videoContentMode)
        _presentationActive = State(initialValue: store.presentationActive)
        _startupProgress = State(initialValue: store.startupProgress)
    }

    var body: some View {
        player
            .onReceive(store.$state.removeDuplicates()) { playerState = $0 }
            .onReceive(store.$videoTrack.removeDuplicates { $0 === $1 }) { track = $0 }
            .onReceive(store.$picture.removeDuplicates()) { picture = $0 }
            .onReceive(store.$videoContentMode.removeDuplicates()) { contentMode = $0 }
            .onReceive(store.$presentationActive.removeDuplicates()) { presentationActive = $0 }
            .onReceive(store.$startupProgress.removeDuplicates()) { startupProgress = $0 }
    }

    private var player: some View {
        ZStack {
            Color.black.ignoresSafeArea()
            StreamingVideoSurface(
                track: track,
                contentMode: contentMode,
                picture: picture,
                active: presentationActive,
                presentationEventSink: store.handlePresentationEvent
            )
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .ignoresSafeArea()
            if playerState == .playing {
                StreamingTouchInputLayer(input: store.input)
            }
            if playerState != .playing {
                VStack(spacing: 16) {
                    if case .failed = playerState {
                        Image(systemName: "exclamationmark.triangle.fill")
                            .font(.largeTitle)
                            .foregroundStyle(.orange)
                    } else {
                        ProgressView().controlSize(.large).tint(.white)
                    }
                    Text(LocalizedStringKey(statusText))
                        .font(.headline)
                        .foregroundStyle(.white)
                        .multilineTextAlignment(.center)
                        .padding(.horizontal, 28)
                    if playerState == .preparingAccess || playerState == .creatingSession,
                        let detail = startupProgress?.localizedDetailText(locale: locale)
                    {
                        Text(detail).appFont(.caption).foregroundStyle(.white.opacity(0.8))
                            .multilineTextAlignment(.center).padding(.horizontal, 28)
                    }
                    if store.canRetry {
                        Button("重新连接") { store.retry() }
                            .buttonStyle(.borderedProminent).tint(AppThemePalette.brand)
                    }
                }
            }
            if playerState == .playing, settings.mobile.compactPerformanceOverlay {
                VStack {
                    StreamingLiveMetricsBar(store: store)
                        .padding(.horizontal, 64)
                    Spacer()
                }.allowsHitTesting(false)
            }
            if playerState == .playing {
                StreamingControlsOverlay(
                    store: store,
                    revealToken: controlsRevealToken,
                    revealSource: controlsRevealSource
                )
            } else {
                VStack {
                    HStack {
                        Spacer()
                        Button {
                            store.stop()
                        } label: {
                            Image(systemName: "xmark")
                                .font(.headline.bold())
                                .foregroundStyle(.white)
                                .frame(width: 44, height: 44)
                                .appSurface(radius: 999, prominent: true, forceDark: true)
                        }
                        .accessibilityLabel("结束串流")
                    }
                    Spacer()
                }
                .padding()
            }
        }
        .background {
            StreamingControlsGestureLayer { revealControls(.twoFingerInward) }
        }
        .statusBarHidden(true)
        .onReceive(store.input.$menuRevealToken.dropFirst()) { _ in revealControls(.accessibility) }
    }

    private var statusText: String {
        if playerState == .preparingAccess || playerState == .creatingSession, let startupProgress {
            return startupProgress.statusText
        }
        return playerState.statusText
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

private struct StreamingTouchInputLayer: View {
    @ObservedObject var input: StreamInputSession
    @EnvironmentObject private var settings: AppSettingsStore
    @State private var hasController = false
    var body: some View {
        Group {
            if settings.mobile.touchControllerMode == .always
                || (settings.mobile.touchControllerMode == .automatic && !hasController)
            {
                TouchGamepadOverlay(input: input)
            }
        }
        .task {
            while !Task.isCancelled {
                hasController = input.hasController
                try? await Task.sleep(for: .milliseconds(250))
            }
        }
    }
}

private struct StreamingVideoSurface: UIViewRepresentable {
    let track: StreamingVideoTrackHandle?
    let contentMode: StreamingVideoContentMode
    let picture: MobilePicturePreferences
    let active: Bool
    let presentationEventSink: @MainActor (StreamingPresentationLifecycleEvent) -> Void

    func makeCoordinator() -> Coordinator { Coordinator() }

    func makeUIView(context: Context) -> StreamingMetalVideoSurfaceView {
        let view = StreamingMetalVideoSurfaceView()
        context.coordinator.view = view
        context.coordinator.contentMode = contentMode
        context.coordinator.picture = picture
        view.presentationEventSink = presentationEventSink
        view.updateVideoContentMode(contentMode)
        view.updatePicture(picture)
        view.setPresentationActive(active)
        context.coordinator.update(track: track)
        return view
    }

    func updateUIView(_ uiView: StreamingMetalVideoSurfaceView, context: Context) {
        context.coordinator.view = uiView
        uiView.presentationEventSink = presentationEventSink
        if context.coordinator.contentMode != contentMode {
            context.coordinator.contentMode = contentMode
            uiView.updateVideoContentMode(contentMode)
        }
        if context.coordinator.picture != picture {
            context.coordinator.picture = picture
            uiView.updatePicture(picture)
        }
        uiView.setPresentationActive(active)
        context.coordinator.update(track: track)
    }

    static func dismantleUIView(_ uiView: StreamingMetalVideoSurfaceView, coordinator: Coordinator)
    {
        coordinator.detach()
    }

    @MainActor
    final class Coordinator {
        weak var view: StreamingMetalVideoSurfaceView?
        var contentMode: StreamingVideoContentMode?
        var picture: MobilePicturePreferences?
        private var currentTrack: RTCVideoTrack?
        private var videoGeneration: UInt64?

        func update(track handle: StreamingVideoTrackHandle?) {
            let nextTrack = handle?.rawValue
            let traceContext = handle?.traceContext
            view?.updateTraceContext(traceContext)
            view?.updateFrameFeedbackSession(handle?.frameFeedback)
            guard
                !streamingVideoTracksMatch(currentTrack, nextTrack)
                    || videoGeneration != traceContext?.videoGeneration
            else {
                return
            }
            videoGeneration = traceContext?.videoGeneration
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
