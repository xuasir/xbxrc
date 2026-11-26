import AVFoundation
import Foundation
@preconcurrency import WebRTC

enum StreamingLaunchTarget: String, Equatable, Sendable {
    case cloud
    case home
}

struct CloudStreamingIdentity: Equatable, Sendable {
    let productID: String
    let streamTitleID: String
    let hasEntitlement: Bool?
    var xboxTitleID: String? = nil
    var name: String? = nil
}

protocol CloudGameLaunchResolving: Sendable {
    func resolve(
        accessHandle: String, productID: String, serverRegion: String, startup: XboxStreamStartup?
    ) async throws
        -> CloudStreamingIdentity
}

struct RustCloudGameLaunchResolver: CloudGameLaunchResolving {
    func resolve(
        accessHandle: String, productID: String, serverRegion: String, startup: XboxStreamStartup?
    ) async throws
        -> CloudStreamingIdentity
    {
        guard
            let game = try await resolveCloudGame(
                accessHandle: accessHandle, productId: productID, serverRegion: serverRegion,
                startup: startup),
            let title = game.streamTitleId, !title.isEmpty
        else { throw StreamingRuntimeError.cloudGameUnavailable }
        return CloudStreamingIdentity(
            productID: game.productId, streamTitleID: title, hasEntitlement: game.hasEntitlement,
            xboxTitleID: game.xboxTitleId, name: game.name)
    }
}

struct StreamingStartupProgress: Equatable, Sendable {
    let phase: String
    var status = "entered"
    var streamState: String?
    var totalWaitSeconds: UInt64?
    var allocationWaitSeconds: UInt64?
    var provisioningWaitSeconds: UInt64?
    var retryCount: UInt8 = 0
    var retryLimit: UInt8 = 0

    var statusText: String {
        switch phase {
        case "checkingGame": "正在验证游戏的云端身份与访问权限"
        case "preparingConsole": "正在唤醒并准备 Xbox 主机"
        case "resolvingContext": "正在准备串流会话环境"
        case "creatingSession": "正在创建串流会话"
        case "waitingSessionReady":
            if streamState == "Queued" || totalWaitSeconds != nil {
                "正在排队等待串流会话"
            } else if streamState == "Provisioning" {
                "正在准备串流环境"
            } else {
                "正在等待会话就绪"
            }
        case "ready": "串流会话已就绪"
        case "failed": "串流会话准备失败"
        default: "正在准备串流连接"
        }
    }
    var detailText: String? {
        var parts: [String] = []
        if let totalWaitSeconds { parts.append("预计等待 \(totalWaitSeconds) 秒") }
        if let allocationWaitSeconds { parts.append("分配 \(allocationWaitSeconds) 秒") }
        if let provisioningWaitSeconds { parts.append("准备 \(provisioningWaitSeconds) 秒") }
        if retryCount > 0 { parts.append("重试 \(retryCount)/\(retryLimit)") }
        return parts.isEmpty ? nil : parts.joined(separator: " · ")
    }

    func localizedDetailText(locale: Locale) -> String? {
        var parts: [String] = []
        if let totalWaitSeconds {
            parts.append(
                appLocalizedFormat(
                    "预计等待 %@ 秒", locale: locale, totalWaitSeconds.formatted(.number.locale(locale)))
            )
        }
        if let allocationWaitSeconds {
            parts.append(
                appLocalizedFormat(
                    "分配 %@ 秒", locale: locale,
                    allocationWaitSeconds.formatted(.number.locale(locale))))
        }
        if let provisioningWaitSeconds {
            parts.append(
                appLocalizedFormat(
                    "准备 %@ 秒", locale: locale,
                    provisioningWaitSeconds.formatted(.number.locale(locale))))
        }
        if retryCount > 0 {
            parts.append(
                appLocalizedFormat(
                    "重试 %@/%@", locale: locale, String(retryCount), String(retryLimit)))
        }
        return parts.isEmpty ? nil : parts.joined(separator: " · ")
    }
}

enum StreamingFeatureState: Equatable, Sendable {
    case idle
    case preparingAccess
    case creatingSession
    case negotiating
    case connecting
    case recovering
    case waitingForFirstFrame
    case playing
    case suspending
    case stopping
    case failed(message: String, retryable: Bool)

    var presentsPlayer: Bool { self != .idle }

    var statusText: String {
        switch self {
        case .idle: ""
        case .preparingAccess: "正在准备 Xbox 串流访问权限"
        case .creatingSession: "正在创建串流会话"
        case .negotiating: "正在协商音视频连接"
        case .connecting: "正在连接 Xbox 主机"
        case .recovering: "串流连接正在恢复"
        case .waitingForFirstFrame: "连接成功，正在等待串流就绪"
        case .playing: "串流中"
        case .suspending: "正在挂起串流"
        case .stopping: "正在结束串流"
        case .failed(let message, _): message
        }
    }

    var traceCode: String {
        switch self {
        case .idle: "idle"
        case .preparingAccess: "preparingAccess"
        case .creatingSession: "creatingSession"
        case .negotiating: "negotiating"
        case .connecting: "connecting"
        case .recovering: "recovering"
        case .waitingForFirstFrame: "waitingForFirstFrame"
        case .playing: "playing"
        case .suspending: "suspending"
        case .stopping: "stopping"
        case .failed: "failed"
        }
    }

    var traceRetryable: Bool? {
        guard case .failed(_, let retryable) = self else { return nil }
        return retryable
    }
}

struct StreamingLaunchRequest: Equatable, Sendable {
    let attemptID: String
    let target: StreamingLaunchTarget
    let targetID: String
    let accessHandle: String
    let accountID: String
    let accountGeneration: UInt64
    let ownerGeneration: UInt64
    let sessionGeneration: UInt64
    let settings: StreamingSessionSettingsSnapshot
    let startup: XboxStreamStartup?

    var streamTitleID: String { targetID }

    static func == (lhs: Self, rhs: Self) -> Bool {
        lhs.attemptID == rhs.attemptID && lhs.target == rhs.target && lhs.targetID == rhs.targetID
            && lhs.accessHandle == rhs.accessHandle && lhs.accountID == rhs.accountID
            && lhs.accountGeneration == rhs.accountGeneration
            && lhs.ownerGeneration == rhs.ownerGeneration
            && lhs.sessionGeneration == rhs.sessionGeneration && lhs.settings == rhs.settings
            && lhs.startup === rhs.startup
    }

    init(
        streamTitleID: String,
        accessHandle: String,
        accountGeneration: UInt64,
        sessionGeneration: UInt64,
        settings: StreamingSessionSettingsSnapshot = .standard,
        attemptID: String = UUID().uuidString,
        accountID: String = "",
        ownerGeneration: UInt64? = nil,
        startup: XboxStreamStartup? = nil
    ) {
        self.attemptID = attemptID
        target = .cloud
        targetID = streamTitleID
        self.accessHandle = accessHandle
        self.accountID = accountID
        self.accountGeneration = accountGeneration
        self.ownerGeneration = ownerGeneration ?? accountGeneration
        self.sessionGeneration = sessionGeneration
        self.settings = settings
        self.startup = startup
    }

    init(
        target: StreamingLaunchTarget,
        targetID: String,
        accessHandle: String,
        accountGeneration: UInt64,
        sessionGeneration: UInt64,
        settings: StreamingSessionSettingsSnapshot = .standard,
        attemptID: String = UUID().uuidString,
        accountID: String = "",
        ownerGeneration: UInt64? = nil,
        startup: XboxStreamStartup? = nil
    ) {
        self.attemptID = attemptID
        self.target = target
        self.targetID = targetID
        self.accessHandle = accessHandle
        self.accountID = accountID
        self.accountGeneration = accountGeneration
        self.ownerGeneration = ownerGeneration ?? accountGeneration
        self.sessionGeneration = sessionGeneration
        self.settings = settings
        self.startup = startup
    }
}

struct StreamingIceServer: Equatable, Sendable {
    let urls: [String]
    let username: String?
    let credential: String?
}

enum StreamingMediaDirection: String, Equatable, Sendable {
    case sendReceive
    case receiveOnly
}

enum StreamingAudioPermissionCoordinator {
    static func requiresMicrophonePermission(for direction: StreamingMediaDirection) -> Bool {
        direction == .sendReceive
    }

    @MainActor
    static func requestMicrophoneIfNeeded(
        for direction: StreamingMediaDirection
    ) async throws {
        guard requiresMicrophonePermission(for: direction) else { return }

        let permission = AVAudioApplication.shared.recordPermission
        switch permission {
        case .granted:
            return
        case .denied:
            throw StreamingRuntimeError.microphonePermissionDenied
        case .undetermined:
            let gate = RemoteAnswerApplyGate()
            try await withTaskCancellationHandler {
                try await withCheckedThrowingContinuation { continuation in
                    gate.install(continuation)
                    let request = Task {
                        AVAudioApplication.requestRecordPermission { granted in
                            gate.resolve(
                                granted
                                    ? .success(())
                                    : .failure(StreamingRuntimeError.microphonePermissionDenied))
                        }
                    }
                    let timeout = Task {
                        do { try await Task.sleep(for: .seconds(30)) } catch { return }
                        gate.resolve(.failure(StreamingRuntimeError.microphoneUnavailable))
                    }
                    gate.installTasks(operationTask: request, timeoutTask: timeout)
                }
            } onCancel: {
                gate.resolve(.failure(CancellationError()))
            }
        @unknown default:
            throw StreamingRuntimeError.microphonePermissionDenied
        }
    }
}

/// WebRtcDirect negotiation 的 iOS 可消费投影，由 Rust plan 生成并保留桌面默认 fixture。
struct StreamingWebRtcPlan: Equatable, Sendable {
    var audioDirection: StreamingMediaDirection
    let videoDirection: StreamingMediaDirection
    let videoCodecMimeType: String
    let targetVideoWidth: Int
    let targetVideoHeight: Int
    let audioBitrateKbps: Int?
    let h264Profiles: [String]
    let h264PacketizationMode: Int
    let h264LevelAsymmetryAllowed: Bool
    let maxFrameSize: Int
    let maxFrameRate: Int
    var minVideoBitrateKbps: Int?
    var startVideoBitrateKbps: Int?
    var maxVideoBitrateKbps: Int?
    let stereoAudio: Bool
    let requiredVideoRtcpFeedback: [String]
    let allowedCandidateTypes: [String]
    let iceTransportPolicy: String
    let preferIPv6: Bool
    let normalizeEndOfCandidates: Bool
    let preventResolutionDrops: Bool

    static let desktopWebRtcDirect = StreamingWebRtcPlan()

    init(
        audioDirection: StreamingMediaDirection = .receiveOnly,
        videoDirection: StreamingMediaDirection = .receiveOnly,
        videoCodecMimeType: String = "video/H264",
        targetVideoWidth: Int = 1_920,
        targetVideoHeight: Int = 1_080,
        audioBitrateKbps: Int? = 128,
        h264Profiles: [String] = ["4d", "42e", "420"],
        h264PacketizationMode: Int = 1,
        h264LevelAsymmetryAllowed: Bool = true,
        maxFrameSize: Int = 8_160,
        maxFrameRate: Int = 60,
        minVideoBitrateKbps: Int? = 5_000,
        startVideoBitrateKbps: Int? = 20_000,
        maxVideoBitrateKbps: Int? = 20_000,
        stereoAudio: Bool = true,
        requiredVideoRtcpFeedback: [String] = [
            "nack", "nack pli", "ccm fir", "goog-remb", "transport-cc",
        ],
        allowedCandidateTypes: [String] = ["host", "srflx", "relay"],
        iceTransportPolicy: String = "all",
        preferIPv6: Bool = false,
        normalizeEndOfCandidates: Bool = true,
        preventResolutionDrops: Bool = false
    ) {
        self.audioDirection = audioDirection
        self.videoDirection = videoDirection
        self.videoCodecMimeType = videoCodecMimeType
        self.targetVideoWidth = max(targetVideoWidth, 1)
        self.targetVideoHeight = max(targetVideoHeight, 1)
        self.audioBitrateKbps = audioBitrateKbps.map { max($0, 1) }
        self.h264Profiles = h264Profiles
        self.h264PacketizationMode = h264PacketizationMode
        self.h264LevelAsymmetryAllowed = h264LevelAsymmetryAllowed
        self.maxFrameSize = maxFrameSize
        self.maxFrameRate = maxFrameRate
        self.minVideoBitrateKbps = minVideoBitrateKbps
        self.startVideoBitrateKbps = startVideoBitrateKbps
        self.maxVideoBitrateKbps = maxVideoBitrateKbps
        self.stereoAudio = stereoAudio
        self.requiredVideoRtcpFeedback = requiredVideoRtcpFeedback
        self.allowedCandidateTypes = allowedCandidateTypes
        self.iceTransportPolicy = iceTransportPolicy
        self.preferIPv6 = preferIPv6
        self.normalizeEndOfCandidates = normalizeEndOfCandidates
        self.preventResolutionDrops = preventResolutionDrops
    }

    func withVideoBitrateCap(_ bitrateKbps: Int) -> Self {
        var plan = self
        let cap = max(0, bitrateKbps)
        plan.maxVideoBitrateKbps = cap
        plan.minVideoBitrateKbps = minVideoBitrateKbps.map { min($0, cap) }
        plan.startVideoBitrateKbps = startVideoBitrateKbps.map { min($0, cap) }
        return plan
    }
}

struct StreamingPreparedSignaling: Equatable, Sendable {
    var iceServers: [StreamingIceServer]
    var webRtcPlan: StreamingWebRtcPlan
    var fallbackIceServers: [StreamingIceServer]?

    init(
        iceServers: [StreamingIceServer],
        webRtcPlan: StreamingWebRtcPlan = .desktopWebRtcDirect,
        fallbackIceServers: [StreamingIceServer]? = nil
    ) {
        self.iceServers = iceServers
        self.webRtcPlan = webRtcPlan
        self.fallbackIceServers = fallbackIceServers
    }
}

struct StreamingIceCandidate: Equatable, Sendable {
    let sdp: String
    let sdpMid: String?
    let sdpMLineIndex: Int32
    var usernameFragment: String? = nil
    var source: String? = nil
}

struct StreamingRemoteIceBatch: Equatable, Sendable {
    let candidates: [StreamingIceCandidate]
    let endOfCandidates: Bool
    var configurationPending: Bool = false
}

/// Rust opaque session 的窄桥接面。Swift 负责 libwebrtc 输入输出，所有远端会话策略留在 Rust。
protocol StreamingControlSession: Sendable {
    func pendingAuthUpdate() async -> XboxStreamingAuthUpdate?
    func acknowledgeAuthUpdate(revision: UInt64) async
    func synchronizeAuthSession(revision: UInt64, session: AuthSession) async throws
    func prepareSignaling() async throws -> StreamingPreparedSignaling
    func startupProgress() async -> StreamingStartupProgress?
    func exchangeOffer(_ sdp: String) async throws -> String
    func exchangeMediaOffer(_ sdp: String, kind: String) async throws -> String
    func validateResume() async throws
    func supervise(_ sample: StreamingSupervisionSample) async throws
        -> StreamingSupervisionDecision
    func submitLocalCandidates(_ candidates: [StreamingIceCandidate]) async throws
    func completeLocalIceGathering() async throws
    func nextRemoteIceBatch() async throws -> StreamingRemoteIceBatch
    func markConnected() async throws
    func sendText(_ text: String) async throws -> Bool
    func powerOffConsole() async throws -> Bool
    func close() async
}

extension StreamingControlSession {
    func exchangeMediaOffer(_ sdp: String, kind: String) async throws -> String {
        try await exchangeOffer(sdp)
    }
    func validateResume() async throws {}
    func supervise(_ sample: StreamingSupervisionSample) async throws
        -> StreamingSupervisionDecision
    { .unavailable }
    func pendingAuthUpdate() async -> XboxStreamingAuthUpdate? { nil }
    func acknowledgeAuthUpdate(revision: UInt64) async {}
    func synchronizeAuthSession(revision: UInt64, session: AuthSession) async throws {}
    func startupProgress() async -> StreamingStartupProgress? { nil }
}

enum StreamingAuthCommitResult: Sendable {
    case committed
    case synchronize(AuthSession)
}

protocol StreamingControlSessionFactory: Sendable {
    func createSession(request: StreamingLaunchRequest) async throws -> any StreamingControlSession
    func releaseAccess(handle: String) async
}

enum StreamingPeerConnectionState: String, Equatable, Sendable {
    case new
    case checking
    case connected
    case disconnected
    case failed
    case closed
}

struct StreamingPeerStats: Equatable, Sendable {
    let inboundVideoBytes: UInt64
    let inboundAudioBytes: UInt64
    let framesDecoded: UInt64
    let framesDropped: UInt64
    let packetsLost: Int64
    let roundTripTimeSeconds: Double?
    let jitterSeconds: Double?
    let packetsReceived: UInt64
    let receiveBitrateBps: Double?
    let freezeCount: UInt64?
    let freezeDurationSeconds: Double?
    let nackCount: UInt64?
    let pliCount: UInt64?
    let firCount: UInt64?
    let selectedCandidatePairProtocol: String?
    let selectedCandidatePairAddressFamily: String?
    let selectedLocalCandidateType: String?
    let selectedRemoteCandidateType: String?
    let firstMediaAtMilliseconds: Double?
    let lastMediaAtMilliseconds: Double?
    let frameSupplyDelta: Int64?
    var observedAtMilliseconds: Double? = nil
    var framesReceived: UInt64? = nil
    var observedFramesDecoded: UInt64? = nil
    var videoPacketsLost: Int64? = nil
    var selectedCandidatePairID: String? = nil
    var videoTiming = StreamingMediaTimingSnapshot()
    var audioTiming = StreamingMediaTimingSnapshot()

    init(
        inboundVideoBytes: UInt64,
        inboundAudioBytes: UInt64,
        framesDecoded: UInt64,
        framesDropped: UInt64,
        packetsLost: Int64,
        roundTripTimeSeconds: Double?,
        jitterSeconds: Double? = nil,
        packetsReceived: UInt64 = 0,
        receiveBitrateBps: Double? = nil,
        freezeCount: UInt64? = nil,
        freezeDurationSeconds: Double? = nil,
        nackCount: UInt64? = nil,
        pliCount: UInt64? = nil,
        firCount: UInt64? = nil,
        selectedCandidatePairProtocol: String? = nil,
        selectedCandidatePairAddressFamily: String? = nil,
        selectedLocalCandidateType: String? = nil,
        selectedRemoteCandidateType: String? = nil,
        firstMediaAtMilliseconds: Double? = nil,
        lastMediaAtMilliseconds: Double? = nil,
        frameSupplyDelta: Int64? = nil
    ) {
        self.inboundVideoBytes = inboundVideoBytes
        self.inboundAudioBytes = inboundAudioBytes
        self.framesDecoded = framesDecoded
        self.framesDropped = framesDropped
        self.packetsLost = packetsLost
        self.roundTripTimeSeconds = roundTripTimeSeconds
        self.jitterSeconds = jitterSeconds
        self.packetsReceived = packetsReceived
        self.receiveBitrateBps = receiveBitrateBps
        self.freezeCount = freezeCount
        self.freezeDurationSeconds = freezeDurationSeconds
        self.nackCount = nackCount
        self.pliCount = pliCount
        self.firCount = firCount
        self.selectedCandidatePairProtocol = selectedCandidatePairProtocol
        self.selectedCandidatePairAddressFamily = selectedCandidatePairAddressFamily
        self.selectedLocalCandidateType = selectedLocalCandidateType
        self.selectedRemoteCandidateType = selectedRemoteCandidateType
        self.firstMediaAtMilliseconds = firstMediaAtMilliseconds
        self.lastMediaAtMilliseconds = lastMediaAtMilliseconds
        self.frameSupplyDelta = frameSupplyDelta
    }
}

struct StreamingDataChannelDebugSnapshot: Equatable, Sendable {
    let readyStates: [String: String]
    let phases: [String: String]
    let handshakeAcknowledged: Bool
    let controlReady: Bool
    let inputStarted: Bool
    let terminalReason: String?
}

struct StreamingPeerDebugSnapshot: Equatable, Sendable {
    let signalingState: String
    let iceConnectionState: String
    let iceGatheringState: String
    let transceiverCount: Int
    let audioReceiverTrackCount: Int
    let videoReceiverTrackCount: Int
    let localDescriptionSet: Bool
    let remoteDescriptionSet: Bool
    let dataChannels: StreamingDataChannelDebugSnapshot?
}

struct StreamingPresentationTraceContext: Equatable, Sendable {
    let attemptID: String
    let generation: UInt64
    let peerEpoch: UInt64
    var videoGeneration: UInt64 = 0
}

final class StreamingVideoTrackHandle: @unchecked Sendable {
    // libwebrtc 轨道保持原生身份，精确配对 renderer 的挂载与解绑。
    let rawValue: RTCVideoTrack
    let traceContext: StreamingPresentationTraceContext?
    let frameFeedback: StreamingFrameFeedbackSession?
    let presentation: StreamingPresentationSession

    init(
        rawValue: RTCVideoTrack,
        traceContext: StreamingPresentationTraceContext? = nil,
        frameFeedback: StreamingFrameFeedbackSession? = nil,
        presentation: StreamingPresentationSession = StreamingPresentationSession()
    ) {
        self.rawValue = rawValue
        self.traceContext = traceContext
        self.frameFeedback = frameFeedback
        self.presentation = presentation
    }
}

enum StreamingPeerEvent: @unchecked Sendable {
    case localCandidate(StreamingIceCandidate)
    case localIceGatheringComplete
    case connectionStateChanged(StreamingPeerConnectionState)
    case videoTrack(StreamingVideoTrackHandle)
    case firstVideoFrame
    case audioTrackReady
    case audioLifecycle(event: String, payload: [String: IOSRuntimeTraceValue])
    case stats(StreamingPeerStats)
    case dataChannelEvent(label: String, event: String)
    case failed(String)
}

enum StreamingDataChannelLabel: String, CaseIterable, Equatable, Sendable {
    case input
    case control
    case chat
    case message

    init?(wireLabel: String) {
        self.init(rawValue: wireLabel.lowercased())
    }
}

enum StreamingDataChannelPhase: String, Equatable, Sendable {
    case created
    case open
    case closed
}

enum StreamingDataChannelBootstrapStage: Equatable, Sendable {
    case preHandshake
    case postHandshake
}

enum StreamingDataChannelBootstrapAction: Equatable, Sendable {
    case sendMessageHandshake
    case sendPostHandshake(index: Int)
    case sendControlBootstrap(stage: StreamingDataChannelBootstrapStage, index: Int)
    case sendInputMetadata(stage: StreamingDataChannelBootstrapStage)
    case announceControlReady
    case scheduleGamepadAnnouncement
    case startInput
}

enum StreamingTerminalReason: Equatable, Sendable {
    case userStop
    case backgroundStop
    case superseded
    case authRevoked
    case sessionFailed
    case sessionClosed
    case remoteKickClosedGame
    case remoteClosed
    case remoteError
    case iceFailed
    case peerClosed
    case dataChannelClosed(StreamingDataChannelLabel)
    case runtimeFailure

    var code: String {
        switch self {
        case .userStop: "user.stop"
        case .backgroundStop: "app.background"
        case .superseded: "session.superseded"
        case .authRevoked: "auth.revoked"
        case .sessionFailed: "session.failed"
        case .sessionClosed: "session.closed"
        case .remoteKickClosedGame: "remote.kick.closedGame"
        case .remoteClosed: "remote.closed"
        case .remoteError: "remote.error"
        case .iceFailed: "ice.failed"
        case .peerClosed: "peer.closed"
        case .dataChannelClosed(let label): "dataChannel.closed.\(label.rawValue)"
        case .runtimeFailure: "runtime.failed"
        }
    }

    var retryable: Bool {
        switch self {
        case .userStop, .backgroundStop, .superseded, .authRevoked,
            .sessionClosed, .remoteKickClosedGame, .remoteClosed, .peerClosed:
            false
        case .sessionFailed, .remoteError, .iceFailed,
            .dataChannelClosed, .runtimeFailure:
            true
        }
    }

    var userMessage: String {
        switch self {
        case .userStop: "串流已结束"
        case .backgroundStop: "应用进入后台，串流已挂起"
        case .superseded: "串流请求已被新的请求替代"
        case .authRevoked: "Xbox 登录状态已失效"
        case .sessionFailed: "Xbox 串流会话失败"
        case .sessionClosed: "Xbox 串流会话已经关闭"
        case .remoteKickClosedGame: "Xbox 已结束当前游戏串流"
        case .remoteClosed: "Xbox 已关闭远端串流"
        case .remoteError: "Xbox 返回了串流错误"
        case .iceFailed: "WebRTC 网络连接失败"
        case .peerClosed: "WebRTC 连接已经关闭"
        case .dataChannelClosed(let label): "Xbox 串流通道已关闭（\(label.rawValue)）"
        case .runtimeFailure: "Xbox 串流运行失败"
        }
    }

    init(peerFailureMessage message: String) {
        if let code = message.split(separator: ":", maxSplits: 1).first {
            switch code {
            case "remote.kick.closedGame": self = .remoteKickClosedGame
            case "remote.closed": self = .remoteClosed
            case "remote.error": self = .remoteError
            case "dataChannel.closed.input": self = .dataChannelClosed(.input)
            case "dataChannel.closed.control": self = .dataChannelClosed(.control)
            case "dataChannel.closed.chat": self = .dataChannelClosed(.chat)
            case "dataChannel.closed.message": self = .dataChannelClosed(.message)
            default: self = .runtimeFailure
            }
        } else {
            self = .runtimeFailure
        }
    }
}

enum StreamingRemoteMessageClassifier {
    static func isHandshakeAck(_ data: Data) -> Bool {
        guard let object = try? JSONSerialization.jsonObject(with: data),
            let dictionary = object as? [String: Any],
            let type = dictionary["type"] as? String
        else { return false }
        return type == "HandshakeAck"
    }

    static func terminalReason(_ data: Data) -> StreamingTerminalReason? {
        guard let text = String(data: data, encoding: .utf8) else { return nil }
        if text.localizedCaseInsensitiveContains("KickForClosedGame") {
            return .remoteKickClosedGame
        }
        guard let object = try? JSONSerialization.jsonObject(with: data) else { return nil }
        let catalog = messageCatalog(object)
        if catalog.contains(where: { $0.contains("kickforclosedgame") }) {
            return .remoteKickClosedGame
        }
        if catalog.contains(where: { value in
            value == "closed" || value == "close" || value == "sessionclosed"
                || value.contains("/closed") || value.contains("closedgame")
        }) {
            return .remoteClosed
        }
        if catalog.contains(where: { value in
            value == "error" || value.hasSuffix("error") || value.contains("/error")
        }) {
            return .remoteError
        }
        return nil
    }

    private static func messageCatalog(_ value: Any) -> [String] {
        if let dictionary = value as? [String: Any] {
            return dictionary.flatMap { key, value in
                let normalizedKey = key.lowercased()
                var entries = messageCatalog(value)
                if normalizedKey == "error", !(value is NSNull) {
                    entries.append("error")
                } else if normalizedKey == "closed", value as? Bool == true {
                    entries.append("closed")
                }
                return entries
            }
        }
        if let array = value as? [Any] {
            return array.flatMap(messageCatalog)
        }
        if let string = value as? String {
            var result = [string.lowercased()]
            if let nestedData = string.data(using: .utf8),
                let nested = try? JSONSerialization.jsonObject(with: nestedData)
            {
                result.append(contentsOf: messageCatalog(nested))
            }
            return result
        }
        return []
    }
}

struct StreamingDataChannelStateSnapshot: Equatable, Sendable {
    let phases: [StreamingDataChannelLabel: StreamingDataChannelPhase]
    let handshakeSent: Bool
    let handshakeAcknowledged: Bool
    let postHandshakeSentCount: Int
    let preHandshakeControlBootstrapSentCount: Int
    let postHandshakeControlBootstrapSentCount: Int
    let preHandshakeInputMetadataSent: Bool
    let postHandshakeInputMetadataSent: Bool
    let controlReady: Bool
    let gamepadAnnouncementScheduled: Bool
    let inputStarted: Bool
    let terminalReason: StreamingTerminalReason?
}

struct StreamingDataChannelStateMachine: Sendable {
    private let postHandshakeCount: Int
    private let controlBootstrapCount: Int
    private var phases: [StreamingDataChannelLabel: StreamingDataChannelPhase] = Dictionary(
        uniqueKeysWithValues: StreamingDataChannelLabel.allCases.map { ($0, .created) }
    )
    private var handshakeSent = false
    private var handshakeAcknowledged = false
    private var postHandshakeSentCount = 0
    private var preHandshakeControlBootstrapSentCount = 0
    private var postHandshakeControlBootstrapSentCount = 0
    private var preHandshakeInputMetadataSent = false
    private var postHandshakeInputMetadataSent = false
    private var controlReady = false
    private var gamepadAnnouncementScheduled = false
    private var inputStarted = false
    private var inFlightAction: StreamingDataChannelBootstrapAction?
    private(set) var terminalReason: StreamingTerminalReason?

    init(postHandshakeCount: Int, controlBootstrapCount: Int) {
        self.postHandshakeCount = max(0, postHandshakeCount)
        self.controlBootstrapCount = max(0, controlBootstrapCount)
    }

    var snapshot: StreamingDataChannelStateSnapshot {
        StreamingDataChannelStateSnapshot(
            phases: phases,
            handshakeSent: handshakeSent,
            handshakeAcknowledged: handshakeAcknowledged,
            postHandshakeSentCount: postHandshakeSentCount,
            preHandshakeControlBootstrapSentCount: preHandshakeControlBootstrapSentCount,
            postHandshakeControlBootstrapSentCount: postHandshakeControlBootstrapSentCount,
            preHandshakeInputMetadataSent: preHandshakeInputMetadataSent,
            postHandshakeInputMetadataSent: postHandshakeInputMetadataSent,
            controlReady: controlReady,
            gamepadAnnouncementScheduled: gamepadAnnouncementScheduled,
            inputStarted: inputStarted,
            terminalReason: terminalReason
        )
    }

    mutating func channelDidOpen(_ label: StreamingDataChannelLabel) {
        guard terminalReason == nil else { return }
        phases[label] = .open
    }

    @discardableResult
    mutating func channelDidClose(
        _ label: StreamingDataChannelLabel
    ) -> StreamingTerminalReason? {
        phases[label] = .closed
        guard label != .chat, terminalReason == nil else { return nil }
        let reason = StreamingTerminalReason.dataChannelClosed(label)
        terminalReason = reason
        inFlightAction = nil
        return reason
    }

    @discardableResult
    mutating func receiveMessage(_ data: Data) -> StreamingTerminalReason? {
        if let reason = StreamingRemoteMessageClassifier.terminalReason(data) {
            guard terminalReason == nil else { return nil }
            terminalReason = reason
            inFlightAction = nil
            return reason
        }
        if StreamingRemoteMessageClassifier.isHandshakeAck(data), handshakeSent {
            handshakeAcknowledged = true
        }
        return nil
    }

    mutating func nextAction() -> StreamingDataChannelBootstrapAction? {
        guard terminalReason == nil, inFlightAction == nil else { return nil }
        let action: StreamingDataChannelBootstrapAction?
        if !handshakeAcknowledged,
            preHandshakeControlBootstrapSentCount < controlBootstrapCount,
            phases[.control] == .open
        {
            action = .sendControlBootstrap(
                stage: .preHandshake,
                index: preHandshakeControlBootstrapSentCount
            )
        } else if !handshakeAcknowledged,
            !preHandshakeInputMetadataSent,
            phases[.input] == .open
        {
            action = .sendInputMetadata(stage: .preHandshake)
        } else if phases[.message] == .open, !handshakeSent {
            action = .sendMessageHandshake
        } else if handshakeAcknowledged,
            postHandshakeSentCount < postHandshakeCount,
            phases[.message] == .open
        {
            action = .sendPostHandshake(index: postHandshakeSentCount)
        } else if handshakeAcknowledged, postHandshakeSentCount == postHandshakeCount,
            postHandshakeControlBootstrapSentCount < controlBootstrapCount,
            phases[.control] == .open
        {
            action = .sendControlBootstrap(
                stage: .postHandshake,
                index: postHandshakeControlBootstrapSentCount
            )
        } else if handshakeAcknowledged, postHandshakeSentCount == postHandshakeCount,
            postHandshakeControlBootstrapSentCount == controlBootstrapCount,
            !controlReady, phases[.control] == .open
        {
            action = .announceControlReady
        } else if handshakeAcknowledged, postHandshakeSentCount == postHandshakeCount,
            !postHandshakeInputMetadataSent,
            phases[.input] == .open
        {
            action = .sendInputMetadata(stage: .postHandshake)
        } else if controlReady, postHandshakeInputMetadataSent,
            !gamepadAnnouncementScheduled,
            phases[.control] == .open, phases[.input] == .open
        {
            action = .scheduleGamepadAnnouncement
        } else if gamepadAnnouncementScheduled, !inputStarted,
            phases[.control] == .open, phases[.input] == .open
        {
            action = .startInput
        } else {
            action = nil
        }
        inFlightAction = action
        return action
    }

    mutating func actionDidSucceed(_ action: StreamingDataChannelBootstrapAction) {
        guard inFlightAction == action else { return }
        switch action {
        case .sendMessageHandshake:
            handshakeSent = true
        case .sendPostHandshake(let index) where index == postHandshakeSentCount:
            postHandshakeSentCount += 1
        case .sendControlBootstrap(let stage, let index):
            switch stage {
            case .preHandshake where index == preHandshakeControlBootstrapSentCount:
                preHandshakeControlBootstrapSentCount += 1
            case .postHandshake where index == postHandshakeControlBootstrapSentCount:
                postHandshakeControlBootstrapSentCount += 1
                preHandshakeControlBootstrapSentCount = max(
                    preHandshakeControlBootstrapSentCount,
                    index + 1
                )
            default:
                break
            }
        case .sendInputMetadata(let stage):
            switch stage {
            case .preHandshake:
                preHandshakeInputMetadataSent = true
            case .postHandshake:
                preHandshakeInputMetadataSent = true
                postHandshakeInputMetadataSent = true
            }
        case .announceControlReady:
            controlReady = true
        case .scheduleGamepadAnnouncement:
            gamepadAnnouncementScheduled = true
        case .startInput:
            inputStarted = true
        default:
            break
        }
        inFlightAction = nil
    }

    mutating func actionDidFail(_ action: StreamingDataChannelBootstrapAction) {
        guard inFlightAction == action else { return }
        inFlightAction = nil
    }
}

protocol StreamingPeerRuntime: AnyObject, Sendable {
    func makeOffer(
        configuration: StreamingPreparedSignaling,
        iceRestart: Bool
    ) async throws -> String
    func applyAnswer(_ sdp: String) async throws
    func addRemoteCandidates(_ candidates: [StreamingIceCandidate]) async throws
    func debugSnapshot() async -> StreamingPeerDebugSnapshot
    func setAudioMuted(_ muted: Bool) async throws
    func makeMicrophoneOffer(enabled: Bool) async throws -> String
    func stopMicrophoneCapture() async
    func rollbackMediaNegotiation() async throws
    func requestVideoKeyframe() async -> Bool
    func makeQualityOffer(maxVideoBitrateKbps: Int) async throws -> String
    func remoteVideoBitrateCapKbps() async -> Int?
    func stopInputAndHaptics() async
    func closeTransport() async
}

extension StreamingPeerRuntime {
    func makeMicrophoneOffer(enabled: Bool) async throws -> String {
        throw StreamingRuntimeError.microphoneUnavailable
    }
    func stopMicrophoneCapture() async {}
    func rollbackMediaNegotiation() async throws {}
    func requestVideoKeyframe() async -> Bool { false }
    func makeQualityOffer(maxVideoBitrateKbps: Int) async throws -> String {
        throw StreamingRuntimeError.superseded
    }
    func remoteVideoBitrateCapKbps() async -> Int? { nil }
}

protocol StreamingPeerRuntimeFactory: AnyObject, Sendable {
    func makeRuntime(
        eventSink: @escaping @Sendable (StreamingPeerEvent) -> Void
    ) -> any StreamingPeerRuntime
}

enum StreamingRuntimeError: LocalizedError, Equatable {
    case invalidStreamTitleID
    case cloudGameUnavailable
    case cloudEntitlementRequired
    case cloudEntitlementUnconfirmed
    case superseded
    case peerConnectionCreationFailed
    case missingLocalDescription
    case missingRemoteDescription
    case remoteDescriptionTimedOut
    case microphonePermissionDenied
    case microphoneUnavailable
    case mediaNegotiationBusy
    case peerConnectionFailed
    case peerConnectionClosed

    var errorDescription: String? {
        switch self {
        case .invalidStreamTitleID: "串流目标标识无效"
        case .cloudGameUnavailable: "当前云目录暂未提供此游戏的可用启动身份"
        case .cloudEntitlementRequired: "当前账户需要此游戏的云游戏访问权限"
        case .cloudEntitlementUnconfirmed: "游玩资格待确认，请刷新后重试"
        case .superseded: "串流请求已被新的请求替代"
        case .peerConnectionCreationFailed: "创建 WebRTC 连接失败"
        case .missingLocalDescription: "WebRTC 本地协商描述缺失"
        case .missingRemoteDescription: "Xbox 远端协商描述缺失"
        case .remoteDescriptionTimedOut: "应用 Xbox 远端协商描述超时"
        case .microphonePermissionDenied: "麦克风权限未授权"
        case .microphoneUnavailable: "当前设备无法使用麦克风"
        case .mediaNegotiationBusy: "音视频协商正在进行"
        case .peerConnectionFailed: "WebRTC 连接已经失败"
        case .peerConnectionClosed: "WebRTC 连接已经关闭"
        }
    }
}

/// 原生仅投影事实与执行结果；窗口、压力、冷却和效果由公共 Rust 判定。
struct StreamingSupervisionSample: Encodable, Sendable {
    var nowMs: UInt64
    var sampledAtMs: UInt64?
    var sampleId: String?
    var observationFresh = false
    var visible = true
    var isClosing = false
    var connectionState = "connected"
    var controlChannelState = "unknown"
    var connectedElapsedMs: UInt64 = 0
    var inactivityElapsedMs: UInt64 = 0
    var frameIntervalMs: Double?
    var inboundBytesTotal: UInt64?
    var inboundVideoPacketCountTotal: UInt64?
    var videoPresentAgeMs: Double?
    var packetAgeMs: Double?
    var peerGeneration: UInt64 = 0
    var videoGeneration: UInt64 = 0
    var warmupUntilMs: UInt64 = 0
    var previousActionAtMs: UInt64?
    var eligibleSinceMs: UInt64 = 0
    var short: [String: Double] = [:]
    var long: [String: Double] = [:]
    var counters: StreamingMediaCounters?
    var rttMs: Double?
    var jitterMs: Double?
    var jitterBufferMs: Double?
    var pathId: String?
    var qualityLevel: Int = 0
    var qualitySupported = false
    var requestConnectionRecovery = false
    var connectionRecoveryFollowup = false
    var connectionRecoveryActionCompleted = false
    var connectionRecoveryFactKey: String?
    var qualityCompletion: StreamingQualityCompletion?
    var probe: StreamingRecoveryProbe?
    var displayObservation: StreamingDisplayObservation?
    var displayRecoveryCompletion: StreamingDisplayRecoveryCompletion?
}

struct StreamingMediaCounters: Encodable, Sendable {
    var received: UInt64?
    var decoded: UInt64?
    var presented: UInt64?
    var packetsReceived: UInt64?
    var packetsLost: Int64?
}

struct StreamingObservationWindow: Codable, Equatable, Sendable {
    var inboundFpsAvg: Double?
    var decodeFpsAvg: Double?
    var videoPresentFpsAvg: Double?
}

struct StreamingRecoveryProbe: Codable, Sendable {
    struct Baseline: Codable, Sendable {
        var decodeFps: Double?
        var videoFps: Double?
    }
    var action: String
    var startedAtMs: UInt64
    var observeMs: UInt64
    var requiredObserveMs: UInt64
    var sampleId: String?
    var videoGeneration: UInt64?
    var peerGeneration: UInt64?
    var baseline: Baseline
}

struct StreamingQualityAction: Codable, Equatable, Sendable {
    var id: UInt64
    var level: Int
    var reason: String
}
struct StreamingQualityCompletion: Encodable, Sendable {
    var id: UInt64
    var result: String
    var previousLevel: Int
    var completedAtMs: UInt64
}
struct StreamingQualityDecision: Decodable, Equatable, Sendable {
    var state: String
    var reason: String
    var rttBaselineMs: Double?
    var rttDeltaMs: Double?
    var effect: String?
    var action: StreamingQualityAction?
}
struct StreamingRecoveryEffect: Decodable, Equatable, Sendable {
    var result: String
    var reason: String
    var score: Double?
}
struct StreamingSupervisionDecision: Decodable, Equatable, Sendable {
    var fresh: Bool
    var duplicate: Bool
    var stallThresholdMs: UInt64
    var hasInboundProgress: Bool
    var decodeRecoveryEligible: Bool
    var decodePressureSustainedMs: UInt64
    var healthyEpisodeComplete: Bool
    var mediaConnectionEligible: Bool
    var networkPressure: String
    var probeEffect: StreamingRecoveryEffect?
    var short: StreamingObservationWindow
    var quality: StreamingQualityDecision?
    var connectionRecoveryAllowed = false
    var connectionRecoveryExhausted = false
    var connectionRecoveryActive = false
    var displayRecoveryEligible: Bool?
    var displayRecovery: StreamingDisplayRecoveryDecision?
    static let unavailable = Self(
        fresh: false, duplicate: false, stallThresholdMs: 5000, hasInboundProgress: false,
        decodeRecoveryEligible: false, decodePressureSustainedMs: 0, healthyEpisodeComplete: false,
        mediaConnectionEligible: false, networkPressure: "unknown", short: .init())
}
