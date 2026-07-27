import Foundation

enum StreamingLaunchTarget: String, Equatable, Sendable {
    case cloud
    case home
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
        case let .failed(message, _): message
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
        guard case let .failed(_, retryable) = self else { return nil }
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

    var streamTitleID: String { targetID }

    init(
        streamTitleID: String,
        accessHandle: String,
        accountGeneration: UInt64,
        sessionGeneration: UInt64,
        settings: StreamingSessionSettingsSnapshot = .standard,
        attemptID: String = UUID().uuidString,
        accountID: String = "",
        ownerGeneration: UInt64? = nil
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
        ownerGeneration: UInt64? = nil
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

/// WebRtcDirect negotiation 的 iOS 可消费投影，由 Rust plan 生成并保留桌面默认 fixture。
struct StreamingWebRtcPlan: Equatable, Sendable {
    let audioDirection: StreamingMediaDirection
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
    let minVideoBitrateKbps: Int?
    let startVideoBitrateKbps: Int?
    let maxVideoBitrateKbps: Int?
    let stereoAudio: Bool
    let requiredVideoRtcpFeedback: [String]
    let allowedCandidateTypes: [String]
    let iceTransportPolicy: String
    let preferIPv6: Bool
    let normalizeEndOfCandidates: Bool

    static let desktopWebRtcDirect = StreamingWebRtcPlan()

    init(
        audioDirection: StreamingMediaDirection = .sendReceive,
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
        normalizeEndOfCandidates: Bool = true
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
    }
}

struct StreamingPreparedSignaling: Equatable, Sendable {
    let iceServers: [StreamingIceServer]
    let webRtcPlan: StreamingWebRtcPlan

    init(
        iceServers: [StreamingIceServer],
        webRtcPlan: StreamingWebRtcPlan = .desktopWebRtcDirect
    ) {
        self.iceServers = iceServers
        self.webRtcPlan = webRtcPlan
    }
}

struct StreamingIceCandidate: Equatable, Sendable {
    let sdp: String
    let sdpMid: String?
    let sdpMLineIndex: Int32
}

struct StreamingRemoteIceBatch: Equatable, Sendable {
    let candidates: [StreamingIceCandidate]
    let endOfCandidates: Bool
}

/// Rust opaque session 的窄桥接面。Swift 负责 libwebrtc 输入输出，所有远端会话策略留在 Rust。
protocol StreamingControlSession: Sendable {
    func prepareSignaling() async throws -> StreamingPreparedSignaling
    func exchangeOffer(_ sdp: String) async throws -> String
    func submitLocalCandidates(_ candidates: [StreamingIceCandidate]) async throws
    func completeLocalIceGathering() async throws
    func nextRemoteIceBatch() async throws -> StreamingRemoteIceBatch
    func markConnected() async throws
    func close() async
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
}

final class StreamingVideoTrackHandle: @unchecked Sendable {
    let rawValue: AnyObject
    let traceContext: StreamingPresentationTraceContext?

    init(
        rawValue: AnyObject,
        traceContext: StreamingPresentationTraceContext? = nil
    ) {
        self.rawValue = rawValue
        self.traceContext = traceContext
    }
}

enum StreamingPeerEvent: @unchecked Sendable {
    case localCandidate(StreamingIceCandidate)
    case localIceGatheringComplete
    case connectionStateChanged(StreamingPeerConnectionState)
    case videoTrack(StreamingVideoTrackHandle)
    case firstVideoFrame
    case audioTrackReady
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
        case let .dataChannelClosed(label): "dataChannel.closed.\(label.rawValue)"
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
        case let .dataChannelClosed(label): "Xbox 串流通道已关闭（\(label.rawValue)）"
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
        case let .sendPostHandshake(index) where index == postHandshakeSentCount:
            postHandshakeSentCount += 1
        case let .sendControlBootstrap(stage, index):
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
        case let .sendInputMetadata(stage):
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
    func stopInputAndHaptics() async
    func closeTransport() async
}

protocol StreamingPeerRuntimeFactory: AnyObject, Sendable {
    func makeRuntime(
        eventSink: @escaping @Sendable (StreamingPeerEvent) -> Void
    ) -> any StreamingPeerRuntime
}

enum StreamingRuntimeError: LocalizedError, Equatable {
    case invalidStreamTitleID
    case superseded
    case webRTCUnavailable
    case peerConnectionCreationFailed
    case missingLocalDescription
    case missingRemoteDescription
    case remoteDescriptionTimedOut
    case peerConnectionFailed
    case peerConnectionClosed

    var errorDescription: String? {
        switch self {
        case .invalidStreamTitleID: "串流目标标识无效"
        case .superseded: "串流请求已被新的请求替代"
        case .webRTCUnavailable: "当前构建缺少 WebRTC 运行时"
        case .peerConnectionCreationFailed: "创建 WebRTC 连接失败"
        case .missingLocalDescription: "WebRTC 本地协商描述缺失"
        case .missingRemoteDescription: "Xbox 远端协商描述缺失"
        case .remoteDescriptionTimedOut: "应用 Xbox 远端协商描述超时"
        case .peerConnectionFailed: "WebRTC 连接已经失败"
        case .peerConnectionClosed: "WebRTC 连接已经关闭"
        }
    }
}
