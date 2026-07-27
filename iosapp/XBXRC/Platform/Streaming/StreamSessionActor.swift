import Foundation

private extension Duration {
    var milliseconds: Int64 {
        Int64(components.seconds) * 1_000
            + Int64(components.attoseconds / 1_000_000_000_000_000)
    }
}

private final class RemoteAnswerApplyGate: @unchecked Sendable {
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
    private var remoteIceApplicationTask: Task<Void, Never>?
    private var localIceFlushTask: Task<Void, Error>?
    private var activeLocalIceFlushTaskID: UInt64 = 0
    private var nextLocalIceFlushTaskID: UInt64 = 0
    private var latestRequestedGeneration: UInt64 = 0
    private var stateSink: StateSink?
    private var videoSink: VideoSink?
    private var didMarkConnected = false
    private var didReceiveFirstVideoFrame = false
    private var didBecomeControlReady = false
    private var didBecomeVideoSurfaceRendererReady = false
    private var remoteDescriptionApplied = false
    private var localIceFlushInFlight = false
    private var localIceGatheringComplete = false
    private var localIceCompletionSent = false
    private var reconnectInFlight = false
    private var didObserveSteadyMedia = false
    private var localIceTraceCompleted = false
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

    init(
        controlFactory: any StreamingControlSessionFactory,
        peerFactory: any StreamingPeerRuntimeFactory,
        remoteAnswerApplyTimeout: Duration = .seconds(10)
    ) {
        self.controlFactory = controlFactory
        self.peerFactory = peerFactory
        self.remoteAnswerApplyTimeout = remoteAnswerApplyTimeout
    }

    func start(
        request: StreamingLaunchRequest,
        stateSink: @escaping StateSink,
        videoSink: @escaping VideoSink
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
        self.stateSink = stateSink
        self.videoSink = videoSink
        didMarkConnected = false
        didReceiveFirstVideoFrame = false
        didBecomeControlReady = false
        didBecomeVideoSurfaceRendererReady = false
        remoteDescriptionApplied = false
        localIceFlushInFlight = false
        localIceGatheringComplete = false
        localIceCompletionSent = false
        reconnectInFlight = false
        didObserveSteadyMedia = false
        localIceTraceCompleted = false
        selectedTerminalReason = nil
        pendingLocalCandidates.removeAll(keepingCapacity: true)
        interactionEventCounts.removeAll(keepingCapacity: true)
        dataChannelProfilesPendingTrace = false
        activePeerEpoch = 0
        activeLocalIceEpoch = nil
        pendingConnectedPeerEpoch = nil
        pendingDisconnectedPeerEpoch = nil

        await publish(.creatingSession, generation: request.sessionGeneration)
        trace("sessionCreateStarted", dimension: .lifecycle)
        do {
            let control = try await controlFactory.createSession(request: request)
            do {
                try ensureCurrent(request.sessionGeneration)
            } catch {
                await control.close()
                throw error
            }
            controlSession = control
            let prepared = try await control.prepareSignaling()
            preparedSignaling = prepared
            try ensureCurrent(request.sessionGeneration)
            trace("sessionReady", dimension: .lifecycle)

            await publish(.negotiating, generation: request.sessionGeneration)
            let peer = try await makePeerRuntime(generation: request.sessionGeneration)
            guard let localIceEpoch = await beginLocalIceNegotiation(
                generation: request.sessionGeneration,
                peerEpoch: activePeerEpoch
            ) else {
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
            remoteDescriptionApplied = true
            try await flushLocalIceAndWait(
                generation: request.sessionGeneration,
                epoch: localIceEpoch
            )
            try ensureCurrent(request.sessionGeneration)
            await publish(.connecting, generation: request.sessionGeneration)
            await completePendingPeerConnection(
                generation: request.sessionGeneration,
                peerEpoch: activePeerEpoch
            )
            startRemoteIceApplication(
                generation: request.sessionGeneration,
                peerEpoch: activePeerEpoch
            )
            await completePendingPeerDisconnection(
                generation: request.sessionGeneration,
                peerEpoch: activePeerEpoch
            )
        } catch LocalIceFlushError.negotiationSuperseded {
            return
        } catch {
            if isCurrent(request.sessionGeneration) {
                if peerRuntime != nil, !remoteDescriptionApplied {
                    trace(
                        "signalingTerminal",
                        payload: ["reason": .string(StreamingTerminalReason.sessionFailed.code)],
                        dimension: .network,
                        importance: .key
                    )
                }
                await stopActiveSession(reason: .sessionFailed, publishIdle: false)
            } else {
                await controlFactory.releaseAccess(handle: request.accessHandle)
            }
            throw error
        }
    }

    func stop(
        generation: UInt64? = nil,
        reason: StreamingTerminalReason = .userStop
    ) async {
        if let generation, generation != activeGeneration { return }
        await stopActiveSession(reason: reason, publishIdle: true)
    }

    func videoSurfaceRendererReady(context: StreamingPresentationTraceContext) async {
        guard isCurrent(context.generation),
              context.attemptID == activeAttemptID,
              context.peerEpoch == activePeerEpoch,
              !didBecomeVideoSurfaceRendererReady
        else { return }
        didBecomeVideoSurfaceRendererReady = true
        if playbackReady {
            await publish(.playing, generation: context.generation)
        }
    }

    private func handlePeerEvent(
        _ event: StreamingPeerEvent,
        generation: UInt64,
        peerEpoch: UInt64
    ) async {
        guard isCurrent(generation), peerEpoch == activePeerEpoch else { return }
        switch event {
        case let .localCandidate(candidate):
            guard let epoch = activeLocalIceEpoch, epoch.peerEpoch == peerEpoch else { return }
            pendingLocalCandidates.append(candidate)
            scheduleLocalIceFlush(generation: generation, epoch: epoch)
        case .localIceGatheringComplete:
            guard let epoch = activeLocalIceEpoch, epoch.peerEpoch == peerEpoch else { return }
            localIceGatheringComplete = true
            scheduleLocalIceFlush(generation: generation, epoch: epoch)
        case let .connectionStateChanged(state):
            switch state {
            case .connected:
                guard remoteDescriptionApplied else {
                    pendingConnectedPeerEpoch = peerEpoch
                    return
                }
                await markPeerConnected(generation: generation, peerEpoch: peerEpoch)
            case .disconnected:
                guard remoteDescriptionApplied else {
                    if !reconnectInFlight { pendingDisconnectedPeerEpoch = peerEpoch }
                    return
                }
                await recoverDisconnectedPeer(generation: generation, peerEpoch: peerEpoch)
            case .failed:
                await fail(
                    reason: .iceFailed,
                    generation: generation,
                    error: StreamingRuntimeError.peerConnectionFailed
                )
            case .closed:
                await fail(
                    reason: .peerClosed,
                    generation: generation,
                    error: StreamingRuntimeError.peerConnectionClosed
                )
            case .new, .checking:
                break
            }
        case let .videoTrack(track):
            guard let attemptID = activeAttemptID else { return }
            await videoSink?(
                generation,
                StreamingVideoTrackHandle(
                    rawValue: track.rawValue,
                    traceContext: StreamingPresentationTraceContext(
                        attemptID: attemptID,
                        generation: generation,
                        peerEpoch: peerEpoch
                    )
                )
            )
        case .firstVideoFrame:
            let isFirstFrame = !didReceiveFirstVideoFrame
            didReceiveFirstVideoFrame = true
            if isFirstFrame { trace("firstVideoFrame", dimension: .mediaSupply) }
            if playbackReady {
                await publish(.playing, generation: generation)
            }
        case .audioTrackReady:
            trace("audioTrackReady", dimension: .mediaSupply)
        case let .stats(stats):
            traceSnapshot(
                "rtcHealthSnapshot",
                payload: [
                    "inboundVideoBytes": .integer(Int64(clamping: stats.inboundVideoBytes)),
                    "inboundAudioBytes": .integer(Int64(clamping: stats.inboundAudioBytes)),
                    "framesDecoded": .integer(Int64(clamping: stats.framesDecoded)),
                    "framesDropped": .integer(Int64(clamping: stats.framesDropped)),
                    "packetsLost": .integer(stats.packetsLost),
                    "packetsReceived": .integer(Int64(clamping: stats.packetsReceived)),
                    "rttMs": stats.roundTripTimeSeconds.map { .double($0 * 1_000) } ?? .null,
                    "jitterMs": stats.jitterSeconds.map { .double($0 * 1_000) } ?? .null,
                    "receiveBitrateBps": stats.receiveBitrateBps.map(IOSRuntimeTraceValue.double) ?? .null,
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
                ],
                dimension: .mediaSupply,
                importance: .key
            )
            traceSnapshot(
                "displayHealthSnapshot",
                payload: [
                    "submitCount": .string("unsupported"),
                    "presentCount": .string("unsupported"),
                    "lastSubmitAtMs": .string("unsupported"),
                    "lastPresentAtMs": .string("unsupported"),
                    "submitToPresentP95Ms": .string("unsupported"),
                    "displayedFrameDelta": .string("unsupported"),
                ],
                dimension: .presentation,
                importance: .key
            )
            if !didObserveSteadyMedia,
               stats.framesDecoded > 0, stats.inboundVideoBytes > 0,
               stats.frameSupplyDelta.map({ $0 > 0 }) == true
            {
                didObserveSteadyMedia = true
                trace("steadyMediaObserved", dimension: .mediaSupply)
            }
        case let .dataChannelEvent(label, event):
            await traceDataChannelEvent(label: label, event: event)
        case let .failed(message):
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

    private func makePeerRuntime(generation: UInt64) async throws -> any StreamingPeerRuntime {
        nextPeerEpoch &+= 1
        let peerEpoch = nextPeerEpoch
        activePeerEpoch = peerEpoch
        didReceiveFirstVideoFrame = false
        dataChannelProfilesPendingTrace = false
        didBecomeControlReady = false
        didBecomeVideoSurfaceRendererReady = false
        didObserveSteadyMedia = false
        pendingConnectedPeerEpoch = nil
        pendingDisconnectedPeerEpoch = nil
        let peer = peerFactory.makeRuntime { [weak self] event in
            Task {
                await self?.handlePeerEvent(
                    event,
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
              remoteDescriptionApplied, !didMarkConnected
        else { return }
        didMarkConnected = true
        do {
            try await controlSession?.markConnected()
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
            didMarkConnected = false
            await fail(reason: .sessionFailed, generation: generation, error: error)
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
                    guard await self.isCurrent(generation),
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
                    guard let self, await self.isCurrent(generation),
                          await self.activePeerEpoch == peerEpoch
                    else { return }
                    await self.fail(reason: .sessionFailed, generation: generation, error: error)
                    return
                }
            }
        }
    }

    private func recoverDisconnectedPeer(generation: UInt64, peerEpoch: UInt64) async {
        guard isCurrent(generation), !reconnectInFlight,
              peerEpoch == activePeerEpoch,
              let peer = peerRuntime,
              let control = controlSession,
              let prepared = preparedSignaling
        else { return }
        reconnectInFlight = true
        pendingDisconnectedPeerEpoch = nil
        didMarkConnected = false
        remoteDescriptionApplied = false
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
            guard let localIceEpoch = await beginLocalIceNegotiation(
                generation: generation,
                peerEpoch: peerEpoch
            ) else {
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
            remoteDescriptionApplied = true
            try await flushLocalIceAndWait(generation: generation, epoch: localIceEpoch)
            try ensureCurrent(generation)
            reconnectInFlight = false
            await publish(.connecting, generation: generation)
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
        restartError _: Error
    ) async {
        guard isCurrent(generation), previousPeerEpoch == activePeerEpoch else { return }
        let rebuildOperationID = UUID().uuidString
        trace(
            "runtimeRebuildStarted",
            payload: ["previousPeerEpoch": .integer(Int64(previousPeerEpoch))],
            dimension: .recovery,
            operationID: rebuildOperationID
        )

        activePeerEpoch = 0
        peerRuntime = nil
        activeLocalIceEpoch = nil
        await cancelAndAwaitLocalIceFlush()
        remoteDescriptionApplied = false
        localIceGatheringComplete = false
        localIceCompletionSent = false
        localIceTraceCompleted = false
        pendingLocalCandidates.removeAll(keepingCapacity: true)
        dataChannelProfilesPendingTrace = false
        pendingConnectedPeerEpoch = nil
        pendingDisconnectedPeerEpoch = nil
        retiringPeerRuntime = peer
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
        await videoSink?(generation, nil)

        var replacementPeerEpoch: UInt64 = 0
        do {
            let replacement = try await makePeerRuntime(generation: generation)
            replacementPeerEpoch = activePeerEpoch
            guard let localIceEpoch = await beginLocalIceNegotiation(
                generation: generation,
                peerEpoch: replacementPeerEpoch
            ) else {
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
            remoteDescriptionApplied = true
            try await flushLocalIceAndWait(generation: generation, epoch: localIceEpoch)
            try ensureCurrent(generation)
            reconnectInFlight = false
            await publish(.connecting, generation: generation)
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
        localIceGatheringComplete = false
        localIceCompletionSent = false
        localIceTraceCompleted = false
        pendingLocalCandidates.removeAll(keepingCapacity: true)
        return epoch
    }

    private func scheduleLocalIceFlush(generation: UInt64, epoch: LocalIceEpoch) {
        guard remoteDescriptionApplied, localIceFlushTask == nil,
              activeLocalIceEpoch == epoch else { return }
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
        guard isCurrent(generation), remoteDescriptionApplied,
              activeLocalIceEpoch == epoch, !localIceFlushInFlight else { return }
        localIceFlushInFlight = true
        defer {
            if isCurrent(generation), activeLocalIceEpoch == epoch {
                localIceFlushInFlight = false
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
            if localIceGatheringComplete, !localIceCompletionSent {
                localIceCompletionSent = true
                do {
                    try await controlSession?.completeLocalIceGathering()
                    try Task.checkCancellation()
                    guard isCurrent(generation), activeLocalIceEpoch == epoch else {
                        throw StreamingRuntimeError.superseded
                    }
                    if !localIceTraceCompleted {
                        localIceTraceCompleted = true
                        trace("localIceCompleted", dimension: .network)
                    }
                } catch {
                    if isCurrent(generation) {
                        localIceCompletionSent = false
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
        localIceFlushInFlight = false
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
        localIceFlushInFlight = false
    }

    private func cancelAndAwaitLocalIceFlush() async {
        guard let task = localIceFlushTask else {
            localIceFlushInFlight = false
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

    private func fail(
        reason: StreamingTerminalReason,
        generation: UInt64,
        error _: Error
    ) async {
        guard isCurrent(generation), selectTerminal(reason, generation: generation) else { return }
        await publish(
            .failed(message: reason.userMessage, retryable: reason.retryable),
            generation: generation
        )
        await stopActiveSession(
            reason: reason,
            publishIdle: false,
            preserveStateSink: true
        )
    }

    private func stopActiveSession(
        reason: StreamingTerminalReason,
        publishIdle: Bool,
        preserveStateSink: Bool = false
    ) async {
        let generation = activeGeneration
        if let generation { _ = selectTerminal(reason, generation: generation) }
        let attemptID = activeAttemptID
        let remoteIceTask = remoteIceApplicationTask
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
        videoSink = nil
        didMarkConnected = false
        didReceiveFirstVideoFrame = false
        didBecomeControlReady = false
        didBecomeVideoSurfaceRendererReady = false
        remoteDescriptionApplied = false
        localIceFlushInFlight = false
        localIceGatheringComplete = false
        localIceCompletionSent = false
        reconnectInFlight = false
        didObserveSteadyMedia = false
        localIceTraceCompleted = false
        pendingLocalCandidates.removeAll(keepingCapacity: true)
        interactionEventCounts.removeAll(keepingCapacity: true)
        dataChannelProfilesPendingTrace = false
        activePeerEpoch = 0
        activeLocalIceEpoch = nil
        pendingConnectedPeerEpoch = nil
        pendingDisconnectedPeerEpoch = nil
        if let peer {
            await peer.stopInputAndHaptics()
        }
        if let retiringStopTask {
            await retiringStopTask.value
        } else if let retiringPeer {
            await retiringPeer.stopInputAndHaptics()
        }
        if peer != nil || retiringPeer != nil {
            traceCleanup("inputStopped", attemptID: attemptID, generation: generation)
            traceCleanup("hapticsStopped", attemptID: attemptID, generation: generation)
        }
        remoteIceTask?.cancel()
        localIceTask?.cancel()
        _ = try? await localIceTask?.value
        traceCleanup("iceTasksCancelled", attemptID: attemptID, generation: generation)
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
            traceCleanup("peerClosed", attemptID: attemptID, generation: generation)
        }
        if let control {
            await control.close()
            traceCleanup("remoteSessionClosed", attemptID: attemptID, generation: generation)
        }
        if let accessHandle {
            await controlFactory.releaseAccess(handle: accessHandle)
            traceCleanup("accessReleased", attemptID: attemptID, generation: generation)
        }
        if let generation {
            await previousVideoSink?(generation, nil)
            if publishIdle { await previousStateSink?(generation, .idle) }
        }
        if preserveStateSink { return }
    }

    @discardableResult
    private func selectTerminal(
        _ reason: StreamingTerminalReason,
        generation: UInt64
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
                "retryable": .bool(reason.retryable),
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
            if didMarkConnected {
                dataChannelProfilesPendingTrace = false
                traceDataChannelProfiles()
            }
            return
        }
        if label == "control", event == "ready" {
            guard !didBecomeControlReady else { return }
            didBecomeControlReady = true
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
        let canonicalEvent: String? = switch (label, event) {
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
        await stateSink?(generation, state)
    }

    private func ensureCurrent(_ generation: UInt64) throws {
        guard isCurrent(generation), !Task.isCancelled else {
            throw StreamingRuntimeError.superseded
        }
    }

    private var playbackReady: Bool {
        didMarkConnected
            && didReceiveFirstVideoFrame
            && didBecomeControlReady
            && didBecomeVideoSurfaceRendererReady
    }

    private func isCurrent(_ generation: UInt64) -> Bool {
        activeGeneration == generation
    }
}
