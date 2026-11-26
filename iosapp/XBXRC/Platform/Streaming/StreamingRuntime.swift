import Foundation
import SwiftUI

@MainActor
final class StreamingFeatureStore: ObservableObject {
    @Published private(set) var state: StreamingFeatureState = .idle
    @Published private(set) var videoTrack: StreamingVideoTrackHandle?
    @Published private(set) var streamTitleID: String?
    @Published private(set) var streamTarget: StreamingLaunchTarget?
    @Published private(set) var diagnostics: StreamingSessionDiagnostics = .empty
    @Published private(set) var audioMuted = false
    @Published private(set) var microphoneEnabled = false
    @Published private(set) var microphonePending = false
    @Published private(set) var presentationActive = true
    @Published var picture = MobilePicturePreferences()
    @Published private(set) var enablesAudioControl = true
    @Published private(set) var videoContentMode: StreamingVideoContentMode = .fit
    @Published private(set) var controlMessage: String?
    @Published private(set) var startupProgress: StreamingStartupProgress?

    let input: StreamInputSession
    let touchLayout = StreamingTouchLayoutController()
    weak var authentication: AuthStore?
    private let sessionActor: StreamSessionActor
    private let settingsProvider: any StreamingSessionSettingsProviding
    private let cloudLaunchResolver: any CloudGameLaunchResolving
    private var generation: UInt64 = 0
    private var attemptID: String?
    private var launchTask: Task<Void, Never>?
    private var startupContext: XboxStreamStartup?
    private var retryLaunch: (@MainActor () -> Void)?
    private var microphoneOperationRevision: UInt64 = 0
    private var previousIdleTimerDisabled: Bool?
    private var metricsSampler = StreamingMetricsSampler()
    private var sessionStartedAtMilliseconds: Double = 0
    private var connectedElapsedMilliseconds: Double?
    private var mediaReadyElapsedMilliseconds: Double?

    init(
        settingsProvider: any StreamingSessionSettingsProviding = StaticStreamingSettingsProvider(),
        cloudLaunchResolver: any CloudGameLaunchResolving = RustCloudGameLaunchResolver()
    ) {
        self.settingsProvider = settingsProvider
        self.cloudLaunchResolver = cloudLaunchResolver
        let input = StreamInputSession {
            (settingsProvider as? AppSettingsStore)?.mobile ?? MobilePreferences()
        }
        self.input = input
        touchLayout.bind(to: input)
        sessionActor = StreamSessionActor(
            controlFactory: RustStreamingControlSessionFactory(),
            peerFactory: LibWebRTCPeerRuntimeFactory(inputSession: input)
        )
        configureTouchLayoutPreferences()
    }

    init(
        settingsProvider: any StreamingSessionSettingsProviding = StaticStreamingSettingsProvider(),
        controlFactory: any StreamingControlSessionFactory,
        peerFactory: any StreamingPeerRuntimeFactory,
        cloudLaunchResolver: any CloudGameLaunchResolving = RustCloudGameLaunchResolver(),
        remoteAnswerApplyTimeout: Duration = .seconds(10),
        initialConnectionTimeout: Duration = .seconds(15)
    ) {
        self.settingsProvider = settingsProvider
        self.cloudLaunchResolver = cloudLaunchResolver
        let input = StreamInputSession {
            (settingsProvider as? AppSettingsStore)?.mobile ?? MobilePreferences()
        }
        self.input = input
        touchLayout.bind(to: input)
        sessionActor = StreamSessionActor(
            controlFactory: controlFactory,
            peerFactory: peerFactory,
            remoteAnswerApplyTimeout: remoteAnswerApplyTimeout,
            initialConnectionTimeout: initialConnectionTimeout
        )
        configureTouchLayoutPreferences()
    }

    private func configureTouchLayoutPreferences() {
        touchLayout.restoreRecentSelection(preferences: input.preferences)
        let settings = settingsProvider as? AppSettingsStore
        touchLayout.onRecentSelectionChanged = { [weak settings] id in
            if settings?.mobile.recentCustomTouchLayoutID != id {
                settings?.mobile.recentCustomTouchLayoutID = id
            }
        }
    }

    #if DEBUG
        func prepareControlsUITestPreview(cloudGame: CloudLibraryGame? = nil) {
            let target: StreamingLaunchTarget = cloudGame == nil ? .home : .cloud
            touchLayout.beginSession(generation: 1, preferences: input.preferences, target: target)
            if let cloudGame { touchLayout.setCloudGame(cloudGame, generation: 1) }
            state = .playing
            streamTarget = target
            diagnostics = StreamingSessionDiagnostics(
            peerStats: nil, displayHealth: nil, peer: nil, audioLifecycleState: nil,
            audioRoute: nil, audioTrackReady: true, updatedAtMilliseconds: 0)
    }
    #endif

    var isPresentingPlayer: Bool { state.presentsPlayer }
    var statusText: String {
        if state == .preparingAccess || state == .creatingSession, let startupProgress {
            return startupProgress.statusText
        }
        return state.statusText
    }
    var startupDetailText: String? {
        state == .preparingAccess || state == .creatingSession ? startupProgress?.detailText : nil
    }

    func startCloudGame(
        productID: String,
        prepareAccess: @escaping @MainActor (XboxStreamStartup?) async throws -> PreparedCloudAccess
    ) {
        let region = settingsProvider.streamingSessionSettings.cloudServerRegion
        let resolutionGeneration = generation &+ 1
        start(target: .cloud, targetID: productID) { [weak self] startup in
            let access = try await prepareAccess(startup)
            do {
                guard let self, !Task.isCancelled, self.generation == resolutionGeneration else {
                    throw StreamingRuntimeError.superseded
                }
                self.startupProgress = StreamingStartupProgress(phase: "checkingGame")
                let identity = try await self.cloudLaunchResolver.resolve(
                    accessHandle: access.handle, productID: productID, serverRegion: region,
                    startup: startup)
                guard !Task.isCancelled, self.generation == resolutionGeneration else {
                    throw StreamingRuntimeError.superseded
                }
                guard identity.productID.caseInsensitiveCompare(productID) == .orderedSame,
                    !identity.streamTitleID.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                else { throw StreamingRuntimeError.cloudGameUnavailable }
                guard let entitled = identity.hasEntitlement else {
                    throw StreamingRuntimeError.cloudEntitlementUnconfirmed
                }
                guard entitled else {
                    throw StreamingRuntimeError.cloudEntitlementRequired
                }
                let layoutGame = CloudLibraryGame(
                    gameSummary: GameSummary(
                        id: identity.productID, titleID: identity.xboxTitleID ?? "",
                        name: identity.name ?? "当前游戏", artworkURL: nil, lastPlayedAt: nil,
                        playtimeMinutes: nil, achievementProgress: nil,
                        productID: identity.productID))
                self.touchLayout.setCloudGame(layoutGame, generation: resolutionGeneration)
                return PreparedStreamingAccess(
                    handle: access.handle, accountID: access.accountID,
                    regionHost: access.regionHost,
                    ownerGeneration: access.ownerGeneration, expiresAtMs: access.expiresAtMs,
                    resolvedTargetID: identity.streamTitleID)
            } catch {
                if let self {
                    await self.sessionActor.releaseUnclaimedAccess(handle: access.handle)
                } else {
                    try? releaseStreamAccess(accessHandle: access.handle)
                }
                throw error
            }
        }
        retryLaunch = { [weak self] in
            self?.startCloudGame(productID: productID, prepareAccess: prepareAccess)
        }
    }

    #if DEBUG
        // 测试夹具直达 title；生产 Cloud 入口统一经过商品身份与资格复核。
        func start(
            streamTitleID rawStreamTitleID: String,
            prepareAccess:
                @escaping @MainActor (XboxStreamStartup?) async throws -> PreparedCloudAccess
        ) {
            start(target: .cloud, targetID: rawStreamTitleID) { startup in
                let access = try await prepareAccess(startup)
                return PreparedStreamingAccess(
                handle: access.handle,
                accountID: access.accountID,
                regionHost: access.regionHost,
                ownerGeneration: access.ownerGeneration,
                expiresAtMs: access.expiresAtMs
            )
        }
        }
    #endif

    func startHome(
        targetID rawTargetID: String,
        prepareAccess: @escaping @MainActor (XboxStreamStartup?) async throws -> PreparedHomeAccess
    ) {
        start(target: .home, targetID: rawTargetID) { startup in
            let access = try await prepareAccess(startup)
            return PreparedStreamingAccess(
                handle: access.handle,
                accountID: access.accountID,
                regionHost: access.regionHost,
                ownerGeneration: access.ownerGeneration,
                expiresAtMs: access.expiresAtMs
            )
        }
    }

    private func start(
        target: StreamingLaunchTarget,
        targetID rawTargetID: String,
        prepareAccess:
            @escaping @MainActor (XboxStreamStartup?) async throws -> PreparedStreamingAccess
    ) {
        let targetID = rawTargetID.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !targetID.isEmpty else {
            state = .failed(
                message: StreamingRuntimeError.invalidStreamTitleID.localizedDescription,
                retryable: false
            )
            return
        }

        retryLaunch = { [weak self] in
            self?.start(target: target, targetID: targetID, prepareAccess: prepareAccess)
        }
        generation &+= 1
        let requestGeneration = generation
        let requestAttemptID = UUID().uuidString
        let sessionSettings = settingsProvider.streamingSessionSettings
        attemptID = requestAttemptID
        startupContext?.cancel()
        let context: XboxStreamStartup
        do { context = try createStreamStartup(targetType: target.rawValue) } catch {
            state = .failed(message: "串流启动上下文创建失败", retryable: true)
            return
        }
        startupContext = context
        launchTask?.cancel()
        streamTitleID = targetID
        streamTarget = target
        videoTrack = nil
        diagnostics = .empty
        metricsSampler = .init()
        sessionStartedAtMilliseconds = ProcessInfo.processInfo.systemUptime * 1_000
        connectedElapsedMilliseconds = nil
        mediaReadyElapsedMilliseconds = nil
        input.reset()
        touchLayout.beginSession(
            generation: requestGeneration, preferences: input.preferences, target: target)
        audioMuted = sessionSettings.startMuted
        enablesAudioControl = sessionSettings.enableAudioControl
        picture =
            (settingsProvider as? AppSettingsStore)?.mobile.picture ?? MobilePicturePreferences()
        videoContentMode = StreamingVideoContentMode(rawValue: sessionSettings.videoFit) ?? .fit
        if previousIdleTimerDisabled == nil {
            previousIdleTimerDisabled = UIApplication.shared.isIdleTimerDisabled
        }
        UIApplication.shared.isIdleTimerDisabled = sessionSettings.keepScreenAwake
        controlMessage = nil
        startupProgress = nil
        state = .preparingAccess
        traceState(state, generation: requestGeneration)
        traceAttempt(
            "streamLaunchStarted",
            attemptID: requestAttemptID,
            generation: requestGeneration,
            payload: ["target": .string(target.rawValue)],
            dimension: .lifecycle,
            importance: .essential
        )

        launchTask = Task { [weak self] in
            guard let self else { return }
            var sessionActorOwnsTerminal = false
            do {
                self.traceAttempt(
                    "accessPrepareStarted",
                    attemptID: requestAttemptID,
                    generation: requestGeneration,
                    payload: [:],
                    dimension: .lifecycle,
                    importance: .key
                )
                let access: PreparedStreamingAccess
                do {
                    access = try await prepareAccess(context)
                } catch {
                    self.traceAttempt(
                        "accessPrepareFailed",
                        attemptID: requestAttemptID,
                        generation: requestGeneration,
                        payload: ["error": .string(CloudLibraryDiagnostics.safeError(error))],
                        dimension: .lifecycle,
                        importance: .key
                    )
                    throw error
                }
                guard !Task.isCancelled, requestGeneration == self.generation else {
                    self.traceAttempt(
                        "terminalSelected",
                        attemptID: requestAttemptID,
                        generation: requestGeneration,
                        payload: [
                            "reason": .string(StreamingTerminalReason.superseded.code),
                            "retryable": .bool(false),
                        ],
                        dimension: .lifecycle,
                        importance: .essential
                    )
                    await self.sessionActor.releaseUnclaimedAccess(handle: access.handle)
                    self.traceAttempt(
                        "accessReleased",
                        attemptID: requestAttemptID,
                        generation: requestGeneration,
                        payload: [:],
                        dimension: .lifecycle,
                        importance: .key
                    )
                    return
                }
                self.traceAttempt(
                    "accessPrepareSucceeded",
                    attemptID: requestAttemptID,
                    generation: requestGeneration,
                    payload: [:],
                    dimension: .lifecycle,
                    importance: .key
                )
                sessionActorOwnsTerminal = true
                let resolvedTargetID = access.resolvedTargetID ?? targetID
                let authOwnerGeneration = self.authentication?.ownerGeneration
                self.streamTitleID = resolvedTargetID
                try await self.sessionActor.start(
                    request: StreamingLaunchRequest(
                        target: target,
                        targetID: resolvedTargetID,
                        accessHandle: access.handle,
                        accountGeneration: Self.accountGeneration(
                            accountID: access.accountID,
                            regionHost: access.regionHost
                        ),
                        sessionGeneration: requestGeneration,
                        settings: sessionSettings,
                        attemptID: requestAttemptID,
                        accountID: access.accountID,
                        ownerGeneration: access.ownerGeneration,
                        startup: context
                    ),
                    stateSink: { [weak self] generation, state in
                        self?.apply(state: state, generation: generation)
                    },
                    videoSink: { [weak self] generation, track in
                        guard let self, generation == self.generation else { return }
                        if self.videoTrack?.traceContext?.peerEpoch
                            != track?.traceContext?.peerEpoch
                        {
                            self.metricsSampler = .init()
                        }
                        self.videoTrack = track
                    },
                    diagnosticsSink: { [weak self] generation, diagnostics in
                        guard let self, generation == self.generation else { return }
                        var projected = diagnostics
                        projected.metrics = self.metricsSampler.observe(diagnostics)
                        projected.connectedElapsedMilliseconds = self.connectedElapsedMilliseconds
                        projected.mediaReadyElapsedMilliseconds = self.mediaReadyElapsedMilliseconds
                        self.diagnostics = projected
                    },
                    authSink: { [weak self] generation, update in
                        guard let self, self.generation == generation,
                            let authentication = self.authentication, let authOwnerGeneration
                        else { throw AuthStoreError.staleOperation }
                        return try await authentication.persistStreamingAuthUpdate(
                            update, expectedOwnerGeneration: authOwnerGeneration)
                    },
                    startupSink: { [weak self] generation, progress in
                        guard let self, generation == self.generation,
                            self.state == .creatingSession
                        else {
                            return
                        }
                        self.startupProgress = progress
                    },
                    microphoneSink: { [weak self] generation, enabled, pending in
                        guard let self, generation == self.generation else { return }
                        self.microphoneEnabled = enabled
                        self.microphonePending = pending
                    }
                )
            } catch is CancellationError {
                if !sessionActorOwnsTerminal {
                    self.traceAttempt(
                        "terminalSelected",
                        attemptID: requestAttemptID,
                        generation: requestGeneration,
                        payload: [
                            "reason": .string(StreamingTerminalReason.superseded.code),
                            "retryable": .bool(false),
                        ],
                        dimension: .lifecycle,
                        importance: .essential
                    )
                }
                return
            } catch StreamingRuntimeError.superseded {
                if !sessionActorOwnsTerminal {
                    self.traceAttempt(
                        "terminalSelected",
                        attemptID: requestAttemptID,
                        generation: requestGeneration,
                        payload: [
                            "reason": .string(StreamingTerminalReason.superseded.code),
                            "retryable": .bool(false),
                        ],
                        dimension: .lifecycle,
                        importance: .essential
                    )
                }
                return
            } catch {
                let wasSuperseded =
                    Task.isCancelled || requestGeneration != self.generation
                    || AuthStoreError.isCancelled(error) || AuthStoreError.isStale(error)
                let retryable = StreamingStartupDiagnostics.isRetryable(error, fallback: true)
                if !sessionActorOwnsTerminal {
                    let reason: StreamingTerminalReason =
                        wasSuperseded
                        ? .superseded
                        : .runtimeFailure
                    self.traceAttempt(
                        "terminalSelected",
                        attemptID: requestAttemptID,
                        generation: requestGeneration,
                        payload: [
                            "reason": .string(reason.code),
                            "retryable": .bool(retryable),
                        ],
                        dimension: .lifecycle,
                        importance: .essential
                    )
                }
                guard !wasSuperseded else { return }
                // 会话 actor 负责已启动会话的终态与重试属性；准备授权阶段由当前 Store 收口。
                guard !sessionActorOwnsTerminal else { return }
                self.state = .failed(
                    message: StreamingStartupDiagnostics.userMessage(
                        error,
                        fallback: CloudLibraryDiagnostics.safeError(error)),
                    retryable: retryable
                )
                self.input.setSuppressed(true, reason: .terminal)
                self.touchLayout.endSession()
                restoreIdleTimer()
                self.traceState(self.state, generation: requestGeneration)
            }
        }
    }

    var canRetry: Bool {
        if case .failed(_, let retryable) = state { return retryable && retryLaunch != nil }
        return false
    }

    func retry() {
        guard canRetry else { return }
        retryLaunch?()
    }

    func stop() {
        stop(reason: .userStop)
    }

    func stopForAuthRevocation() {
        stop(reason: .authRevoked)
    }

    var controlCapabilities: StreamingControlCapabilities {
        StreamingControlCapabilities(
            canMuteAudio: state == .playing && diagnostics.audioTrackReady,
            canSendText: state == .playing && streamTarget == .home,
            canPowerOff: state == .playing && streamTarget == .home
        )
    }

    var connectionSummary: StreamingConnectionSummary {
        diagnostics.connectionSummary(featureState: state)
    }

    func setAudioMuted(_ muted: Bool) {
        guard controlCapabilities.canMuteAudio else { return }
        let requestGeneration = generation
        traceControl("audioMute", phase: "invoked")
        Task { [weak self] in
            guard let self else { return }
            do {
                try await self.sessionActor.setAudioMuted(muted, generation: requestGeneration)
                guard requestGeneration == self.generation else { return }
                self.audioMuted = muted
                self.controlMessage = muted ? "声音已静音" : "声音已恢复"
                self.traceControl("audioMute", phase: "completed", succeeded: true)
            } catch {
                guard requestGeneration == self.generation else { return }
                self.controlMessage = "声音设置失败"
                self.traceControl("audioMute", phase: "completed", succeeded: false)
            }
        }
    }

    func sendText(_ text: String) {
        let normalized = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard controlCapabilities.canSendText, !normalized.isEmpty else { return }
        let requestGeneration = generation
        traceControl("sendText", phase: "invoked")
        Task { [weak self] in
            guard let self else { return }
            do {
                let accepted = try await self.sessionActor.sendText(
                    normalized,
                    generation: requestGeneration
                )
                guard requestGeneration == self.generation else { return }
                self.controlMessage = accepted ? "文字已发送" : "文字发送失败"
                self.traceControl("sendText", phase: "completed", succeeded: accepted)
            } catch {
                guard requestGeneration == self.generation else { return }
                self.controlMessage = "文字发送失败"
                self.traceControl("sendText", phase: "completed", succeeded: false)
            }
        }
    }

    func setMicrophoneEnabled(_ enabled: Bool) {
        guard state == .playing || microphonePending else { return }
        microphoneOperationRevision &+= 1
        let revision = microphoneOperationRevision
        let requestGeneration = generation
        microphonePending = true
        traceControl("microphone", phase: "invoked")
        Task { [weak self] in
            guard let self else { return }
            do {
                try await sessionActor.setMicrophoneEnabled(enabled, generation: requestGeneration)
                guard generation == requestGeneration, revision == microphoneOperationRevision
                else {
                    return
                }
                microphoneEnabled = enabled
                microphonePending = false
                controlMessage = enabled ? "麦克风已开启" : "麦克风已关闭"
                traceControl("microphone", phase: "completed", succeeded: true)
            } catch {
                guard generation == requestGeneration, revision == microphoneOperationRevision
                else {
                    return
                }
                microphoneEnabled = false
                microphonePending = false
                controlMessage =
                    (error as? StreamingRuntimeError)?.localizedDescription ?? "麦克风设置失败"
                traceControl("microphone", phase: "completed", succeeded: false)
            }
        }
    }

    func powerOffAndStop() {
        guard controlCapabilities.canPowerOff else { return }
        let requestGeneration = generation
        traceControl("powerOff", phase: "invoked")
        Task { [weak self] in
            guard let self else { return }
            do {
                let accepted = try await self.sessionActor.powerOffConsole(
                    generation: requestGeneration
                )
                guard requestGeneration == self.generation else { return }
                self.traceControl("powerOff", phase: "completed", succeeded: accepted)
                if accepted { self.stop() } else { self.controlMessage = "关机请求失败" }
            } catch {
                guard requestGeneration == self.generation else { return }
                self.controlMessage = "关机请求失败"
                self.traceControl("powerOff", phase: "completed", succeeded: false)
            }
        }
    }

    func toggleVideoContentMode() {
        setVideoContentMode(videoContentMode == .fit ? .fill : .fit)
    }

    func setVideoContentMode(_ mode: StreamingVideoContentMode) {
        guard videoContentMode != mode else { return }
        traceControl("videoContentMode", phase: "invoked")
        videoContentMode = mode
        controlMessage = "画面：\(videoContentMode.title)"
        traceControl("videoContentMode", phase: "completed", succeeded: true)
    }

    func clearControlMessage(ifMatching message: String) {
        if controlMessage == message { controlMessage = nil }
    }

    private func stop(reason: StreamingTerminalReason) {
        guard state != .idle else { return }
        let sessionGeneration = generation
        videoTrack = nil
        input.setSuppressed(true, reason: .terminal)
        touchLayout.endSession()
        microphoneOperationRevision &+= 1
        microphoneEnabled = false
        microphonePending = false
        restoreIdleTimer()
        generation &+= 1
        let stoppedGeneration = generation
        startupContext?.cancel()
        startupContext = nil
        launchTask?.cancel()
        launchTask = nil
        retryLaunch = nil
        startupProgress = nil
        state = .stopping
        traceState(state, generation: sessionGeneration)
        Task { [weak self] in
            guard let self else { return }
            await self.sessionActor.stop(generation: sessionGeneration, reason: reason)
            guard stoppedGeneration == self.generation else { return }
            self.videoTrack = nil
            self.diagnostics = .empty
            self.audioMuted = false
            self.controlMessage = nil
            self.streamTitleID = nil
            self.streamTarget = nil
            self.state = .idle
            self.attemptID = nil
            self.traceState(.idle, generation: stoppedGeneration)
        }
    }

    func handleScenePhase(_ phase: ScenePhase) {
        input.setSuppressed(phase != .active, reason: .lifecycle)
        presentationActive = phase == .active
        guard state != .idle, state != .stopping else { return }
        let requestGeneration = generation
        Task { [sessionActor] in
            await sessionActor.setVisible(phase == .active, generation: requestGeneration)
        }
    }

    func handlePresentationEvent(_ event: StreamingPresentationLifecycleEvent) {
        let context = event.context
        guard presentationActive, context.generation == generation,
            context.attemptID == attemptID,
            videoTrack?.traceContext == context
        else { return }
        Task { [sessionActor] in
            switch event {
            case .displayPipelineReady(let context):
                await sessionActor.displayPipelineReady(context: context)
            case .firstFrameAccepted(let context):
                await sessionActor.firstFrameAcceptedToMailbox(context: context)
            case .firstFrameSubmitted(let context):
                await sessionActor.firstFrameSubmitted(context: context)
            case .firstFramePresented(let context):
                await sessionActor.firstFramePresented(context: context)
            case .displayHealth(let context, let snapshot):
                await sessionActor.displayHealthUpdated(context: context, snapshot: snapshot)
            }
        }
    }

    private func restoreIdleTimer() {
        if let previousIdleTimerDisabled {
            UIApplication.shared.isIdleTimerDisabled = previousIdleTimerDisabled
        }
        previousIdleTimerDisabled = nil
    }

    private func apply(state: StreamingFeatureState, generation: UInt64) {
        guard generation == self.generation else { return }
        if case .failed = self.state { return }
        guard self.state != state else { return }
        self.state = state
        let elapsed = ProcessInfo.processInfo.systemUptime * 1_000 - sessionStartedAtMilliseconds
        if state == .waitingForFirstFrame, connectedElapsedMilliseconds == nil {
            connectedElapsedMilliseconds = elapsed
            diagnostics.connectedElapsedMilliseconds = elapsed
        }
        if state == .playing, mediaReadyElapsedMilliseconds == nil {
            mediaReadyElapsedMilliseconds = elapsed
            diagnostics.mediaReadyElapsedMilliseconds = elapsed
        }
        if case .failed = state {
            microphoneEnabled = false
            microphonePending = false
            input.setSuppressed(true, reason: .terminal)
            touchLayout.endSession()
            restoreIdleTimer()
        }
        traceState(state, generation: generation)
    }

    private func traceState(_ state: StreamingFeatureState, generation: UInt64) {
        IOSRuntimeTrace.state(
            domain: "ios-streaming",
            event: "streamingStateChanged",
            payload: [
                "generation": .integer(Int64(generation)),
                "attemptId": .string(attemptID ?? "none"),
                "state": .string(state.traceCode),
                "retryable": state.traceRetryable.map(IOSRuntimeTraceValue.bool) ?? .null,
                "target": .string(streamTarget?.rawValue ?? "none"),
                "hasVideoTrack": .bool(videoTrack != nil),
            ],
            dimension: .lifecycle,
            importance: .key
        )
    }

    private func traceControl(_ action: String, phase: String, succeeded: Bool? = nil) {
        var payload: [String: IOSRuntimeTraceValue] = [
            "action": .string(action),
            "phase": .string(phase),
            "generation": .integer(Int64(clamping: generation)),
            "target": .string(streamTarget?.rawValue ?? "none"),
        ]
        if let succeeded { payload["succeeded"] = .bool(succeeded) }
        IOSRuntimeTrace.event(
            domain: "ios-streaming",
            event: phase == "invoked" ? "streamControlInvoked" : "streamControlCompleted",
            payload: payload,
            dimension: .input,
            importance: .debug,
            operationID: attemptID
        )
    }

    private func traceAttempt(
        _ event: String,
        attemptID: String,
        generation: UInt64,
        payload: [String: IOSRuntimeTraceValue],
        dimension: IOSRuntimeTraceDimension,
        importance: IOSRuntimeTraceImportance
    ) {
        var contextualPayload = payload
        contextualPayload["attemptId"] = .string(attemptID)
        contextualPayload["generation"] = .integer(Int64(generation))
        IOSRuntimeTrace.state(
            domain: "ios-streaming",
            event: event,
            payload: contextualPayload,
            dimension: dimension,
            importance: importance,
            operationID: attemptID
        )
    }

    private static func accountGeneration(accountID: String, regionHost: String) -> UInt64 {
        var hasher = Hasher()
        hasher.combine(accountID)
        hasher.combine(regionHost)
        return UInt64(bitPattern: Int64(hasher.finalize()))
    }
}

private struct PreparedStreamingAccess: Sendable {
    let handle: String
    let accountID: String
    let regionHost: String
    let ownerGeneration: UInt64
    let expiresAtMs: UInt64
    var resolvedTargetID: String? = nil
}

@MainActor
private final class StaticStreamingSettingsProvider: StreamingSessionSettingsProviding {
    var streamingSessionSettings: StreamingSessionSettingsSnapshot { .standard }
}

struct RustStreamingControlSessionFactory: StreamingControlSessionFactory {
    func createSession(request: StreamingLaunchRequest) async throws -> any StreamingControlSession
    {
        let session = try createScopedStreamSession(
            accessHandle: request.accessHandle,
            targetType: request.target == .cloud ? "cloud" : "home",
            targetId: request.targetID,
            accountId: request.accountID,
            ownerGeneration: request.ownerGeneration,
            settings: XboxStreamSettings(
                cloudServerRegion: request.settings.cloudServerRegion,
                preferredGameLocale: request.settings.preferredGameLocale,
                cloudResolution: Int64(request.settings.cloudResolution),
                homeResolution: Int64(request.settings.homeResolution),
                preferIpv6: request.settings.preferIPv6,
                videoCodec: request.settings.videoCodec,
                homeBitrateMode: request.settings.homeBitrateMode,
                homeBitrateMbps: Int64(request.settings.homeBitrateMbps),
                cloudBitrateMode: request.settings.cloudBitrateMode,
                cloudBitrateMbps: Int64(request.settings.cloudBitrateMbps),
                audioBitrateMode: request.settings.audioBitrateMode,
                audioBitrateKbps: Int64(request.settings.audioBitrateKbps),
                homeTurnFallback: request.settings.homeTurnFallback,
                customTurn: request.settings.customTurn.map {
                    XboxCustomTurn(
                        serverUrl: $0.serverURL, username: $0.username, credential: $0.credential)
                },
                preventResolutionDrops: request.settings.preventResolutionDrops,
                powerOn: request.settings.powerOn
            ),
            startup: request.startup
        )
        return RustStreamingControlSession(session: session)
    }

    func releaseAccess(handle: String) async {
        try? releaseStreamAccess(accessHandle: handle)
    }
}

private actor RustStreamingControlSession: StreamingControlSession {
    private let session: XboxStreamSession
    private var rustGeneration: UInt64?

    init(session: XboxStreamSession) {
        self.session = session
    }

    func pendingAuthUpdate() async -> XboxStreamingAuthUpdate? {
        await session.pendingAuthUpdate()
    }

    func acknowledgeAuthUpdate(revision: UInt64) async {
        await session.acknowledgeAuthUpdate(revision: revision)
    }

    func synchronizeAuthSession(revision: UInt64, session authSession: AuthSession) async throws {
        try await session.synchronizeAuthSession(revision: revision, authSession: authSession)
    }

    func prepareSignaling() async throws -> StreamingPreparedSignaling {
        let snapshot = try await session.start()
        rustGeneration = snapshot.generation
        return StreamingPreparedSignaling(
            iceServers: snapshot.iceServers.map {
                StreamingIceServer(
                    urls: $0.urls,
                    username: $0.username,
                    credential: $0.credential
                )
            },
            webRtcPlan: StreamingWebRtcPlan(
                audioDirection: snapshot.webRtcPlan.audioDirection == "sendrecv"
                    ? .sendReceive : .receiveOnly,
                videoDirection: snapshot.webRtcPlan.videoDirection == "sendrecv"
                    ? .sendReceive : .receiveOnly,
                videoCodecMimeType: snapshot.webRtcPlan.videoCodecMimeType,
                targetVideoWidth: Int(snapshot.webRtcPlan.targetVideoWidth),
                targetVideoHeight: Int(snapshot.webRtcPlan.targetVideoHeight),
                audioBitrateKbps: snapshot.webRtcPlan.audioBitrateKbps.map(Int.init),
                h264Profiles: snapshot.webRtcPlan.h264Profiles,
                h264PacketizationMode: Int(snapshot.webRtcPlan.h264PacketizationMode),
                h264LevelAsymmetryAllowed: snapshot.webRtcPlan.h264LevelAsymmetryAllowed,
                maxFrameSize: Int(snapshot.webRtcPlan.maxFrameSize),
                maxFrameRate: Int(snapshot.webRtcPlan.maxFrameRate),
                minVideoBitrateKbps: snapshot.webRtcPlan.minVideoBitrateKbps.map(Int.init),
                startVideoBitrateKbps: snapshot.webRtcPlan.startVideoBitrateKbps.map(Int.init),
                maxVideoBitrateKbps: snapshot.webRtcPlan.maxVideoBitrateKbps.map(Int.init),
                stereoAudio: snapshot.webRtcPlan.stereoAudio,
                requiredVideoRtcpFeedback: snapshot.webRtcPlan.requiredVideoRtcpFeedback,
                allowedCandidateTypes: snapshot.webRtcPlan.allowedCandidateTypes,
                iceTransportPolicy: snapshot.webRtcPlan.iceTransportPolicy,
                preferIPv6: snapshot.webRtcPlan.preferIpv6,
                normalizeEndOfCandidates: snapshot.webRtcPlan.normalizeEndOfCandidates,
                preventResolutionDrops: snapshot.webRtcPlan.preventResolutionDrops
            ),
            fallbackIceServers: snapshot.fallbackIceServers?.map {
                StreamingIceServer(urls: $0.urls, username: $0.username, credential: $0.credential)
            }
        )
    }

    func startupProgress() async -> StreamingStartupProgress? {
        session.startupProgress().map { value in
            StreamingStartupProgress(
                phase: value.phase, status: value.status, streamState: value.streamState,
                totalWaitSeconds: value.totalWaitSeconds,
                allocationWaitSeconds: value.allocationWaitSeconds,
                provisioningWaitSeconds: value.provisioningWaitSeconds,
                retryCount: value.retryCount,
                retryLimit: value.retryLimit)
        }
    }

    func exchangeMediaOffer(_ sdp: String, kind: String) async throws -> String {
        try await session.exchangeMediaOffer(generation: requireGeneration(), sdp: sdp, kind: kind)
    }

    func supervise(_ sample: StreamingSupervisionSample) async throws
        -> StreamingSupervisionDecision
    {
        let data = try JSONEncoder().encode(sample)
        let result = try await session.superviseRuntime(
            generation: requireGeneration(), sampleJson: String(decoding: data, as: UTF8.self))
        return try JSONDecoder().decode(StreamingSupervisionDecision.self, from: Data(result.utf8))
    }

    func validateResume() async throws {
        try await session.validateResume(generation: requireGeneration())
    }

    func exchangeOffer(_ sdp: String) async throws -> String {
        let operationID = UUID().uuidString
        let startedAt = Date()
        IOSRuntimeTrace.event(
            domain: "ios-streaming",
            event: "signalingOfferStarted",
            payload: ["sdpBytes": .integer(Int64(sdp.utf8.count))],
            dimension: .network,
            importance: .key,
            operationID: operationID
        )
        do {
            let answer = try await session.exchangeOffer(generation: requireGeneration(), sdp: sdp)
            IOSRuntimeTrace.event(
                domain: "ios-streaming",
                event: "signalingOfferSucceeded",
                payload: [
                    "elapsedMs": .integer(Int64(Date().timeIntervalSince(startedAt) * 1_000)),
                    "answerBytes": .integer(Int64(answer.utf8.count)),
                ],
                dimension: .network,
                importance: .key,
                operationID: operationID
            )
            return answer
        } catch {
            IOSRuntimeTrace.event(
                domain: "ios-streaming",
                event: "signalingOfferFailed",
                payload: [
                    "elapsedMs": .integer(Int64(Date().timeIntervalSince(startedAt) * 1_000)),
                    "error": .string(CloudLibraryDiagnostics.safeError(error)),
                ],
                dimension: .network,
                importance: .key,
                operationID: operationID
            )
            throw error
        }
    }

    func submitLocalCandidates(_ candidates: [StreamingIceCandidate]) async throws {
        guard !candidates.isEmpty else { return }
        let generation = try requireGeneration()
        let operationID = UUID().uuidString
        let startedAt = Date()
        IOSRuntimeTrace.event(
            domain: "ios-streaming",
            event: "signalingIceBatchStarted",
            payload: ["candidateCount": .integer(Int64(candidates.count))],
            dimension: .network,
            importance: .key,
            operationID: operationID
        )
        do {
            try await session.submitIce(
                generation: generation,
                candidates: candidates.map { candidate in
                    XboxIceCandidate(
                        source: candidate.source,
                        candidate: candidate.sdp,
                        sdpMLineIndex: UInt32(exactly: candidate.sdpMLineIndex),
                        sdpMid: candidate.sdpMid,
                        usernameFragment: candidate.usernameFragment,
                        messageType: nil
                    )
                }
            )
            IOSRuntimeTrace.event(
                domain: "ios-streaming",
                event: "signalingIceBatchSucceeded",
                payload: [
                    "candidateCount": .integer(Int64(candidates.count)),
                    "elapsedMs": .integer(Int64(Date().timeIntervalSince(startedAt) * 1_000)),
                ],
                dimension: .network,
                importance: .key,
                operationID: operationID
            )
        } catch {
            IOSRuntimeTrace.event(
                domain: "ios-streaming",
                event: "signalingIceBatchFailed",
                payload: [
                    "candidateCount": .integer(Int64(candidates.count)),
                    "elapsedMs": .integer(Int64(Date().timeIntervalSince(startedAt) * 1_000)),
                    "error": .string(CloudLibraryDiagnostics.safeError(error)),
                ],
                dimension: .network,
                importance: .key,
                operationID: operationID
            )
            throw error
        }
    }

    func completeLocalIceGathering() async throws {
        let generation = try requireGeneration()
        let operationID = UUID().uuidString
        let startedAt = Date()
        IOSRuntimeTrace.event(
            domain: "ios-streaming",
            event: "signalingIceCompletionStarted",
            payload: [:],
            dimension: .network,
            importance: .key,
            operationID: operationID
        )
        do {
            try await session.submitIce(generation: generation, candidates: [])
            IOSRuntimeTrace.event(
                domain: "ios-streaming",
                event: "signalingIceCompletionSucceeded",
                payload: [
                    "elapsedMs": .integer(Int64(Date().timeIntervalSince(startedAt) * 1_000))
                ],
                dimension: .network,
                importance: .key,
                operationID: operationID
            )
        } catch {
            IOSRuntimeTrace.event(
                domain: "ios-streaming",
                event: "signalingIceCompletionFailed",
                payload: [
                    "elapsedMs": .integer(Int64(Date().timeIntervalSince(startedAt) * 1_000)),
                    "error": .string(CloudLibraryDiagnostics.safeError(error)),
                ],
                dimension: .network,
                importance: .key,
                operationID: operationID
            )
            throw error
        }
    }

    func nextRemoteIceBatch() async throws -> StreamingRemoteIceBatch {
        let batch = try await session.nextRemoteIceBatch(generation: requireGeneration())
        return StreamingRemoteIceBatch(
            candidates: batch.candidates.map {
                StreamingIceCandidate(
                    sdp: $0.candidate,
                    sdpMid: $0.sdpMid,
                    sdpMLineIndex: Int32(clamping: $0.sdpMLineIndex ?? 0),
                    usernameFragment: $0.usernameFragment,
                    source: $0.source
                )
            },
            endOfCandidates: batch.endOfCandidates,
            configurationPending: batch.configurationPending
        )
    }

    func markConnected() async throws {
        try await session.markConnected(generation: requireGeneration())
    }

    func sendText(_ text: String) async throws -> Bool {
        try await session.sendText(generation: requireGeneration(), text: text)
    }

    func powerOffConsole() async throws -> Bool {
        try await session.powerOffConsole(generation: requireGeneration())
    }

    func close() async {
        _ = await session.cancel()
        try? await session.close()
        rustGeneration = nil
    }

    private func requireGeneration() throws -> UInt64 {
        guard let rustGeneration else {
            throw StreamingRuntimeError.superseded
        }
        return rustGeneration
    }
}
