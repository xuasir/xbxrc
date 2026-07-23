import Foundation
import SwiftUI

@MainActor
final class StreamingFeatureStore: ObservableObject {
    @Published private(set) var state: StreamingFeatureState = .idle
    @Published private(set) var videoTrack: StreamingVideoTrackHandle?
    @Published private(set) var streamTitleID: String?
    @Published private(set) var streamTarget: StreamingLaunchTarget?

    private let sessionActor: StreamSessionActor
    private var generation: UInt64 = 0
    private var attemptID: String?
    private var launchTask: Task<Void, Never>?

    init() {
        sessionActor = StreamSessionActor(
            controlFactory: RustStreamingControlSessionFactory(),
            peerFactory: LibWebRTCPeerRuntimeFactory()
        )
    }

    init(
        controlFactory: any StreamingControlSessionFactory,
        peerFactory: any StreamingPeerRuntimeFactory,
        remoteAnswerApplyTimeout: Duration = .seconds(10)
    ) {
        sessionActor = StreamSessionActor(
            controlFactory: controlFactory,
            peerFactory: peerFactory,
            remoteAnswerApplyTimeout: remoteAnswerApplyTimeout
        )
    }

    var isPresentingPlayer: Bool { state.presentsPlayer }

    func start(
        streamTitleID rawStreamTitleID: String,
        prepareAccess: @escaping @MainActor () async throws -> PreparedCloudAccess
    ) {
        start(target: .cloud, targetID: rawStreamTitleID) {
            let access = try await prepareAccess()
            return PreparedStreamingAccess(
                handle: access.handle,
                accountID: access.accountID,
                regionHost: access.regionHost,
                ownerGeneration: access.ownerGeneration,
                expiresAtMs: access.expiresAtMs
            )
        }
    }

    func startHome(
        targetID rawTargetID: String,
        prepareAccess: @escaping @MainActor () async throws -> PreparedHomeAccess
    ) {
        start(target: .home, targetID: rawTargetID) {
            let access = try await prepareAccess()
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
        prepareAccess: @escaping @MainActor () async throws -> PreparedStreamingAccess
    ) {
        let targetID = rawTargetID.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !targetID.isEmpty else {
            state = .failed(
                message: StreamingRuntimeError.invalidStreamTitleID.localizedDescription,
                retryable: false
            )
            return
        }

        generation &+= 1
        let requestGeneration = generation
        let requestAttemptID = UUID().uuidString
        attemptID = requestAttemptID
        launchTask?.cancel()
        streamTitleID = targetID
        streamTarget = target
        videoTrack = nil
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
                    access = try await prepareAccess()
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
                    try? releaseStreamAccess(accessHandle: access.handle)
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
                try await self.sessionActor.start(
                    request: StreamingLaunchRequest(
                        target: target,
                        targetID: targetID,
                        accessHandle: access.handle,
                        accountGeneration: Self.accountGeneration(
                            accountID: access.accountID,
                            regionHost: access.regionHost
                        ),
                        sessionGeneration: requestGeneration,
                        attemptID: requestAttemptID,
                        accountID: access.accountID,
                        ownerGeneration: access.ownerGeneration
                    ),
                    stateSink: { [weak self] generation, state in
                        self?.apply(state: state, generation: generation)
                    },
                    videoSink: { [weak self] generation, track in
                        guard let self, generation == self.generation else { return }
                        self.videoTrack = track
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
                let wasSuperseded = Task.isCancelled || requestGeneration != self.generation
                if !sessionActorOwnsTerminal {
                    let reason: StreamingTerminalReason = wasSuperseded
                        ? .superseded
                        : .runtimeFailure
                    self.traceAttempt(
                        "terminalSelected",
                        attemptID: requestAttemptID,
                        generation: requestGeneration,
                        payload: [
                            "reason": .string(reason.code),
                            "retryable": .bool(reason.retryable),
                        ],
                        dimension: .lifecycle,
                        importance: .essential
                    )
                }
                guard !wasSuperseded else { return }
                self.state = .failed(
                    message: CloudLibraryDiagnostics.safeError(error),
                    retryable: true
                )
                self.traceState(self.state, generation: requestGeneration)
            }
        }
    }

    func stop() {
        stop(reason: .userStop)
    }

    func stopForAuthRevocation() {
        stop(reason: .authRevoked)
    }

    private func stop(reason: StreamingTerminalReason) {
        guard state != .idle else { return }
        let sessionGeneration = generation
        generation &+= 1
        let stoppedGeneration = generation
        launchTask?.cancel()
        launchTask = nil
        state = .stopping
        traceState(state, generation: sessionGeneration)
        Task { [weak self] in
            guard let self else { return }
            await self.sessionActor.stop(generation: sessionGeneration, reason: reason)
            guard stoppedGeneration == self.generation else { return }
            self.videoTrack = nil
            self.streamTitleID = nil
            self.streamTarget = nil
            self.state = .idle
            self.attemptID = nil
            self.traceState(.idle, generation: stoppedGeneration)
        }
    }

    func handleScenePhase(_ phase: ScenePhase) {
        guard phase == .background, state != .idle else { return }
        state = .suspending
        traceState(state, generation: generation)
        stop(reason: .backgroundStop)
    }

    private func apply(state: StreamingFeatureState, generation: UInt64) {
        guard generation == self.generation else { return }
        self.state = state
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
}

struct RustStreamingControlSessionFactory: StreamingControlSessionFactory {
    func createSession(request: StreamingLaunchRequest) async throws -> any StreamingControlSession {
        let session = try createScopedStreamSession(
            accessHandle: request.accessHandle,
            targetType: request.target == .cloud ? "cloud" : "home",
            targetId: request.targetID,
            accountId: request.accountID,
            ownerGeneration: request.ownerGeneration
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
                normalizeEndOfCandidates: snapshot.webRtcPlan.normalizeEndOfCandidates
            )
        )
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
                candidates: candidates.map { candidate in XboxIceCandidate(
                    candidate: candidate.sdp,
                    sdpMLineIndex: UInt32(exactly: candidate.sdpMLineIndex),
                    sdpMid: candidate.sdpMid,
                    usernameFragment: nil,
                    messageType: nil
                ) }
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
                    "elapsedMs": .integer(Int64(Date().timeIntervalSince(startedAt) * 1_000)),
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
                    sdpMLineIndex: Int32(clamping: $0.sdpMLineIndex ?? 0)
                )
            },
            endOfCandidates: batch.endOfCandidates
        )
    }

    func markConnected() async throws {
        try await session.markConnected(generation: requireGeneration())
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
