import Foundation

extension Duration {
    fileprivate var milliseconds: Int64 {
        Int64(components.seconds) * 1_000
            + Int64(components.attoseconds / 1_000_000_000_000_000)
    }
}

final class RemoteAnswerApplyGate: @unchecked Sendable {
    private let lock = NSLock()
    private var continuation: CheckedContinuation<Void, Error>?
    private var operationTask: Task<Void, Never>?
    private var timeoutTask: Task<Void, Never>?
    private var resolved = false

    func install(_ continuation: CheckedContinuation<Void, Error>) {
        lock.lock()
        if resolved {
            lock.unlock()
            continuation.resume(throwing: CancellationError())
            return
        }
        self.continuation = continuation
        lock.unlock()
    }

    func installTasks(
        operationTask: Task<Void, Never>,
        timeoutTask: Task<Void, Never>
    ) {
        lock.lock()
        if resolved {
            lock.unlock()
            operationTask.cancel()
            timeoutTask.cancel()
            return
        }
        self.operationTask = operationTask
        self.timeoutTask = timeoutTask
        lock.unlock()
    }

    @discardableResult
    func resolve(_ result: Result<Void, Error>) -> Bool {
        lock.lock()
        guard !resolved else {
            lock.unlock()
            return false
        }
        resolved = true
        let continuation = continuation
        let operationTask = operationTask
        let timeoutTask = timeoutTask
        self.continuation = nil
        self.operationTask = nil
        self.timeoutTask = nil
        lock.unlock()

        operationTask?.cancel()
        timeoutTask?.cancel()
        continuation?.resume(with: result)
        return true
    }
}

actor StreamSessionActor {
    typealias StateSink = @MainActor @Sendable (UInt64, StreamingFeatureState) -> Void
    typealias VideoSink = @MainActor @Sendable (UInt64, StreamingVideoTrackHandle?) -> Void
    typealias DiagnosticsSink = @MainActor @Sendable (UInt64, StreamingSessionDiagnostics) -> Void
    typealias StartupSink = @MainActor @Sendable (UInt64, StreamingStartupProgress?) -> Void
    typealias MicrophoneSink = @MainActor @Sendable (UInt64, Bool, Bool) -> Void
    typealias AuthSink =
        @MainActor @Sendable (UInt64, XboxStreamingAuthUpdate) async throws ->
        StreamingAuthCommitResult

    private struct LocalIceEpoch: Equatable, Sendable {
        let peerEpoch: UInt64
        let negotiationEpoch: UInt64
    }

    private enum LocalIceFlushError: Error {
        case negotiationSuperseded
    }

    private let controlFactory: any StreamingControlSessionFactory
    private let peerFactory: any StreamingPeerRuntimeFactory
    private let remoteAnswerApplyTimeout: Duration
    private var activeGeneration: UInt64?
    private var activeAttemptID: String?
    private var activeAccessHandle: String?
    private var controlSession: (any StreamingControlSession)?
    private var peerRuntime: (any StreamingPeerRuntime)?
    private var retiringPeerRuntime: (any StreamingPeerRuntime)?
    private var retiringPeerStopTask: Task<Void, Never>?
    private var preparedSignaling: StreamingPreparedSignaling?
    private var latestStartupProgress: StreamingStartupProgress?
    private var remoteIceApplicationTask: Task<Void, Never>?
    private var credentialUpdateTask: Task<Void, Never>?
    private var localIceFlushTask: Task<Void, Error>?
    private var activeLocalIceFlushTaskID: UInt64 = 0
    private var nextLocalIceFlushTaskID: UInt64 = 0
    private var latestRequestedGeneration: UInt64 = 0
    private var stateSink: StateSink?
    private var lastPublishedState: StreamingFeatureState?
    private var videoSink: VideoSink?
    private var diagnosticsSink: DiagnosticsSink?
    private var microphoneSink: MicrophoneSink?
    private var playbackReadiness = StreamingPlaybackReadiness()
    private var localIceState = StreamingLocalIceState()
    private var reconnectInFlight = false
    private var initialNegotiationInFlight = false
    private var selectedTerminalReason: StreamingTerminalReason?
    private var pendingLocalCandidates: [StreamingIceCandidate] = []
    private var interactionEventCounts: [String: Int64] = [:]
    private var dataChannelProfilesPendingTrace = false
    private var activePeerEpoch: UInt64 = 0
    private var nextPeerEpoch: UInt64 = 0
    private var activeLocalIceEpoch: LocalIceEpoch?
    private var nextNegotiationEpoch: UInt64 = 0
    private var pendingConnectedPeerEpoch: UInt64?
    private var pendingDisconnectedPeerEpoch: UInt64?
    private var latestDisplayHealthSnapshot: StreamingDisplayHealthSnapshot?
    private var latestPeerStats: StreamingPeerStats?
    private var latestAudioLifecycleState: String?
    private var latestAudioRoute: String?
    private var latestAudioOutput: StreamingAudioOutputSnapshot?
    private var audioTrackReady = false
    private var requestedAudioMuted = false
    private var microphonePeerEpoch: UInt64 = 0
    private var visible = true
    private var visibilityRevision: UInt64 = 0
    private var negotiationOwner: UUID?
    private var microphoneRevision: UInt64 = 0
    private var microphoneTask: Task<Void, Error>?
    private var microphoneRequested = false
    private var resumedAtMilliseconds: Double?
    private var supervisionTask: Task<Void, Never>?
    private var firstFrameGuardTask: Task<Void, Never>?
    private var supervisionConnectedAtMs: UInt64 = 0
    private var supervisionEligibleSinceMs: UInt64 = 0
    private var lastDecodeActivityAtMs: UInt64 = 0
    private var displayResetTask: Task<Void, Never>?
    private var displayResetDeadlineTask: Task<Void, Never>?
    private var displayResetActionID: UInt64?
    private var displayResetScope: StreamExecutionScope?
    private var displayRecoveryCompletion: StreamingDisplayRecoveryCompletion?
    private var lastLocalActionAtMs: UInt64?
    private var recoveryProbe: StreamingRecoveryProbe?
    private var keyframeBudget = 2
    private var playbackBudget = 1
    private var videoGeneration: UInt64 = 0
    private var rtcTraceWindow = StreamingTraceCounterWindow()
    private var displayTraceWindow = StreamingTraceCounterWindow()
    private var currentVideoHandle: StreamingVideoTrackHandle?
    private var qualityLevel = 0
    private var qualityCompletion: StreamingQualityCompletion?
    private var qualityOperationInFlight = false
    private var latestDecision: StreamingSupervisionDecision = .unavailable
    private var connectionRecoveryDeadlineTask: Task<Void, Never>?
    private var connectionRecoveryAwaitingFrames = false
    private var peerConnectionState: StreamingPeerConnectionState = .new
    private var hasEverConnected = false
    private var fallbackRetryConsumed = false
    private var initialConnectionTask: Task<Void, Never>?
    private let initialConnectionTimeout: Duration

    init(
        controlFactory: any StreamingControlSessionFactory,
        peerFactory: any StreamingPeerRuntimeFactory,
        remoteAnswerApplyTimeout: Duration = .seconds(10),
        initialConnectionTimeout: Duration = .seconds(15)
    ) {
        self.controlFactory = controlFactory
        self.peerFactory = peerFactory
        self.remoteAnswerApplyTimeout = remoteAnswerApplyTimeout
        self.initialConnectionTimeout = initialConnectionTimeout
    }

    func start(
        request: StreamingLaunchRequest,
        stateSink: @escaping StateSink,
        videoSink: @escaping VideoSink,
        diagnosticsSink: @escaping DiagnosticsSink,
        authSink: @escaping AuthSink = { _, _ in throw AuthStoreError.staleOperation },
        startupSink: @escaping StartupSink = { _, _ in },
        microphoneSink: @escaping MicrophoneSink = { _, _, _ in }
    ) async throws {
        guard request.sessionGeneration > latestRequestedGeneration else {
            traceAttempt(
                "terminalSelected",
                attemptID: request.attemptID,
                generation: request.sessionGeneration,
                payload: [
                    "reason": .string(StreamingTerminalReason.superseded.code),
                    "retryable": .bool(false),
                ],
                dimension: .lifecycle,
                importance: .essential,
                operationID: request.attemptID
            )
            await controlFactory.releaseAccess(handle: request.accessHandle)
            traceCleanup(
                "accessReleased",
                attemptID: request.attemptID,
                generation: request.sessionGeneration
            )
            throw StreamingRuntimeError.superseded
        }
        latestRequestedGeneration = request.sessionGeneration
        await stopActiveSession(reason: .superseded, publishIdle: false)
        guard latestRequestedGeneration == request.sessionGeneration else {
            traceAttempt(
                "terminalSelected",
                attemptID: request.attemptID,
                generation: request.sessionGeneration,
                payload: [
                    "reason": .string(StreamingTerminalReason.superseded.code),
                    "retryable": .bool(false),
                ],
                dimension: .lifecycle,
                importance: .essential,
                operationID: request.attemptID
            )
            await controlFactory.releaseAccess(handle: request.accessHandle)
            traceCleanup(
                "accessReleased",
                attemptID: request.attemptID,
                generation: request.sessionGeneration
            )
            throw StreamingRuntimeError.superseded
        }
        activeGeneration = request.sessionGeneration
        activeAttemptID = request.attemptID
        activeAccessHandle = request.accessHandle
        initialNegotiationInFlight = true
        self.microphoneSink = microphoneSink
        requestedAudioMuted = request.settings.startMuted
        self.stateSink = stateSink
        lastPublishedState = nil
        self.videoSink = videoSink
        self.diagnosticsSink = diagnosticsSink
        playbackReadiness.resetForPeer()
        localIceState.reset()
        reconnectInFlight = false
        selectedTerminalReason = nil
        latestStartupProgress = nil
        pendingLocalCandidates.removeAll(keepingCapacity: true)
        interactionEventCounts.removeAll(keepingCapacity: true)
        dataChannelProfilesPendingTrace = false
        activePeerEpoch = 0
        activeLocalIceEpoch = nil
        pendingConnectedPeerEpoch = nil
        pendingDisconnectedPeerEpoch = nil
        latestDisplayHealthSnapshot = nil
        latestPeerStats = nil
        rtcTraceWindow = StreamingTraceCounterWindow()
        displayTraceWindow = StreamingTraceCounterWindow()
        latestAudioLifecycleState = nil
        latestAudioRoute = nil
        latestAudioOutput = nil
        audioTrackReady = false

        await publish(.creatingSession, generation: request.sessionGeneration)
        trace(
            "sessionCreateStarted",
            payload: [
                "powerOnRequested": .bool(request.settings.powerOn),
                "target": .string(request.target.rawValue),
            ],
            dimension: .lifecycle
        )
        do {
            let control = try await controlFactory.createSession(request: request)
            do {
                try ensureCurrent(request.sessionGeneration)
            } catch {
                await control.close()
                throw error
            }
            controlSession = control
            trace("controlSessionCreated", dimension: .lifecycle)
            startCredentialUpdates(
                control: control, generation: request.sessionGeneration, sink: authSink)
            trace("signalingPreparationStarted", dimension: .lifecycle)
            let prepared = try await prepareSignaling(
                control: control, generation: request.sessionGeneration, startupSink: startupSink)
            preparedSignaling = prepared
            try ensureCurrent(request.sessionGeneration)
            trace("signalingPreparationSucceeded", dimension: .lifecycle)
            trace("sessionReady", dimension: .lifecycle)

            await publish(.negotiating, generation: request.sessionGeneration)
            let peer = try await makePeerRuntime(generation: request.sessionGeneration)
            guard
                let localIceEpoch = await beginLocalIceNegotiation(
                    generation: request.sessionGeneration,
                    peerEpoch: activePeerEpoch
                )
            else {
                throw StreamingRuntimeError.superseded
            }
            trace("offerStarted", dimension: .network)
            let offer = try await peer.makeOffer(configuration: prepared, iceRestart: false)
            try ensureCurrent(request.sessionGeneration)
            let answer = try await control.exchangeOffer(offer)
            try ensureCurrent(request.sessionGeneration)
            guard !answer.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
                throw StreamingRuntimeError.missingRemoteDescription
            }
            try await applyRemoteAnswer(answer, to: peer, phase: "initial")
            try ensureCurrent(request.sessionGeneration)
            trace("answerApplied", dimension: .network)
            trace("localIceStarted", dimension: .network)
            localIceState.remoteDescriptionApplied = true
            try await flushLocalIceAndWait(
                generation: request.sessionGeneration,
                epoch: localIceEpoch
            )
            try ensureCurrent(request.sessionGeneration)
            await publish(connectionStateAfterNegotiation, generation: request.sessionGeneration)
            await completePendingPeerConnection(
                generation: request.sessionGeneration,
                peerEpoch: activePeerEpoch
            )
            initialNegotiationInFlight = false
            startRemoteIceApplication(
                generation: request.sessionGeneration,
                peerEpoch: activePeerEpoch
            )
            await completePendingPeerDisconnection(
                generation: request.sessionGeneration,
                peerEpoch: activePeerEpoch
            )
            armInitialConnectionDeadline(generation: request.sessionGeneration)
        } catch LocalIceFlushError.negotiationSuperseded {
            return
        } catch {
            if isCurrent(request.sessionGeneration) {
                if peerRuntime != nil, !localIceState.remoteDescriptionApplied {
                    trace(
                        "signalingTerminal",
                        payload: ["reason": .string(StreamingTerminalReason.sessionFailed.code)],
                        dimension: .network,
                        importance: .key
                    )
                }
                await fail(
                    reason: .sessionFailed, generation: request.sessionGeneration, error: error)
            }
            throw error
        }
    }

    func releaseUnclaimedAccess(handle: String) async {
        await controlFactory.releaseAccess(handle: handle)
    }

    private func prepareSignaling(
        control: any StreamingControlSession, generation: UInt64,
        startupSink: @escaping StartupSink
    ) async throws -> StreamingPreparedSignaling {
        let progressTask = Task { [weak self] in
            while !Task.isCancelled {
                guard let self, await self.isCurrent(generation) else { return }
                let progress = await control.startupProgress()
                guard !Task.isCancelled, await self.isCurrent(generation) else { return }
                await self.recordStartupProgress(progress, generation: generation)
                await startupSink(generation, progress)
                do { try await Task.sleep(for: .milliseconds(300)) } catch { return }
            }
        }
        defer { progressTask.cancel() }
        return try await control.prepareSignaling()
    }

    private func recordStartupProgress(
        _ progress: StreamingStartupProgress?, generation: UInt64
    ) {
        guard isCurrent(generation), let progress, progress != latestStartupProgress else { return }
        latestStartupProgress = progress
        trace(
            "startupProgressChanged",
            payload: StreamingStartupDiagnostics.progressPayload(progress),
            dimension: .lifecycle,
            importance: .key
        )
    }

    func stop(
        generation: UInt64? = nil,
        reason: StreamingTerminalReason = .userStop
    ) async {
        if let generation, generation != activeGeneration { return }
        await stopActiveSession(reason: reason, publishIdle: true)
    }

    func setAudioMuted(_ muted: Bool, generation: UInt64) async throws {
        guard isCurrent(generation), let peerRuntime else {
            throw StreamingRuntimeError.superseded
        }
        try await peerRuntime.setAudioMuted(muted)
        try ensureCurrent(generation)
        requestedAudioMuted = muted
        trace(
            muted ? "remoteAudioMuted" : "remoteAudioUnmuted",
            dimension: .mediaSupply
        )
    }

    private func acquireNegotiation(generation: UInt64) async throws -> UUID {
        while negotiationOwner != nil {
            try ensureCurrent(generation)
            try Task.checkCancellation()
            try await Task.sleep(for: .milliseconds(20))
        }
        try ensureCurrent(generation)
        try Task.checkCancellation()
        let owner = UUID()
        negotiationOwner = owner
        return owner
    }

    private func releaseNegotiation(_ owner: UUID) {
        if negotiationOwner == owner { negotiationOwner = nil }
    }

    func setMicrophoneEnabled(_ enabled: Bool, generation: UInt64) async throws {
        try ensureCurrent(generation)
        microphoneRevision &+= 1
        let revision = microphoneRevision
        microphoneRequested = enabled
        microphoneTask?.cancel()
        if !enabled { await peerRuntime?.stopMicrophoneCapture() }
        let task = Task { [self] in
            let owner = try await acquireNegotiation(generation: generation)
            defer { releaseNegotiation(owner) }
            try ensureCurrent(generation)
            guard revision == microphoneRevision, let peer = peerRuntime,
                let control = controlSession
            else {
                throw StreamingRuntimeError.superseded
            }
            let epoch = activePeerEpoch
            do {
                let offer = try await peer.makeMicrophoneOffer(enabled: enabled)
                try ensureCurrent(generation)
                try Task.checkCancellation()
                guard epoch == activePeerEpoch, revision == microphoneRevision else {
                    throw StreamingRuntimeError.superseded
                }
                let answer = try await control.exchangeMediaOffer(offer, kind: "chat")
                try ensureCurrent(generation)
                try Task.checkCancellation()
                try await applyRemoteAnswer(answer, to: peer, phase: "microphone")
                try ensureCurrent(generation)
                try Task.checkCancellation()
                guard epoch == activePeerEpoch, revision == microphoneRevision else {
                    throw StreamingRuntimeError.superseded
                }
                microphonePeerEpoch = enabled ? epoch : 0
                await microphoneSink?(generation, enabled, false)
                trace(
                    "microphoneNegotiated", payload: ["enabled": .bool(enabled)],
                    dimension: .mediaSupply)
            } catch {
                await peer.stopMicrophoneCapture()
                if isCurrent(generation), epoch == activePeerEpoch {
                    do { try await peer.rollbackMediaNegotiation() } catch {
                        trace("microphoneRollbackFailed", dimension: .mediaSupply)
                    }
                }
                if revision == microphoneRevision {
                    microphoneRequested = false
                    microphonePeerEpoch = 0
                    if isCurrent(generation) { await microphoneSink?(generation, false, false) }
                }
                throw error
            }
        }
        microphoneTask = task
        try await task.value
    }

    func setVisible(_ value: Bool, generation: UInt64) async {
        guard isCurrent(generation), visible != value else { return }
        visibilityRevision &+= 1
        let revision = visibilityRevision
        visible = value
        if !value { cancelDisplayRecovery() }
        latestPeerStats = nil
        rtcTraceWindow = StreamingTraceCounterWindow()
        displayTraceWindow = StreamingTraceCounterWindow()
        latestDisplayHealthSnapshot = nil
        supervisionEligibleSinceMs = Self.nowMs
        firstFrameGuardTask?.cancel()
        firstFrameGuardTask = nil
        recoveryProbe = nil
        if !value {
            resumedAtMilliseconds = nil
            // 前后台只切换呈现与输入；当前 peer、接收音频和播放就绪事实持续保留。
            trace(
                "presentationPaused", payload: ["logicalSessionRetained": .bool(true)],
                dimension: .lifecycle)
            guard microphoneRequested || microphonePeerEpoch != 0 || microphoneTask != nil else {
                return
            }
            microphoneRevision &+= 1
            microphoneTask?.cancel()
            microphoneTask = nil
            microphonePeerEpoch = 0
            await peerRuntime?.stopMicrophoneCapture()
            guard isCurrent(generation), !visible, revision == visibilityRevision else { return }
            await microphoneSink?(generation, false, microphoneRequested)
            return
        }
        guard peerRuntime != nil else { return }
        let peerEpoch = activePeerEpoch
        resumedAtMilliseconds = ProcessInfo.processInfo.systemUptime * 1_000
        lastDecodeActivityAtMs = Self.nowMs
        trace(
            "presentationResumed", payload: ["logicalSessionRetained": .bool(true)],
            dimension: .lifecycle)
        switch peerConnectionState {
        case .connected:
            armFirstFrameGuard(generation: generation, peerEpoch: peerEpoch)
        case .failed, .closed:
            // 后台期间记录的真实连接故障沿公共恢复合同处理。
            await recoverDisconnectedPeer(generation: generation, peerEpoch: peerEpoch)
        case .new, .checking, .disconnected:
            break
        }
        guard isCurrent(generation), visible, revision == visibilityRevision,
            peerEpoch == activePeerEpoch, microphoneRequested
        else { return }
        do {
            try await setMicrophoneEnabled(true, generation: generation)
        } catch {
            trace("microphoneRestoreFailed", dimension: .mediaSupply)
        }
    }

    func sendText(_ text: String, generation: UInt64) async throws -> Bool {
        guard isCurrent(generation), let controlSession else {
            throw StreamingRuntimeError.superseded
        }
        let accepted = try await controlSession.sendText(text)
        trace(
            accepted ? "consoleTextAccepted" : "consoleTextRejected",
            dimension: .input
        )
        return accepted
    }

    func powerOffConsole(generation: UInt64) async throws -> Bool {
        guard isCurrent(generation), let controlSession else {
            throw StreamingRuntimeError.superseded
        }
        let accepted = try await controlSession.powerOffConsole()
        trace(
            accepted ? "consolePowerOffAccepted" : "consolePowerOffRejected",
            dimension: .lifecycle
        )
        return accepted
    }

    func displayPipelineReady(context: StreamingPresentationTraceContext) async {
        guard isCurrent(context.generation),
            context.attemptID == activeAttemptID,
            context.peerEpoch == activePeerEpoch, context.videoGeneration == videoGeneration,
            !playbackReadiness.contains(.displayPipelineReady)
        else { return }
        playbackReadiness.mark(.displayPipelineReady)
        if playbackReady {
            await publish(.playing, generation: context.generation)
        }
    }

    func firstFrameAcceptedToMailbox(context: StreamingPresentationTraceContext) async {
        guard isCurrent(context.generation),
            context.attemptID == activeAttemptID,
            context.peerEpoch == activePeerEpoch, context.videoGeneration == videoGeneration,
            !playbackReadiness.contains(.frameAcceptedToMailbox)
        else { return }
        playbackReadiness.mark(.frameAcceptedToMailbox)
        if playbackReady {
            await publish(.playing, generation: context.generation)
        }
    }

    func firstFrameSubmitted(context: StreamingPresentationTraceContext) async {
        guard isCurrent(context.generation),
            context.attemptID == activeAttemptID,
            context.peerEpoch == activePeerEpoch, context.videoGeneration == videoGeneration,
            !playbackReadiness.contains(.frameSubmitted)
        else { return }
        playbackReadiness.mark(.frameSubmitted)
        if playbackReady {
            await publish(.playing, generation: context.generation)
        }
    }

    func firstFramePresented(context: StreamingPresentationTraceContext) async {
        guard visible, resumedAtMilliseconds == nil, isCurrent(context.generation),
            context.attemptID == activeAttemptID,
            context.peerEpoch == activePeerEpoch, context.videoGeneration == videoGeneration,
            !playbackReadiness.contains(.framePresented)
        else { return }
        playbackReadiness.mark(.framePresented)
        if playbackReady {
            await publish(.playing, generation: context.generation)
        }
    }

    func displayHealthUpdated(
        context: StreamingPresentationTraceContext,
        snapshot: StreamingDisplayHealthSnapshot
    ) async {
        guard visible, isCurrent(context.generation),
            context.attemptID == activeAttemptID,
            context.peerEpoch == activePeerEpoch
        else { return }
        guard context.videoGeneration == videoGeneration else { return }
        if let current = currentVideoHandle?.presentation.snapshot,
            current.scope.renderEpoch != snapshot.renderEpoch
        {
            return
        }
        latestDisplayHealthSnapshot = snapshot
        if let resumedAtMilliseconds,
            let presentedAt = snapshot.lastDisplayedAtMs,
            presentedAt > resumedAtMilliseconds
        {
            playbackReadiness.mark(.framePresented)
            self.resumedAtMilliseconds = nil
            firstFrameGuardTask?.cancel()
            firstFrameGuardTask = nil
        }
        if playbackReady {
            await publish(.playing, generation: context.generation)
        }
    }

    private func handlePeerEvent(
        _ event: StreamingPeerEvent,
        attemptID: String,
        generation: UInt64,
        peerEpoch: UInt64
    ) async {
        guard isCurrent(generation), peerEpoch == activePeerEpoch else { return }
        if case .audioLifecycle(let event, let payload) = event {
            latestAudioOutput = StreamingAudioOutputSnapshot(payload: payload) ?? latestAudioOutput
            latestAudioLifecycleState =
                Self.stringValue(payload["state"])
                ?? latestAudioLifecycleState
            latestAudioRoute =
                Self.stringValue(payload["route"])
                ?? Self.stringValue(payload["outputRoute"])
                ?? latestAudioRoute
            var contextualPayload = payload
            contextualPayload["peerEpoch"] = .integer(Int64(peerEpoch))
            traceAttempt(
                event,
                attemptID: attemptID,
                generation: generation,
                payload: contextualPayload,
                dimension: .mediaSupply,
                importance: .key,
                operationID: attemptID
            )
            await publishDiagnostics(generation: generation)
            return
        }
        switch event {
        case .localCandidate(let candidate):
            guard let epoch = activeLocalIceEpoch, epoch.peerEpoch == peerEpoch else { return }
            pendingLocalCandidates.append(candidate)
            scheduleLocalIceFlush(generation: generation, epoch: epoch)
        case .localIceGatheringComplete:
            guard let epoch = activeLocalIceEpoch, epoch.peerEpoch == peerEpoch else { return }
            localIceState.gatheringComplete = true
            scheduleLocalIceFlush(generation: generation, epoch: epoch)
        case .connectionStateChanged(let state):
            peerConnectionState = state
            switch state {
            case .connected:
                guard localIceState.remoteDescriptionApplied, !initialNegotiationInFlight else {
                    pendingConnectedPeerEpoch = peerEpoch
                    return
                }
                await markPeerConnected(generation: generation, peerEpoch: peerEpoch)
            case .disconnected:
                trace("peerDisconnected", dimension: .network)
            case .failed, .closed:
                guard localIceState.remoteDescriptionApplied, !initialNegotiationInFlight else {
                    if !reconnectInFlight { pendingDisconnectedPeerEpoch = peerEpoch }
                    return
                }
                await recoverDisconnectedPeer(generation: generation, peerEpoch: peerEpoch)
            case .new, .checking:
                break
            }
        case .videoTrack(let track):
            currentVideoHandle = track
            guard let attemptID = activeAttemptID else { return }
            await videoSink?(
                generation,
                StreamingVideoTrackHandle(
                    rawValue: track.rawValue,
                    traceContext: StreamingPresentationTraceContext(
                        attemptID: attemptID,
                        generation: generation,
                        peerEpoch: peerEpoch, videoGeneration: videoGeneration
                    ),
                    frameFeedback: track.frameFeedback, presentation: track.presentation
                )
            )
        case .firstVideoFrame:
            lastDecodeActivityAtMs = Self.nowMs
            firstFrameGuardTask?.cancel()
            firstFrameGuardTask = nil
            let isFirstFrame = playbackReadiness.mark(.firstVideoFrame)
            if isFirstFrame { trace("firstVideoFrame", dimension: .mediaSupply) }
            if playbackReady {
                await publish(.playing, generation: generation)
            }
        case .audioTrackReady:
            audioTrackReady = true
            trace("audioTrackReady", dimension: .mediaSupply)
            await publishDiagnostics(generation: generation)
        case .audioLifecycle:
            break
        case .stats(let stats):
            if stats.frameSupplyDelta.map({ $0 > 0 }) == true {
                lastDecodeActivityAtMs = Self.nowMs
            }
            latestPeerStats = stats
            let sampledAt = stats.observedAtMilliseconds ?? Double(Self.nowMs)
            let sampleScope = "\(activePeerEpoch):\(videoGeneration)"
            let rtcWindow = rtcTraceWindow.observe(
                at: sampledAt, scope: sampleScope,
                counters: [
                    "framesReceived": stats.framesReceived.map { Int64(clamping: $0) },
                    "framesDecoded": stats.observedFramesDecoded.map { Int64(clamping: $0) },
                    "packetsReceived": Int64(clamping: stats.packetsReceived),
                    "packetsLost": stats.videoPacketsLost,
                ])
            var rtcPayload: [String: IOSRuntimeTraceValue] = [
                "inboundVideoBytes": .integer(Int64(clamping: stats.inboundVideoBytes)),
                "inboundAudioBytes": .integer(Int64(clamping: stats.inboundAudioBytes)),
                "framesDecoded": .integer(Int64(clamping: stats.framesDecoded)),
                "framesDropped": .integer(Int64(clamping: stats.framesDropped)),
                "packetsLost": .integer(stats.packetsLost),
                "packetStatisticsScope": .string("video"),
                "packetsReceived": .integer(Int64(clamping: stats.packetsReceived)),
                "rttMs": stats.roundTripTimeSeconds.map { .double($0 * 1_000) } ?? .null,
                "jitterMs": stats.jitterSeconds.map { .double($0 * 1_000) } ?? .null,
                "receiveBitrateBps": stats.receiveBitrateBps.map(IOSRuntimeTraceValue.double)
                    ?? .null,
                "freezeCount": stats.freezeCount.map { .integer(Int64(clamping: $0)) } ?? .null,
                "freezeDurationMs": stats.freezeDurationSeconds.map {
                    .double($0 * 1_000)
                } ?? .null,
                "nackCount": stats.nackCount.map { .integer(Int64(clamping: $0)) } ?? .null,
                "pliCount": stats.pliCount.map { .integer(Int64(clamping: $0)) } ?? .null,
                "firCount": stats.firCount.map { .integer(Int64(clamping: $0)) } ?? .null,
                "selectedCandidatePairProtocol": stats.selectedCandidatePairProtocol.map(
                    IOSRuntimeTraceValue.string
                ) ?? .string("unsupported"),
                "selectedCandidatePairAddressFamily": stats
                    .selectedCandidatePairAddressFamily.map(IOSRuntimeTraceValue.string)
                    ?? .string("unsupported"),
                "selectedLocalCandidateType": stats.selectedLocalCandidateType.map(
                    IOSRuntimeTraceValue.string
                ) ?? .string("unsupported"),
                "selectedRemoteCandidateType": stats.selectedRemoteCandidateType.map(
                    IOSRuntimeTraceValue.string
                ) ?? .string("unsupported"),
                "firstMediaAtMs": stats.firstMediaAtMilliseconds.map(
                    IOSRuntimeTraceValue.double
                ) ?? .null,
                "lastMediaAtMs": stats.lastMediaAtMilliseconds.map(
                    IOSRuntimeTraceValue.double
                ) ?? .null,
                "frameSupplyDelta": stats.frameSupplyDelta.map(IOSRuntimeTraceValue.integer)
                    ?? .null,
            ]
            rtcPayload.merge(stats.videoTiming.tracePayload()) { _, new in new }
            rtcPayload.merge(stats.audioTiming.tracePayload(prefix: "audio")) { _, new in new }
            rtcPayload["videoPacketsLost"] =
                stats.videoPacketsLost.map(IOSRuntimeTraceValue.integer) ?? .null
            rtcPayload["framesReceived"] =
                stats.framesReceived.map {
                    IOSRuntimeTraceValue.integer(Int64(clamping: $0))
                } ?? .null
            rtcPayload["observedFramesDecoded"] =
                stats.observedFramesDecoded.map {
                    IOSRuntimeTraceValue.integer(Int64(clamping: $0))
                } ?? .null
            rtcPayload["mediaTimeSource"] = .string("rtcStatisticsObservation")
            rtcPayload["sampleId"] = .string("\(sampleScope):\(sampledAt)")
            rtcPayload.merge(rtcWindow) { _, value in value }
            traceSnapshot(
                "rtcHealthSnapshot", payload: rtcPayload, dimension: .mediaSupply, importance: .key)
            if let latestDisplayHealthSnapshot {
                var displayPayload = latestDisplayHealthSnapshot.tracePayload()
                let window = displayTraceWindow.observe(
                    at: sampledAt, scope: sampleScope,
                    counters: [
                        "presentCount": latestDisplayHealthSnapshot.presentCount,
                        "submitCount": latestDisplayHealthSnapshot.submitCount,
                        "drawablePresentCount": latestDisplayHealthSnapshot.drawablePresentCount,
                    ])
                displayPayload.merge(window) { _, value in value }
                displayPayload["displayedFrameDelta"] = window["presentCountDelta"]
                displayPayload["sampleId"] = .string("\(sampleScope):\(sampledAt)")
                traceSnapshot(
                    "displayHealthSnapshot",
                    payload: displayPayload,
                    dimension: .presentation,
                    importance: .key
                )
            }
            await publishDiagnostics(generation: generation)
            if !playbackReadiness.contains(.steadyMedia),
                stats.framesDecoded > 0, stats.inboundVideoBytes > 0,
                stats.frameSupplyDelta.map({ $0 > 0 }) == true
            {
                playbackReadiness.mark(.steadyMedia)
                trace(
                    "steadyMediaObserved", payload: ["presentationActive": .bool(visible)],
                    dimension: .mediaSupply)
            }
        case .dataChannelEvent(let label, let event):
            await traceDataChannelEvent(label: label, event: event)
        case .failed(let message):
            let reason = StreamingTerminalReason(peerFailureMessage: message)
            await fail(
                reason: reason,
                generation: generation,
                error: NSError(
                    domain: "ios-streaming-peer",
                    code: 1,
                    userInfo: [NSLocalizedDescriptionKey: reason.userMessage]
                )
            )
        }
    }

    private func publishDiagnostics(generation: UInt64) async {
        guard isCurrent(generation) else { return }
        let peerSnapshot = await peerRuntime?.debugSnapshot()
        let negotiatedBitrate = await peerRuntime?.remoteVideoBitrateCapKbps()
        let snapshot = StreamingSessionDiagnostics(
            peerStats: latestPeerStats,
            displayHealth: latestDisplayHealthSnapshot,
            peer: peerSnapshot,
            audioLifecycleState: latestAudioLifecycleState,
            audioRoute: latestAudioRoute,
            audioTrackReady: audioTrackReady,
            updatedAtMilliseconds: ProcessInfo.processInfo.systemUptime * 1_000,
            supervision: latestDecision,
            requestedVideoBitrateKbps: preparedSignaling?.webRtcPlan.maxVideoBitrateKbps,
            negotiatedVideoBitrateKbps: negotiatedBitrate,
            audioOutput: latestAudioOutput
        )
        if let sink = diagnosticsSink {
            Task { @MainActor in sink(generation, snapshot) }
        }
    }

    private static func stringValue(_ value: IOSRuntimeTraceValue?) -> String? {
        guard case .string(let value) = value else { return nil }
        return value
    }

    private func makePeerRuntime(generation: UInt64) async throws -> any StreamingPeerRuntime {
        guard let attemptID = activeAttemptID else {
            throw StreamingRuntimeError.superseded
        }
        nextPeerEpoch &+= 1
        let peerEpoch = nextPeerEpoch
        activePeerEpoch = peerEpoch
        playbackReadiness.resetForPeer()
        videoGeneration &+= 1
        currentVideoHandle = nil
        latestPeerStats = nil
        rtcTraceWindow = StreamingTraceCounterWindow()
        displayTraceWindow = StreamingTraceCounterWindow()
        qualityLevel = 0
        qualityCompletion = nil
        recoveryProbe = nil
        lastLocalActionAtMs = nil
        dataChannelProfilesPendingTrace = false
        pendingConnectedPeerEpoch = nil
        pendingDisconnectedPeerEpoch = nil
        latestDisplayHealthSnapshot = nil
        audioTrackReady = false
        let peer = peerFactory.makeRuntime { [weak self] event in
            Task {
                await self?.handlePeerEvent(
                    event,
                    attemptID: attemptID,
                    generation: generation,
                    peerEpoch: peerEpoch
                )
            }
        }
        guard isCurrent(generation), activePeerEpoch == peerEpoch else {
            await peer.stopInputAndHaptics()
            await peer.closeTransport()
            throw StreamingRuntimeError.superseded
        }
        peerRuntime = peer
        try await peer.setAudioMuted(requestedAudioMuted)
        try ensureCurrent(generation)
        return peer
    }

    private func completePendingPeerConnection(generation: UInt64, peerEpoch: UInt64) async {
        guard pendingConnectedPeerEpoch == peerEpoch else { return }
        pendingConnectedPeerEpoch = nil
        await markPeerConnected(generation: generation, peerEpoch: peerEpoch)
    }

    private func completePendingPeerDisconnection(
        generation: UInt64,
        peerEpoch: UInt64
    ) async {
        guard pendingDisconnectedPeerEpoch == peerEpoch else { return }
        pendingDisconnectedPeerEpoch = nil
        await recoverDisconnectedPeer(generation: generation, peerEpoch: peerEpoch)
    }

    private func markPeerConnected(generation: UInt64, peerEpoch: UInt64) async {
        guard isCurrent(generation), peerEpoch == activePeerEpoch,
            localIceState.remoteDescriptionApplied,
            !playbackReadiness.contains(.peerConnected)
        else { return }
        playbackReadiness.mark(.peerConnected)
        hasEverConnected = true
        initialConnectionTask?.cancel()
        initialConnectionTask = nil
        do {
            try await controlSession?.markConnected()
            guard isCurrent(generation), peerEpoch == activePeerEpoch else { return }
            supervisionConnectedAtMs = Self.nowMs
            lastDecodeActivityAtMs = Self.nowMs
            startSupervision(generation: generation)
            armFirstFrameGuard(generation: generation, peerEpoch: peerEpoch)
            if microphoneRequested && microphonePeerEpoch != peerEpoch {
                Task { [weak self] in
                    guard let self, await self.isCurrent(generation),
                        await self.activePeerEpoch == peerEpoch
                    else { return }
                    do { try await self.setMicrophoneEnabled(true, generation: generation) } catch {
                        await self.trace("microphoneRestoreFailed", dimension: .mediaSupply)
                    }
                }
            }
            trace("peerConnected", dimension: .network)
            if dataChannelProfilesPendingTrace {
                dataChannelProfilesPendingTrace = false
                traceDataChannelProfiles()
            }
            await publish(
                playbackReady
                    ? .playing
                    : .waitingForFirstFrame,
                generation: generation
            )
        } catch {
            playbackReadiness = StreamingPlaybackReadiness()
            await fail(reason: .sessionFailed, generation: generation, error: error)
        }
    }

    private static var nowMs: UInt64 { UInt64(StreamingFrameFeedbackClock.nowMilliseconds) }

    private func armInitialConnectionDeadline(generation: UInt64) {
        initialConnectionTask?.cancel()
        guard !hasEverConnected else { return }
        let timeout = initialConnectionTimeout
        let epoch = activePeerEpoch
        initialConnectionTask = Task { [weak self] in
            do { try await Task.sleep(for: timeout) } catch { return }
            guard let self, await self.isCurrent(generation), await self.activePeerEpoch == epoch,
                await !self.hasEverConnected
            else { return }
            await self.initialConnectionExpired(generation: generation, peerEpoch: epoch)
        }
    }

    private func initialConnectionExpired(generation: UInt64, peerEpoch: UInt64) async {
        initialConnectionTask = nil
        trace(
            "initialConnectionTimedOut",
            payload: ["timeoutMs": .integer(initialConnectionTimeout.milliseconds)],
            dimension: .recovery)
        if preparedSignaling?.fallbackIceServers != nil && !fallbackRetryConsumed {
            await recoverDisconnectedPeer(generation: generation, peerEpoch: peerEpoch)
        } else {
            await fail(
                reason: .iceFailed, generation: generation,
                error: StreamingRuntimeError.peerConnectionFailed)
        }
    }

    private func startSupervision(generation: UInt64) {
        guard supervisionTask == nil else { return }
        supervisionTask = Task { [weak self] in
            while !Task.isCancelled {
                do { try await Task.sleep(for: .seconds(1)) } catch { return }
                guard let self, await self.isCurrent(generation) else { return }
                await self.superviseTick(generation: generation)
            }
        }
    }

    private func armFirstFrameGuard(generation: UInt64, peerEpoch: UInt64) {
        firstFrameGuardTask?.cancel()
        firstFrameGuardTask = Task { [weak self] in
            do { try await Task.sleep(for: .seconds(4)) } catch { return }
            guard let self, await self.isCurrent(generation),
                await self.activePeerEpoch == peerEpoch
            else { return }
            await self.requestLocalKeyframe(generation: generation, firstFrame: true)
        }
    }

    @discardableResult
    private func requestLocalKeyframe(generation: UInt64, firstFrame: Bool) async -> Bool {
        guard visible, isCurrent(generation), !reconnectInFlight, keyframeBudget > 0,
            recoveryProbe == nil, let peer = peerRuntime
        else { return false }
        if firstFrame && playbackReadiness.contains(.firstVideoFrame)
            && resumedAtMilliseconds == nil
        {
            return false
        }
        let epoch = activePeerEpoch
        let sent = await peer.requestVideoKeyframe()
        guard isCurrent(generation), epoch == activePeerEpoch else { return false }
        lastLocalActionAtMs = Self.nowMs
        trace(
            firstFrame ? "firstFrameGuardTriggered" : "recoveryActionExecuted",
            payload: ["action": .string("keyframeRequest"), "sent": .bool(sent)],
            dimension: .recovery)
        if sent {
            keyframeBudget -= 1
            beginRecoveryProbe(action: "keyframeRequest")
        }
        return sent
    }

    private func beginRecoveryProbe(action: String) {
        let now = Self.nowMs
        let required =
            action == "playbackRebuild"
            ? UInt64(3000) : UInt64(max(3000, (latestPeerStats?.roundTripTimeSeconds ?? 0) * 2000))
        lastLocalActionAtMs = now
        recoveryProbe = StreamingRecoveryProbe(
            action: action, startedAtMs: now, observeMs: min(15000, required),
            requiredObserveMs: required,
            sampleId: latestPeerStats?.observedAtMilliseconds.map {
                "\(activePeerEpoch):\(UInt64($0))"
            },
            videoGeneration: videoGeneration, peerGeneration: activePeerEpoch,
            baseline: .init(
                decodeFps: latestDecision.short.decodeFpsAvg,
                videoFps: latestDecision.short.videoPresentFpsAvg))
    }

    private func superviseTick(generation: UInt64) async {
        guard visible, peerConnectionState == .connected, isCurrent(generation), !reconnectInFlight,
            let control = controlSession,
            let peer = peerRuntime
        else { return }
        let epoch = activePeerEpoch
        let debug = await peer.debugSnapshot()
        let qualitySupported = await peer.remoteVideoBitrateCapKbps() != nil
        guard isCurrent(generation), epoch == activePeerEpoch, visible,
            peerConnectionState == .connected, !reconnectInFlight
        else { return }
        // 所有异步依赖完成后取判定时间，后续事实快照在同一 actor 执行片段内完成。
        let displayObservation = currentVideoHandle?.presentation.snapshot
        let now = Self.nowMs
        if let displayObservation,
            displayObservation.scope.presentation?.peerEpoch == epoch,
            displayObservation.scope.presentation?.videoGeneration == videoGeneration
        {
            latestDisplayHealthSnapshot = displayObservation.health
        }
        var sample = StreamingSupervisionSample(nowMs: now)
        sample.displayRecoveryCompletion = displayRecoveryCompletion
        if let displayObservation,
            displayObservation.scope.presentation?.peerEpoch == epoch,
            displayObservation.scope.presentation?.videoGeneration == videoGeneration
        {
            sample.displayObservation = .init(displayObservation, now: Double(now))
        }
        sample.peerGeneration = epoch
        sample.connectionState = peerConnectionState.rawValue
        sample.connectionRecoveryActionCompleted = connectionRecoveryAwaitingFrames
        sample.videoGeneration = videoGeneration
        sample.connectedElapsedMs =
            now >= supervisionConnectedAtMs ? now - supervisionConnectedAtMs : 0
        sample.inactivityElapsedMs =
            now >= lastDecodeActivityAtMs ? now - lastDecodeActivityAtMs : 0
        sample.frameIntervalMs = latestDisplayHealthSnapshot?.frameIntervalMilliseconds
        sample.controlChannelState = debug.dataChannels?.readyStates["control"] ?? "unknown"
        sample.previousActionAtMs = lastLocalActionAtMs
        sample.eligibleSinceMs = supervisionEligibleSinceMs
        sample.warmupUntilMs = supervisionConnectedAtMs + 7000
        sample.probe = recoveryProbe
        sample.qualityLevel = qualityLevel
        sample.qualitySupported = qualitySupported
        sample.qualityCompletion = qualityCompletion
        if let stats = latestPeerStats, let at = stats.observedAtMilliseconds,
            at.isFinite && at >= 0
        {
            sample.sampledAtMs = UInt64(at)
            sample.sampleId = "\(epoch):\(UInt64(at))"
            sample.observationFresh = now >= UInt64(at) && now - UInt64(at) <= 1000
            sample.inboundBytesTotal = stats.inboundVideoBytes
            sample.inboundVideoPacketCountTotal = stats.packetsReceived
            sample.counters = StreamingMediaCounters(
                received: stats.framesReceived, decoded: stats.observedFramesDecoded,
                presented: latestDisplayHealthSnapshot.map { UInt64(max(0, $0.presentCount)) },
                packetsReceived: stats.packetsReceived, packetsLost: stats.videoPacketsLost)
            sample.rttMs = stats.roundTripTimeSeconds.map { $0 * 1000 }
            sample.jitterMs = stats.jitterSeconds.map { $0 * 1000 }
            sample.jitterBufferMs = stats.videoTiming.jitterBufferMilliseconds
            sample.pathId = stats.selectedCandidatePairID
            sample.packetAgeMs = stats.lastMediaAtMilliseconds.map { max(0, Double(now) - $0) }
        }
        sample.videoPresentAgeMs = latestDisplayHealthSnapshot?.lastDisplayedAtMs.map {
            max(0, Double(now) - $0)
        }
        do {
            let decision = try await control.supervise(sample)
            guard isCurrent(generation), epoch == activePeerEpoch, visible else { return }
            latestDecision = decision
            if let completion = sample.displayRecoveryCompletion,
                displayRecoveryCompletion?.id == completion.id
            {
                displayRecoveryCompletion = nil
            }
            await publishDiagnostics(generation: generation)
            guard isCurrent(generation), epoch == activePeerEpoch, visible else { return }
            if let effect = decision.probeEffect {
                trace(
                    "recoveryActionEffectEvaluated",
                    payload: [
                        "result": .string(effect.result), "reason": .string(effect.reason),
                        "score": effect.score.map(IOSRuntimeTraceValue.double) ?? .null,
                    ], dimension: .recovery)
                recoveryProbe = nil
            }
            traceSnapshot(
                "runtimeSupervisionSnapshot",
                payload: [
                    "fresh": .bool(decision.fresh),
                    "sampledAtMonotonicMs": sample.sampledAtMs.map { .integer(Int64(clamping: $0)) }
                        ?? .null,
                    "sampleId": sample.sampleId.map(IOSRuntimeTraceValue.string) ?? .null,
                    "videoGeneration": .integer(Int64(clamping: videoGeneration)),
                    "networkPressure": .string(decision.networkPressure),
                    "stallThresholdMs": .integer(Int64(decision.stallThresholdMs)),
                    "decodePressureSustainedMs": .integer(
                        Int64(decision.decodePressureSustainedMs)),
                    "decodeRecoveryEligible": .bool(decision.decodeRecoveryEligible),
                    "displayRecoveryEligible": .bool(decision.displayRecoveryEligible == true),
                    "displayRecoveryActive": .bool(decision.displayRecovery?.active == true),
                    "displayRecoveryReason": .string(
                        decision.displayRecovery?.reason ?? "unobserved"),
                    "displayAgeMs": .integer(Int64(clamping: decision.displayRecovery?.ageMs ?? 0)),
                    "displayCoverageMs": .integer(
                        Int64(clamping: decision.displayRecovery?.coverageMs ?? 0)),
                    "qualityState": .string(decision.quality?.state ?? "unknown"),
                    "qualityEffect": decision.quality?.effect.map(IOSRuntimeTraceValue.string)
                        ?? .null,
                    "rttBaselineMs": decision.quality?.rttBaselineMs.map(
                        IOSRuntimeTraceValue.double) ?? .null,
                    "rttDeltaMs": decision.quality?.rttDeltaMs.map(IOSRuntimeTraceValue.double)
                        ?? .null,
                    "inboundFps": decision.short.inboundFpsAvg.map(IOSRuntimeTraceValue.double)
                        ?? .null,
                    "decodeFps": decision.short.decodeFpsAvg.map(IOSRuntimeTraceValue.double)
                        ?? .null,
                    "presentFps": decision.short.videoPresentFpsAvg.map(IOSRuntimeTraceValue.double)
                        ?? .null,
                    "keyframeBudget": .integer(Int64(keyframeBudget)),
                    "playbackBudget": .integer(Int64(playbackBudget)),
                    "connectionRecoveryActive": .bool(decision.connectionRecoveryActive),
                    "videoPresentAgeMs": sample.videoPresentAgeMs.map(IOSRuntimeTraceValue.double)
                        ?? .null,
                ], dimension: .recovery, importance: .key)
            guard decision.fresh, !decision.duplicate else { return }
            if !decision.connectionRecoveryActive {
                connectionRecoveryAwaitingFrames = false
                connectionRecoveryDeadlineTask?.cancel()
                connectionRecoveryDeadlineTask = nil
            }
            if decision.healthyEpisodeComplete && recoveryProbe == nil {
                keyframeBudget = 2
                playbackBudget = 1
                lastLocalActionAtMs = nil
            }
            if decision.displayRecovery?.effect == "degraded" {
                trace(
                    "displayRecoveryFailed",
                    payload: ["reason": .string(decision.displayRecovery?.reason ?? "unknown")],
                    dimension: .recovery)
                await fail(
                    reason: .runtimeFailure, generation: generation,
                    error: NSError(
                        domain: "XBXRC.DisplayRecovery", code: 1,
                        userInfo: [NSLocalizedDescriptionKey: "画面呈现恢复失败，请重新连接"]))
                return
            }
            if let action = decision.displayRecovery?.action,
                let display = sample.displayObservation,
                let source = currentVideoHandle?.presentation
            {
                startDisplayRecovery(
                    action, observation: display, source: source,
                    thresholdMs: decision.displayRecovery?.thresholdMs ?? 500,
                    generation: generation, peerEpoch: epoch)
            }
            if let action = decision.quality?.action, !qualityOperationInFlight {
                await applyQualityAction(action, generation: generation, peerEpoch: epoch)
                return
            }
            if decision.displayRecovery?.active == true {
                if decision.mediaConnectionEligible {
                    cancelDisplayRecovery()
                    await recoverDisconnectedPeer(generation: generation, peerEpoch: epoch)
                }
                return
            }
            guard sample.inactivityElapsedMs >= decision.stallThresholdMs else { return }
            if decision.hasInboundProgress
                && sample.inactivityElapsedMs < decision.stallThresholdMs + 8000
            {
                return
            }
            if decision.decodeRecoveryEligible && recoveryProbe == nil
                && sample.controlChannelState == "open"
            {
                if keyframeBudget > 0 {
                    if await requestLocalKeyframe(generation: generation, firstFrame: false) {
                        return
                    }
                }
                if keyframeBudget == 0, playbackBudget > 0, let handle = currentVideoHandle,
                    let attemptID = activeAttemptID
                {
                    playbackBudget -= 1
                    beginRecoveryProbe(action: "playbackRebuild")
                    videoGeneration &+= 1
                    latestDisplayHealthSnapshot = nil
                    await videoSink?(
                        generation,
                        StreamingVideoTrackHandle(
                            rawValue: handle.rawValue,
                            traceContext: .init(
                                attemptID: attemptID, generation: generation, peerEpoch: epoch,
                                videoGeneration: videoGeneration),
                            frameFeedback: handle.frameFeedback, presentation: handle.presentation))
                    trace(
                        "recoveryActionExecuted", payload: ["action": .string("playbackRebuild")],
                        dimension: .recovery)
                    return
                }
            }
            if decision.mediaConnectionEligible {
                await recoverDisconnectedPeer(generation: generation, peerEpoch: epoch)
            }
        } catch {
            guard isCurrent(generation), epoch == activePeerEpoch else { return }
            trace(
                "runtimeSupervisionUnavailable",
                payload: ["reason": .string(CloudLibraryDiagnostics.safeError(error))],
                dimension: .recovery
            )
        }
    }

    private func applyQualityAction(
        _ action: StreamingQualityAction, generation: UInt64, peerEpoch: UInt64
    ) async {
        guard let peer = peerRuntime, let control = controlSession,
            let base = preparedSignaling?.webRtcPlan.maxVideoBitrateKbps
        else { return }
        qualityOperationInFlight = true
        defer { qualityOperationInFlight = false }
        let previousLevel = qualityLevel
        let requested = max(
            2000, Int((Double(base) * [1.0, 0.75, 0.55][min(2, max(0, action.level))]).rounded()))
        var negotiation: UUID?
        defer { if let negotiation { releaseNegotiation(negotiation) } }
        do {
            negotiation = try await acquireNegotiation(generation: generation)
            guard isCurrent(generation), peerEpoch == activePeerEpoch else { return }
            let offer = try await peer.makeQualityOffer(maxVideoBitrateKbps: requested)
            try ensureCurrent(generation)
            let answer = try await control.exchangeMediaOffer(offer, kind: "quality")
            try ensureCurrent(generation)
            try await applyRemoteAnswer(answer, to: peer, phase: "quality")
            guard isCurrent(generation), peerEpoch == activePeerEpoch else { return }
            let accepted =
                await peer.remoteVideoBitrateCapKbps().map {
                    abs(Double($0 - requested)) <= Double(requested) * 0.02
                } ?? false
            if accepted { qualityLevel = action.level }
            qualityCompletion = .init(
                id: action.id, result: accepted ? "applied" : "unsupported",
                previousLevel: previousLevel,
                completedAtMs: Self.nowMs)
            trace(
                "qualityActionCompleted",
                payload: [
                    "level": .integer(Int64(action.level)),
                    "requestedBitrateKbps": .integer(Int64(requested)),
                    "result": .string(accepted ? "applied" : "unsupported"),
                ], dimension: .network)
        } catch {
            guard isCurrent(generation), peerEpoch == activePeerEpoch else { return }
            try? await peer.rollbackMediaNegotiation()
            qualityCompletion = .init(
                id: action.id, result: "failed", previousLevel: previousLevel,
                completedAtMs: Self.nowMs)
            trace(
                "qualityActionCompleted", payload: ["result": .string("failed")],
                dimension: .network)
        }
    }

    private func startRemoteIceApplication(generation: UInt64, peerEpoch: UInt64) {
        remoteIceApplicationTask?.cancel()
        remoteIceApplicationTask = Task { [weak self] in
            var didTraceRemoteIceCompleted = false
            while !Task.isCancelled {
                do {
                    guard let self, await self.isCurrent(generation),
                        await self.activePeerEpoch == peerEpoch,
                        let control = await self.controlSession,
                        let peer = await self.peerRuntime
                    else { return }
                    let batch = try await control.nextRemoteIceBatch()
                    guard !Task.isCancelled, await self.isCurrent(generation),
                        await self.activePeerEpoch == peerEpoch
                    else { return }
                    if !didTraceRemoteIceCompleted {
                        await self.traceRemoteIceBatchReceived(batch)
                    }
                    if !batch.candidates.isEmpty {
                        try await peer.addRemoteCandidates(batch.candidates)
                        await self.traceRemoteIceBatchApplied(batch)
                    }
                    if batch.endOfCandidates {
                        if !didTraceRemoteIceCompleted {
                            didTraceRemoteIceCompleted = true
                            await self.traceRemoteIceCompleted()
                        }
                        try await Task.sleep(for: .seconds(1))
                    }
                } catch is CancellationError {
                    return
                } catch {
                    guard !Task.isCancelled, let self, await self.isCurrent(generation),
                        await self.activePeerEpoch == peerEpoch
                    else { return }
                    await self.fail(reason: .sessionFailed, generation: generation, error: error)
                    return
                }
            }
        }
    }

    private func startCredentialUpdates(
        control: any StreamingControlSession, generation: UInt64, sink: @escaping AuthSink
    ) {
        credentialUpdateTask?.cancel()
        credentialUpdateTask = Task { [weak self] in
            while !Task.isCancelled {
                guard let self, await self.isCurrent(generation) else { return }
                if let update = await control.pendingAuthUpdate() {
                    guard !Task.isCancelled, await self.isCurrent(generation) else { return }
                    do {
                        let result = try await sink(generation, update)
                        guard !Task.isCancelled, await self.isCurrent(generation) else { return }
                        switch result {
                        case .committed:
                            await control.acknowledgeAuthUpdate(revision: update.revision)
                        case .synchronize(let session):
                            try await control.synchronizeAuthSession(
                                revision: update.revision, session: session)
                        }
                        await self.trace(
                            "credentialUpdateCommitted",
                            payload: ["revision": .integer(Int64(clamping: update.revision))],
                            dimension: .lifecycle)
                    } catch {
                        guard !Task.isCancelled, await self.isCurrent(generation) else { return }
                        if AuthStoreError.isStale(error) {
                            await self.fail(
                                reason: .authRevoked, generation: generation, error: error)
                            return
                        }
                        await self.trace(
                            "credentialUpdateDeferred",
                            payload: ["revision": .integer(Int64(clamping: update.revision))],
                            dimension: .lifecycle)
                    }
                }
                do { try await Task.sleep(for: .seconds(1)) } catch { return }
            }
        }
    }

    private func recoverDisconnectedPeer(generation: UInt64, peerEpoch: UInt64) async {
        guard isCurrent(generation), !reconnectInFlight,
            peerEpoch == activePeerEpoch,
            let peer = peerRuntime,
            let control = controlSession,
            var prepared = preparedSignaling
        else { return }
        reconnectInFlight = true
        initialConnectionTask?.cancel()
        initialConnectionTask = nil
        let useFallback =
            !hasEverConnected && !fallbackRetryConsumed && prepared.fallbackIceServers != nil
        var admission = StreamingSupervisionSample(nowMs: Self.nowMs)
        admission.peerGeneration = peerEpoch
        admission.videoGeneration = videoGeneration
        admission.visible = visible
        admission.requestConnectionRecovery = true
        admission.connectionRecoveryActionCompleted = connectionRecoveryAwaitingFrames
        admission.connectionState = peerConnectionState.rawValue
        admission.connectionRecoveryFactKey =
            useFallback
            ? "directPathExhausted"
            : (peerConnectionState == .failed || peerConnectionState == .closed)
                ? "transportConnectionState:\(peerConnectionState.rawValue)" : "mediaStalled"
        do {
            let decision = try await control.supervise(admission)
            guard isCurrent(generation), peerEpoch == activePeerEpoch else { return }
            guard decision.connectionRecoveryAllowed else {
                reconnectInFlight = false
                if decision.connectionRecoveryExhausted {
                    await fail(
                        reason: .iceFailed, generation: generation,
                        error: StreamingRuntimeError.peerConnectionFailed)
                }
                return
            }
        } catch {
            reconnectInFlight = false
            await fail(reason: .sessionFailed, generation: generation, error: error)
            return
        }
        if connectionRecoveryDeadlineTask == nil {
            connectionRecoveryDeadlineTask = Task { [weak self] in
                do { try await Task.sleep(for: .seconds(120)) } catch { return }
                guard let self, await self.isCurrent(generation) else { return }
                await self.fail(
                    reason: .iceFailed, generation: generation,
                    error: StreamingRuntimeError.peerConnectionFailed)
            }
        }
        let owner: UUID
        do { owner = try await acquireNegotiation(generation: generation) } catch {
            reconnectInFlight = false
            return
        }
        defer { releaseNegotiation(owner) }
        guard isCurrent(generation), peerEpoch == activePeerEpoch else { return }
        if useFallback, let servers = prepared.fallbackIceServers {
            fallbackRetryConsumed = true
            prepared.iceServers = servers
            prepared.fallbackIceServers = nil
            preparedSignaling = prepared
            await publish(.recovering, generation: generation)
            trace(
                "fallbackTurnRetry", payload: ["directPathExhausted": .bool(true)],
                dimension: .recovery)
            await rebuildPeerRuntime(
                replacing: peer, control: control, prepared: prepared,
                generation: generation, previousPeerEpoch: peerEpoch,
                restartError: StreamingRuntimeError.peerConnectionFailed, reserveFollowup: false)
            armInitialConnectionDeadline(generation: generation)
            return
        }
        pendingDisconnectedPeerEpoch = nil
        playbackReadiness.remove(.peerConnected)
        localIceState.remoteDescriptionApplied = false
        pendingConnectedPeerEpoch = nil
        remoteIceApplicationTask?.cancel()
        remoteIceApplicationTask = nil
        await publish(.recovering, generation: generation)
        let restartOperationID = UUID().uuidString
        trace(
            "iceRestartStarted",
            dimension: .recovery,
            operationID: restartOperationID
        )

        do {
            guard
                let localIceEpoch = await beginLocalIceNegotiation(
                    generation: generation,
                    peerEpoch: peerEpoch
                )
            else {
                throw StreamingRuntimeError.superseded
            }
            let offer = try await peer.makeOffer(configuration: prepared, iceRestart: true)
            try ensureCurrent(generation)
            let answer = try await control.exchangeOffer(offer)
            try ensureCurrent(generation)
            guard !answer.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
                throw StreamingRuntimeError.missingRemoteDescription
            }
            try await applyRemoteAnswer(
                answer,
                to: peer,
                phase: "iceRestart",
                operationID: restartOperationID
            )
            try ensureCurrent(generation)
            trace(
                "answerApplied",
                payload: ["iceRestart": .bool(true)],
                dimension: .network,
                operationID: restartOperationID
            )
            trace(
                "localIceStarted",
                payload: ["iceRestart": .bool(true)],
                dimension: .network,
                operationID: restartOperationID
            )
            localIceState.remoteDescriptionApplied = true
            try await flushLocalIceAndWait(generation: generation, epoch: localIceEpoch)
            try ensureCurrent(generation)
            reconnectInFlight = false
            connectionRecoveryAwaitingFrames = true
            await publish(connectionStateAfterNegotiation, generation: generation)
            await completePendingPeerConnection(generation: generation, peerEpoch: peerEpoch)
            startRemoteIceApplication(generation: generation, peerEpoch: peerEpoch)
        } catch {
            guard isCurrent(generation), peerEpoch == activePeerEpoch else { return }
            trace(
                "iceRestartFailed",
                payload: ["reason": .string("offerOrAnswerFailed")],
                dimension: .recovery,
                operationID: restartOperationID
            )
            await rebuildPeerRuntime(
                replacing: peer,
                control: control,
                prepared: prepared,
                generation: generation,
                previousPeerEpoch: peerEpoch,
                restartError: error
            )
        }
    }

    private func rebuildPeerRuntime(
        replacing peer: any StreamingPeerRuntime,
        control: any StreamingControlSession,
        prepared: StreamingPreparedSignaling,
        generation: UInt64,
        previousPeerEpoch: UInt64,
        restartError _: Error,
        reserveFollowup: Bool = true
    ) async {
        guard isCurrent(generation), previousPeerEpoch == activePeerEpoch else { return }
        if reserveFollowup {
            var followup = StreamingSupervisionSample(nowMs: Self.nowMs)
            followup.peerGeneration = previousPeerEpoch
            followup.visible = visible
            followup.requestConnectionRecovery = true
            followup.connectionRecoveryFollowup = true
            do {
                let decision = try await control.supervise(followup)
                guard isCurrent(generation), previousPeerEpoch == activePeerEpoch else { return }
                guard decision.connectionRecoveryAllowed else {
                    reconnectInFlight = false
                    await fail(
                        reason: .iceFailed, generation: generation,
                        error: StreamingRuntimeError.peerConnectionFailed)
                    return
                }
            } catch {
                reconnectInFlight = false
                await fail(reason: .sessionFailed, generation: generation, error: error)
                return
            }
        }
        let rebuildOperationID = UUID().uuidString
        trace(
            "runtimeRebuildStarted",
            payload: ["previousPeerEpoch": .integer(Int64(previousPeerEpoch))],
            dimension: .recovery,
            operationID: rebuildOperationID
        )

        activePeerEpoch = 0
        peerConnectionState = .new
        qualityLevel = 0
        latestDecision = .unavailable
        qualityCompletion = nil
        latestPeerStats = nil
        rtcTraceWindow = StreamingTraceCounterWindow()
        displayTraceWindow = StreamingTraceCounterWindow()
        latestDisplayHealthSnapshot = nil
        microphonePeerEpoch = 0
        await microphoneSink?(generation, false, microphoneRequested)
        peerRuntime = nil
        activeLocalIceEpoch = nil
        await cancelAndAwaitLocalIceFlush()
        localIceState.reset()
        pendingLocalCandidates.removeAll(keepingCapacity: true)
        dataChannelProfilesPendingTrace = false
        pendingConnectedPeerEpoch = nil
        pendingDisconnectedPeerEpoch = nil
        retiringPeerRuntime = peer
        await videoSink?(generation, nil)
        let peerStopTask = Task {
            await peer.stopInputAndHaptics()
        }
        retiringPeerStopTask = peerStopTask
        await peerStopTask.value
        guard isRetiring(peer) else { return }
        await peer.closeTransport()
        guard isRetiring(peer) else { return }
        retiringPeerRuntime = nil
        retiringPeerStopTask = nil
        guard isCurrent(generation) else { return }

        if !reserveFollowup {
            do { try await Task.sleep(for: .milliseconds(120)) } catch { return }
            guard isCurrent(generation) else { return }
        }

        var replacementPeerEpoch: UInt64 = 0
        do {
            let replacement = try await makePeerRuntime(generation: generation)
            replacementPeerEpoch = activePeerEpoch
            guard
                let localIceEpoch = await beginLocalIceNegotiation(
                    generation: generation,
                    peerEpoch: replacementPeerEpoch
                )
            else {
                throw StreamingRuntimeError.superseded
            }
            trace(
                "runtimeRebuildOfferStarted",
                payload: ["previousPeerEpoch": .integer(Int64(previousPeerEpoch))],
                dimension: .recovery,
                operationID: rebuildOperationID
            )
            let offer = try await replacement.makeOffer(configuration: prepared, iceRestart: false)
            try ensureCurrent(generation)
            let answer = try await control.exchangeOffer(offer)
            try ensureCurrent(generation)
            guard !answer.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
                throw StreamingRuntimeError.missingRemoteDescription
            }
            try await applyRemoteAnswer(
                answer,
                to: replacement,
                phase: "runtimeRebuild",
                operationID: rebuildOperationID
            )
            try ensureCurrent(generation)
            guard replacementPeerEpoch == activePeerEpoch else {
                throw StreamingRuntimeError.superseded
            }
            trace(
                "answerApplied",
                payload: ["runtimeRebuild": .bool(true)],
                dimension: .network,
                operationID: rebuildOperationID
            )
            trace(
                "localIceStarted",
                payload: ["runtimeRebuild": .bool(true)],
                dimension: .network,
                operationID: rebuildOperationID
            )
            localIceState.remoteDescriptionApplied = true
            try await flushLocalIceAndWait(generation: generation, epoch: localIceEpoch)
            try ensureCurrent(generation)
            reconnectInFlight = false
            connectionRecoveryAwaitingFrames = true
            await publish(connectionStateAfterNegotiation, generation: generation)
            await completePendingPeerConnection(
                generation: generation,
                peerEpoch: replacementPeerEpoch
            )
            startRemoteIceApplication(
                generation: generation,
                peerEpoch: replacementPeerEpoch
            )
            trace(
                "runtimeRebuildNegotiated",
                payload: ["previousPeerEpoch": .integer(Int64(previousPeerEpoch))],
                dimension: .recovery,
                operationID: rebuildOperationID
            )
        } catch {
            guard isCurrent(generation), replacementPeerEpoch == activePeerEpoch else { return }
            reconnectInFlight = false
            trace(
                "runtimeRebuildFailed",
                payload: ["reason": .string("offerOrAnswerFailed")],
                dimension: .recovery,
                operationID: rebuildOperationID
            )
            await fail(reason: .iceFailed, generation: generation, error: error)
        }
    }

    private func beginLocalIceNegotiation(
        generation: UInt64,
        peerEpoch: UInt64
    ) async -> LocalIceEpoch? {
        await cancelAndAwaitLocalIceFlush()
        guard isCurrent(generation), peerEpoch == activePeerEpoch else { return nil }
        nextNegotiationEpoch &+= 1
        let epoch = LocalIceEpoch(
            peerEpoch: peerEpoch,
            negotiationEpoch: nextNegotiationEpoch
        )
        activeLocalIceEpoch = epoch
        localIceState.gatheringComplete = false
        localIceState.completionSent = false
        localIceState.traceCompleted = false
        pendingLocalCandidates.removeAll(keepingCapacity: true)
        return epoch
    }

    private func scheduleLocalIceFlush(generation: UInt64, epoch: LocalIceEpoch) {
        guard localIceState.remoteDescriptionApplied, localIceFlushTask == nil,
            activeLocalIceEpoch == epoch
        else { return }
        let (taskID, task) = installLocalIceFlushTask(
            generation: generation,
            epoch: epoch,
            delay: .milliseconds(60)
        )
        Task { [weak self] in
            do {
                try await task.value
                await self?.finishScheduledLocalIceFlush(
                    taskID: taskID,
                    generation: generation,
                    epoch: epoch,
                    error: nil
                )
            } catch is CancellationError {
                await self?.clearLocalIceFlushTask(taskID: taskID)
            } catch {
                await self?.finishScheduledLocalIceFlush(
                    taskID: taskID,
                    generation: generation,
                    epoch: epoch,
                    error: error
                )
            }
        }
    }

    private func flushLocalIceAndWait(
        generation: UInt64,
        epoch: LocalIceEpoch
    ) async throws {
        await cancelAndAwaitLocalIceFlush()
        guard isCurrent(generation), activeLocalIceEpoch == epoch else {
            throw StreamingRuntimeError.superseded
        }
        let (taskID, task) = installLocalIceFlushTask(
            generation: generation,
            epoch: epoch,
            delay: nil
        )
        do {
            try await task.value
            clearLocalIceFlushTask(taskID: taskID)
        } catch is CancellationError where isCurrent(generation) {
            clearLocalIceFlushTask(taskID: taskID)
            throw LocalIceFlushError.negotiationSuperseded
        } catch {
            clearLocalIceFlushTask(taskID: taskID)
            throw error
        }
    }

    private func installLocalIceFlushTask(
        generation: UInt64,
        epoch: LocalIceEpoch,
        delay: Duration?
    ) -> (UInt64, Task<Void, Error>) {
        nextLocalIceFlushTaskID &+= 1
        let taskID = nextLocalIceFlushTaskID
        let task = Task { [weak self] in
            if let delay {
                try await Task.sleep(for: delay)
            }
            try Task.checkCancellation()
            try await self?.performLocalIceFlush(generation: generation, epoch: epoch)
        }
        activeLocalIceFlushTaskID = taskID
        localIceFlushTask = task
        return (taskID, task)
    }

    private func performLocalIceFlush(
        generation: UInt64,
        epoch: LocalIceEpoch
    ) async throws {
        guard isCurrent(generation), localIceState.remoteDescriptionApplied,
            activeLocalIceEpoch == epoch, !localIceState.flushInFlight
        else { return }
        localIceState.flushInFlight = true
        defer {
            if isCurrent(generation), activeLocalIceEpoch == epoch {
                localIceState.flushInFlight = false
            }
        }

        while isCurrent(generation), activeLocalIceEpoch == epoch {
            try Task.checkCancellation()
            let candidates = pendingLocalCandidates
            pendingLocalCandidates.removeAll(keepingCapacity: true)
            if !candidates.isEmpty {
                try await controlSession?.submitLocalCandidates(candidates)
                try Task.checkCancellation()
                guard isCurrent(generation), activeLocalIceEpoch == epoch else {
                    throw StreamingRuntimeError.superseded
                }
                continue
            }
            if localIceState.gatheringComplete, !localIceState.completionSent {
                localIceState.completionSent = true
                do {
                    try await controlSession?.completeLocalIceGathering()
                    try Task.checkCancellation()
                    guard isCurrent(generation), activeLocalIceEpoch == epoch else {
                        throw StreamingRuntimeError.superseded
                    }
                    if !localIceState.traceCompleted {
                        localIceState.traceCompleted = true
                        trace("localIceCompleted", dimension: .network)
                    }
                } catch {
                    if isCurrent(generation) {
                        localIceState.completionSent = false
                    }
                    throw error
                }
            }
            return
        }
    }

    private func finishScheduledLocalIceFlush(
        taskID: UInt64,
        generation: UInt64,
        epoch: LocalIceEpoch,
        error: Error?
    ) async {
        guard taskID == activeLocalIceFlushTaskID else { return }
        localIceFlushTask = nil
        localIceState.flushInFlight = false
        guard isCurrent(generation), activeLocalIceEpoch == epoch else { return }
        if let error {
            await fail(reason: .sessionFailed, generation: generation, error: error)
        } else if !pendingLocalCandidates.isEmpty {
            scheduleLocalIceFlush(generation: generation, epoch: epoch)
        }
    }

    private func clearLocalIceFlushTask(taskID: UInt64) {
        guard taskID == activeLocalIceFlushTaskID else { return }
        localIceFlushTask = nil
        localIceState.flushInFlight = false
    }

    private func cancelAndAwaitLocalIceFlush() async {
        guard let task = localIceFlushTask else {
            localIceState.flushInFlight = false
            return
        }
        let taskID = activeLocalIceFlushTaskID
        task.cancel()
        _ = try? await task.value
        clearLocalIceFlushTask(taskID: taskID)
    }

    private func isRetiring(_ peer: any StreamingPeerRuntime) -> Bool {
        guard let retiringPeerRuntime else { return false }
        return retiringPeerRuntime === peer
    }

    private func applyRemoteAnswer(
        _ answer: String,
        to peer: any StreamingPeerRuntime,
        phase: String,
        operationID: String? = nil
    ) async throws {
        let startedAt = Date()
        trace(
            "remoteAnswerApplyStarted",
            payload: [
                "phase": .string(phase),
                "answerBytes": .integer(Int64(answer.utf8.count)),
                "timeoutMs": .integer(remoteAnswerApplyTimeout.milliseconds),
            ],
            dimension: .network,
            importance: .key,
            operationID: operationID
        )

        do {
            try await applyRemoteAnswerWithTimeout(answer, to: peer)
        } catch {
            let timedOut: Bool
            if case .remoteDescriptionTimedOut = error as? StreamingRuntimeError {
                timedOut = true
            } else {
                timedOut = false
            }
            let peerSnapshot = await peer.debugSnapshot()
            let nsError = error as NSError
            trace(
                timedOut ? "remoteAnswerApplyTimedOut" : "remoteAnswerApplyFailed",
                payload: [
                    "phase": .string(phase),
                    "elapsedMs": .integer(Int64(Date().timeIntervalSince(startedAt) * 1_000)),
                    "reason": .string(timedOut ? "timeout" : "applyError"),
                    "errorType": .string(String(describing: type(of: error))),
                    "errorDomain": .string(nsError.domain),
                    "errorCode": .integer(Int64(nsError.code)),
                    "peerSnapshot": peerDebugSnapshotValue(peerSnapshot),
                ],
                dimension: .network,
                importance: .key,
                operationID: operationID
            )
            throw error
        }

        trace(
            "remoteAnswerApplyCompleted",
            payload: [
                "phase": .string(phase),
                "elapsedMs": .integer(Int64(Date().timeIntervalSince(startedAt) * 1_000)),
            ],
            dimension: .network,
            importance: .key,
            operationID: operationID
        )
    }

    private func peerDebugSnapshotValue(
        _ snapshot: StreamingPeerDebugSnapshot
    ) -> IOSRuntimeTraceValue {
        .object([
            "signalingState": .string(snapshot.signalingState),
            "iceConnectionState": .string(snapshot.iceConnectionState),
            "iceGatheringState": .string(snapshot.iceGatheringState),
            "transceiverCount": .integer(Int64(snapshot.transceiverCount)),
            "audioReceiverTrackCount": .integer(Int64(snapshot.audioReceiverTrackCount)),
            "videoReceiverTrackCount": .integer(Int64(snapshot.videoReceiverTrackCount)),
            "localDescriptionSet": .bool(snapshot.localDescriptionSet),
            "remoteDescriptionSet": .bool(snapshot.remoteDescriptionSet),
            "dataChannels": snapshot.dataChannels.map { dataChannels in
                .object([
                    "readyStates": .object(
                        dataChannels.readyStates.mapValues(IOSRuntimeTraceValue.string)
                    ),
                    "phases": .object(
                        dataChannels.phases.mapValues(IOSRuntimeTraceValue.string)
                    ),
                    "handshakeAcknowledged": .bool(dataChannels.handshakeAcknowledged),
                    "controlReady": .bool(dataChannels.controlReady),
                    "inputStarted": .bool(dataChannels.inputStarted),
                    "terminalReason": dataChannels.terminalReason.map(
                        IOSRuntimeTraceValue.string
                    ) ?? .null,
                ])
            } ?? .null,
        ])
    }

    private func applyRemoteAnswerWithTimeout(
        _ answer: String,
        to peer: any StreamingPeerRuntime
    ) async throws {
        let gate = RemoteAnswerApplyGate()
        let timeout = remoteAnswerApplyTimeout
        try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { continuation in
                gate.install(continuation)
                let operationTask = Task {
                    do {
                        try await peer.applyAnswer(answer)
                        gate.resolve(.success(()))
                    } catch {
                        gate.resolve(.failure(error))
                    }
                }
                let timeoutTask = Task {
                    do {
                        try await Task.sleep(for: timeout)
                    } catch {
                        return
                    }
                    gate.resolve(.failure(StreamingRuntimeError.remoteDescriptionTimedOut))
                }
                gate.installTasks(operationTask: operationTask, timeoutTask: timeoutTask)
            }
        } onCancel: {
            gate.resolve(.failure(CancellationError()))
        }
    }

    private func startDisplayRecovery(
        _ action: StreamingDisplayRecoveryAction, observation: StreamingDisplayObservation,
        source: StreamingPresentationSession, thresholdMs: UInt64, generation: UInt64,
        peerEpoch: UInt64
    ) {
        guard displayResetActionID == nil, visible, isCurrent(generation),
            activePeerEpoch == peerEpoch,
            let current = source.snapshot,
            current.scope.renderEpoch == action.renderEpoch,
            current.scope.surfaceRevision == action.surfaceRevision,
            current.scope.presentation?.videoGeneration == videoGeneration,
            current.active, current.surfaceReady,
            current.health.displayedCount == Int64(clamping: observation.displayed),
            current.health.lastDisplayedAtMs.map({ Double(Self.nowMs) - $0 >= Double(thresholdMs) })
                == true
        else {
            displayRecoveryCompletion = .init(
                id: action.id, result: "superseded", completedAtMs: Self.nowMs,
                renderEpoch: source.snapshot?.scope.renderEpoch ?? action.renderEpoch)
            return
        }
        displayResetActionID = action.id
        displayResetScope = current.scope
        trace(
            "displayRecoverySelected",
            payload: [
                "actionId": .integer(Int64(clamping: action.id)),
                "renderEpoch": .integer(Int64(clamping: action.renderEpoch)),
                "reason": .string(action.reason),
            ], dimension: .recovery)
        displayResetTask = Task { [weak self] in
            let outcome = await source.reset(expected: current.scope, actionID: action.id)
            await self?.displayResetCompleted(
                actionID: action.id, outcome: outcome, timedOut: false,
                source: source, generation: generation, peerEpoch: peerEpoch)
        }
        displayResetDeadlineTask = Task { [weak self] in
            do { try await Task.sleep(for: .milliseconds(1_500)) } catch { return }
            source.cancelReset(action.id)
            await self?.displayResetCompleted(
                actionID: action.id, outcome: .failed, timedOut: true,
                source: source, generation: generation, peerEpoch: peerEpoch)
        }
    }

    private func displayResetCompleted(
        actionID: UInt64, outcome: StreamingPresentationResetOutcome, timedOut: Bool,
        source: StreamingPresentationSession, generation: UInt64, peerEpoch: UInt64
    ) async {
        guard displayResetActionID == actionID, isCurrent(generation), activePeerEpoch == peerEpoch
        else { return }
        displayResetActionID = nil
        displayResetTask = nil
        displayResetDeadlineTask?.cancel()
        displayResetDeadlineTask = nil
        let current = source.snapshot
        let acknowledged = outcome == .acknowledged
        let superseded =
            outcome == .superseded || !visible || current?.surfaceReady != true
            || current?.active != true
            || current?.scope.presentation != displayResetScope?.presentation
            || current?.scope.surfaceRevision != displayResetScope?.surfaceRevision
        let result =
            superseded
            ? "superseded" : acknowledged ? "acknowledged" : timedOut ? "timedOut" : "failed"
        displayRecoveryCompletion = .init(
            id: actionID, result: result, completedAtMs: Self.nowMs,
            renderEpoch: source.snapshot?.scope.renderEpoch ?? 0)
        trace(
            "displayRecoveryActionCompleted",
            payload: [
                "actionId": .integer(Int64(clamping: actionID)),
                "result": .string(result),
            ], dimension: .recovery)
        if !acknowledged, !superseded {
            await fail(
                reason: .runtimeFailure, generation: generation,
                error: NSError(
                    domain: "XBXRC.DisplayRecovery", code: timedOut ? 2 : 1,
                    userInfo: [NSLocalizedDescriptionKey: "画面呈现恢复失败，请重新连接"]))
        }
    }

    private func cancelDisplayRecovery() {
        if let actionID = displayResetActionID {
            currentVideoHandle?.presentation.cancelReset(actionID)
        }
        displayResetTask?.cancel()
        displayResetTask = nil
        displayResetDeadlineTask?.cancel()
        displayResetDeadlineTask = nil
        displayResetActionID = nil
        displayResetScope = nil
    }

    private func fail(
        reason: StreamingTerminalReason,
        generation: UInt64,
        error: Error
    ) async {
        let retryable = StreamingStartupDiagnostics.isRetryable(error, fallback: reason.retryable)
        guard isCurrent(generation),
            selectTerminal(reason, generation: generation, retryable: retryable)
        else { return }
        var failure = StreamingStartupDiagnostics.errorPayload(error)
        failure["reason"] = .string(reason.code)
        failure["hasControlSession"] = .bool(controlSession != nil)
        failure["hasPreparedSignaling"] = .bool(preparedSignaling != nil)
        failure["hasPeerRuntime"] = .bool(peerRuntime != nil)
        if let latestStartupProgress {
            failure["startupProgress"] = .object(
                StreamingStartupDiagnostics.progressPayload(latestStartupProgress))
        }
        trace(
            "sessionFailure",
            payload: failure,
            dimension: .lifecycle,
            importance: .essential
        )
        let message: String
        switch reason {
        case .sessionFailed, .runtimeFailure:
            message = StreamingStartupDiagnostics.userMessage(error, fallback: reason.userMessage)
        default:
            message = reason.userMessage
        }
        await stopActiveSession(
            reason: reason,
            publishIdle: false,
            terminalState: .failed(
                message: message,
                retryable: retryable
            )
        )
    }

    private func stopActiveSession(
        reason: StreamingTerminalReason,
        publishIdle: Bool,
        terminalState: StreamingFeatureState? = nil
    ) async {
        supervisionTask?.cancel()
        cancelDisplayRecovery()
        displayRecoveryCompletion = nil
        initialNegotiationInFlight = false
        initialConnectionTask?.cancel()
        initialConnectionTask = nil
        hasEverConnected = false
        fallbackRetryConsumed = false
        supervisionTask = nil
        firstFrameGuardTask?.cancel()
        firstFrameGuardTask = nil
        connectionRecoveryDeadlineTask?.cancel()
        connectionRecoveryDeadlineTask = nil
        recoveryProbe = nil
        lastLocalActionAtMs = nil
        keyframeBudget = 2
        playbackBudget = 1
        qualityCompletion = nil
        qualityOperationInFlight = false
        currentVideoHandle = nil
        connectionRecoveryAwaitingFrames = false
        peerConnectionState = .new
        qualityLevel = 0
        latestDecision = .unavailable
        microphonePeerEpoch = 0
        supervisionEligibleSinceMs = 0
        let generation = activeGeneration
        if let generation { _ = selectTerminal(reason, generation: generation) }
        let attemptID = activeAttemptID
        let remoteIceTask = remoteIceApplicationTask
        credentialUpdateTask?.cancel()
        credentialUpdateTask = nil
        microphoneTask?.cancel()
        microphoneTask = nil
        microphoneRevision &+= 1
        microphoneRequested = false
        negotiationOwner = nil
        visible = true
        visibilityRevision &+= 1
        resumedAtMilliseconds = nil
        let localIceTask = localIceFlushTask
        let peer = peerRuntime
        let retiringPeer = retiringPeerRuntime
        let retiringStopTask = retiringPeerStopTask
        let control = controlSession
        let accessHandle = activeAccessHandle
        let previousStateSink = stateSink
        let previousVideoSink = videoSink
        peerRuntime = nil
        retiringPeerRuntime = nil
        retiringPeerStopTask = nil
        preparedSignaling = nil
        controlSession = nil
        activeAttemptID = nil
        activeAccessHandle = nil
        activeGeneration = nil
        stateSink = nil
        lastPublishedState = nil
        videoSink = nil
        diagnosticsSink = nil
        playbackReadiness.resetForPeer()
        localIceState.reset()
        reconnectInFlight = false
        pendingLocalCandidates.removeAll(keepingCapacity: true)
        interactionEventCounts.removeAll(keepingCapacity: true)
        dataChannelProfilesPendingTrace = false
        activePeerEpoch = 0
        activeLocalIceEpoch = nil
        pendingConnectedPeerEpoch = nil
        pendingDisconnectedPeerEpoch = nil
        latestDisplayHealthSnapshot = nil
        latestPeerStats = nil
        rtcTraceWindow = StreamingTraceCounterWindow()
        displayTraceWindow = StreamingTraceCounterWindow()
        latestAudioLifecycleState = nil
        latestAudioRoute = nil
        latestAudioOutput = nil
        audioTrackReady = false
        if let generation {
            // 先移交并解除会话归属，再发布终态，保证重入的协商回调沿旧代次退出。
            if let terminalState { await previousStateSink?(generation, terminalState) }
            await previousVideoSink?(generation, nil)
            traceCleanup(
                StreamingCleanupStage.presentationDetached.rawValue,
                attemptID: attemptID,
                generation: generation
            )
        }
        if let peer {
            await peer.stopInputAndHaptics()
        }
        if let retiringStopTask {
            await retiringStopTask.value
        } else if let retiringPeer {
            await retiringPeer.stopInputAndHaptics()
        }
        if peer != nil || retiringPeer != nil {
            traceCleanup(
                StreamingCleanupStage.inputStopped.rawValue,
                attemptID: attemptID,
                generation: generation
            )
            traceCleanup(
                StreamingCleanupStage.hapticsStopped.rawValue,
                attemptID: attemptID,
                generation: generation
            )
        }
        remoteIceTask?.cancel()
        localIceTask?.cancel()
        _ = try? await localIceTask?.value
        traceCleanup(
            StreamingCleanupStage.iceTasksCancelled.rawValue,
            attemptID: attemptID,
            generation: generation
        )
        remoteIceApplicationTask = nil
        localIceFlushTask = nil
        activeLocalIceFlushTaskID = 0
        if let peer {
            await peer.closeTransport()
        }
        if let retiringPeer {
            await retiringPeer.closeTransport()
        }
        if peer != nil || retiringPeer != nil {
            traceCleanup(
                StreamingCleanupStage.peerClosed.rawValue,
                attemptID: attemptID,
                generation: generation
            )
        }
        if let control {
            await control.close()
            traceCleanup(
                StreamingCleanupStage.remoteSessionClosed.rawValue,
                attemptID: attemptID,
                generation: generation
            )
        }
        if let accessHandle {
            await controlFactory.releaseAccess(handle: accessHandle)
            traceCleanup(
                StreamingCleanupStage.accessReleased.rawValue,
                attemptID: attemptID,
                generation: generation
            )
        }
        if let generation, publishIdle { await previousStateSink?(generation, .idle) }
    }

    @discardableResult
    private func selectTerminal(
        _ reason: StreamingTerminalReason,
        generation: UInt64,
        retryable: Bool? = nil
    ) -> Bool {
        guard isCurrent(generation), selectedTerminalReason == nil else { return false }
        selectedTerminalReason = reason
        trace(
            "sessionTerminal",
            payload: ["reason": .string(reason.code)],
            dimension: .lifecycle,
            importance: .key
        )
        trace(
            "terminalSelected",
            payload: [
                "reason": .string(reason.code),
                "retryable": .bool(retryable ?? reason.retryable),
            ],
            dimension: .lifecycle,
            importance: .essential
        )
        return true
    }

    private func traceDataChannelEvent(label: String, event: String) async {
        if let countedEvent = countedInteractionEvent(label: label, event: event) {
            traceCountedInteractionEvent(countedEvent)
            return
        }
        if label == "all", event == "profilesCreated" {
            dataChannelProfilesPendingTrace = true
            if playbackReadiness.contains(.peerConnected) {
                dataChannelProfilesPendingTrace = false
                traceDataChannelProfiles()
            }
            return
        }
        if label == "control", event == "ready" {
            guard playbackReadiness.mark(.controlReady) else { return }
            trace(
                "controlReady",
                payload: ["channel": .string(label)],
                dimension: .network
            )
            if let generation = activeGeneration,
                playbackReady
            {
                await publish(.playing, generation: generation)
            }
            return
        }
        let canonicalEvent: String? =
            switch (label, event) {
            case ("message", "handshakeSent"): "messageHandshakeSent"
            case ("message", "handshakeAcked"): "messageHandshakeAcked"
            case ("message", "postHandshakeCompleted"): "messagePostHandshakeCompleted"
            case ("control", "bootstrapPreHandshakeSent"):
                "controlBootstrapPreHandshakeCompleted"
            case ("control", "bootstrapSent"): "controlBootstrapCompleted"
            case ("input", "metadataPreHandshakeSent"):
                "inputBootstrapPreHandshakeCompleted"
            case ("input", "metadataBootstrapSent"): "inputBootstrapCompleted"
            default: nil
            }
        guard let canonicalEvent else { return }
        trace(
            canonicalEvent,
            payload: ["channel": .string(label)],
            dimension: .network
        )
    }

    private func traceRemoteIceBatchReceived(_ batch: StreamingRemoteIceBatch) {
        trace(
            "remoteIceBatchReceived",
            payload: [
                "candidateCount": .integer(Int64(batch.candidates.count)),
                "endOfCandidates": .bool(batch.endOfCandidates),
                "configurationPending": .bool(batch.configurationPending),
                "configurationCandidateCount": .integer(
                    Int64(batch.candidates.filter { $0.source == "session-configuration" }.count)),
                "consoleListCandidateCount": .integer(
                    Int64(batch.candidates.filter { $0.source == "console-list" }.count)),
            ],
            dimension: .network
        )
    }

    private func traceRemoteIceBatchApplied(_ batch: StreamingRemoteIceBatch) {
        trace(
            "remoteIceBatchApplied",
            payload: ["candidateCount": .integer(Int64(batch.candidates.count))],
            dimension: .network
        )
    }

    private func traceRemoteIceCompleted() {
        trace("remoteIceCompleted", dimension: .network)
    }

    private func traceDataChannelProfiles() {
        let profiles = streamDataChannelProfiles().map { profile in
            IOSRuntimeTraceValue.object([
                "label": .string(profile.label),
                "protocol": .string(profile.protocolName),
                "ordered": .bool(profile.ordered),
            ])
        }
        trace(
            "dataChannelProfilesCreated",
            payload: [
                "channelCount": .integer(Int64(profiles.count)),
                "profiles": .array(profiles),
            ],
            dimension: .network
        )
    }

    private func countedInteractionEvent(label: String, event: String) -> String? {
        switch (label, event) {
        case ("input", "frameSent"), ("input", "neutralFrameSent"):
            "inputFrameSent"
        case ("input", "backpressureDrop"):
            "inputBackpressureDrop"
        case ("input", "rumbleParsed"):
            "rumbleParsed"
        case ("input", "hapticsApplied"):
            "hapticsApplied"
        case ("input", "hapticsDegraded"):
            "hapticsDegraded"
        case ("input", "hapticsUnsupported"):
            "hapticsUnsupported"
        default:
            nil
        }
    }

    private func traceCountedInteractionEvent(_ event: String) {
        let count = interactionEventCounts[event, default: 0] + 1
        interactionEventCounts[event] = count
        let sampleInterval: Int64 = event == "inputFrameSent" ? 64 : 16
        guard count == 1 || count.isMultiple(of: sampleInterval) else { return }
        trace(
            event,
            payload: [
                "count": .integer(count),
                "summary": .bool(true),
            ],
            dimension: .input,
            importance: .key
        )
    }

    private func trace(
        _ event: String,
        payload: [String: IOSRuntimeTraceValue] = [:],
        dimension: IOSRuntimeTraceDimension,
        importance: IOSRuntimeTraceImportance = .key,
        operationID: String? = nil
    ) {
        var contextualPayload = payload
        if activePeerEpoch > 0 {
            contextualPayload["peerEpoch"] = .integer(Int64(activePeerEpoch))
        }
        traceAttempt(
            event,
            attemptID: activeAttemptID,
            generation: activeGeneration,
            payload: contextualPayload,
            dimension: dimension,
            importance: importance,
            operationID: operationID ?? activeAttemptID
        )
    }

    private func traceSnapshot(
        _ event: String,
        payload: [String: IOSRuntimeTraceValue],
        dimension: IOSRuntimeTraceDimension,
        importance: IOSRuntimeTraceImportance = .key
    ) {
        guard let attemptID = activeAttemptID, let generation = activeGeneration else { return }
        var contextualPayload = payload
        contextualPayload["attemptId"] = .string(attemptID)
        contextualPayload["generation"] = .integer(Int64(generation))
        if activePeerEpoch > 0 {
            contextualPayload["peerEpoch"] = .integer(Int64(activePeerEpoch))
        }
        IOSRuntimeTrace.snapshot(
            domain: "ios-streaming",
            event: event,
            payload: contextualPayload,
            dimension: dimension,
            importance: importance,
            operationID: attemptID
        )
    }

    private func traceCleanup(
        _ event: String,
        attemptID: String?,
        generation: UInt64?
    ) {
        traceAttempt(
            event,
            attemptID: attemptID,
            generation: generation,
            payload: [:],
            dimension: .lifecycle,
            importance: .key,
            operationID: attemptID
        )
    }

    private func traceAttempt(
        _ event: String,
        attemptID: String?,
        generation: UInt64?,
        payload: [String: IOSRuntimeTraceValue],
        dimension: IOSRuntimeTraceDimension,
        importance: IOSRuntimeTraceImportance,
        operationID: String?
    ) {
        guard let attemptID, let generation else { return }
        var contextualPayload = payload
        contextualPayload["attemptId"] = .string(attemptID)
        contextualPayload["generation"] = .integer(Int64(generation))
        IOSRuntimeTrace.state(
            domain: "ios-streaming",
            event: event,
            payload: contextualPayload,
            dimension: dimension,
            importance: importance,
            operationID: operationID
        )
    }

    private func publish(_ state: StreamingFeatureState, generation: UInt64) async {
        guard isCurrent(generation) else { return }
        guard let stateSink, lastPublishedState != state else { return }
        // 在跨 MainActor 等待前记住状态，合并重入的健康回报。
        lastPublishedState = state
        await stateSink(generation, state)
    }

    private func ensureCurrent(_ generation: UInt64) throws {
        guard isCurrent(generation), !Task.isCancelled else {
            throw StreamingRuntimeError.superseded
        }
    }

    private var playbackReady: Bool {
        playbackReadiness.isPlaybackReady
    }

    private var connectionStateAfterNegotiation: StreamingFeatureState {
        guard playbackReadiness.contains(.peerConnected) else { return .connecting }
        return playbackReady ? .playing : .waitingForFirstFrame
    }

    private func isCurrent(_ generation: UInt64) -> Bool {
        activeGeneration == generation
    }
}
