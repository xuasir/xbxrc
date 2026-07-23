import AVFoundation
import Foundation

struct StreamingOfferSdpProjector {
    func project(_ sdp: String, plan: StreamingWebRtcPlan) -> String {
        var lines = sdp
            .replacingOccurrences(of: "\r\n", with: "\n")
            .split(separator: "\n", omittingEmptySubsequences: false)
            .map(String.init)
        if lines.last == "" { lines.removeLast() }

        if plan.videoCodecMimeType.caseInsensitiveCompare("video/H264") == .orderedSame {
            let h264Payloads = h264PayloadTypes(in: lines)
            reorderVideoPayloads(lines: &lines, h264Payloads: h264Payloads, plan: plan)
            patchH264Parameters(lines: &lines, h264Payloads: h264Payloads, plan: plan)
            patchVideoFeedback(lines: &lines, h264Payloads: h264Payloads, plan: plan)
        }
        patchMediaBitrate(
            lines: &lines,
            media: "video",
            bitrateKbps: plan.maxVideoBitrateKbps
        )
        if plan.stereoAudio {
            patchOpusStereo(lines: &lines)
        }
        return lines.joined(separator: "\r\n") + "\r\n"
    }

    private func h264PayloadTypes(in lines: [String]) -> Set<String> {
        Set(lines.compactMap { line in
            guard line.hasPrefix("a=rtpmap:"),
                  line.localizedCaseInsensitiveContains(" H264/")
            else { return nil }
            return line.dropFirst("a=rtpmap:".count).split(separator: " ").first.map(String.init)
        })
    }

    private func reorderVideoPayloads(
        lines: inout [String],
        h264Payloads: Set<String>,
        plan: StreamingWebRtcPlan
    ) {
        let profileEntries: [(String, String)] = lines.compactMap { line in
            guard line.hasPrefix("a=fmtp:") else { return nil }
            let parts = line.dropFirst("a=fmtp:".count).split(separator: " ", maxSplits: 1)
            guard parts.count == 2,
                  let profile = parameter(named: "profile-level-id", in: String(parts[1]))
            else { return nil }
            return (String(parts[0]), profile.lowercased())
        }
        let profileByPayload = Dictionary<String, String>(uniqueKeysWithValues: profileEntries)
        let profileOrder = plan.h264Profiles.map { $0.lowercased() }
        let preferredPayloads = h264Payloads.sorted { left, right in
            rank(profileByPayload[left], profileOrder: profileOrder)
                < rank(profileByPayload[right], profileOrder: profileOrder)
        }
        guard let index = lines.firstIndex(where: { $0.hasPrefix("m=video ") }) else { return }
        let fields = lines[index].split(separator: " ").map(String.init)
        guard fields.count > 3 else { return }
        let remaining = fields.dropFirst(3).filter { !h264Payloads.contains($0) }
        lines[index] = (Array(fields.prefix(3)) + preferredPayloads + remaining).joined(separator: " ")
    }

    private func patchH264Parameters(
        lines: inout [String],
        h264Payloads: Set<String>,
        plan: StreamingWebRtcPlan
    ) {
        for index in lines.indices where lines[index].hasPrefix("a=fmtp:") {
            let prefix = "a=fmtp:"
            let parts = lines[index].dropFirst(prefix.count).split(separator: " ", maxSplits: 1)
            guard parts.count == 2, h264Payloads.contains(String(parts[0])) else { continue }
            var parameters = orderedParameters(String(parts[1]))
            setParameter(
                "packetization-mode",
                value: String(plan.h264PacketizationMode),
                parameters: &parameters
            )
            setParameter(
                "level-asymmetry-allowed",
                value: plan.h264LevelAsymmetryAllowed ? "1" : "0",
                parameters: &parameters
            )
            setParameter("max-fs", value: String(plan.maxFrameSize), parameters: &parameters)
            setParameter("max-fr", value: String(plan.maxFrameRate), parameters: &parameters)
            setOptionalBitrate("x-google-min-bitrate", plan.minVideoBitrateKbps, &parameters)
            setOptionalBitrate("x-google-start-bitrate", plan.startVideoBitrateKbps, &parameters)
            setOptionalBitrate("x-google-max-bitrate", plan.maxVideoBitrateKbps, &parameters)
            lines[index] = "\(prefix)\(parts[0]) \(parameters.map(\.value).joined(separator: ";"))"
        }
    }

    private func patchVideoFeedback(
        lines: inout [String],
        h264Payloads: Set<String>,
        plan: StreamingWebRtcPlan
    ) {
        guard let videoStart = lines.firstIndex(where: { $0.hasPrefix("m=video ") }) else { return }
        let videoEnd = mediaEndIndex(in: lines, after: videoStart)
        var insertionIndex = videoEnd
        for payload in h264Payloads.sorted() {
            for feedback in plan.requiredVideoRtcpFeedback {
                let line = "a=rtcp-fb:\(payload) \(feedback)"
                if !lines[videoStart..<videoEnd].contains(line) {
                    lines.insert(line, at: insertionIndex)
                    insertionIndex += 1
                }
            }
        }
    }

    private func patchMediaBitrate(lines: inout [String], media: String, bitrateKbps: Int?) {
        guard let bitrateKbps, bitrateKbps > 0,
              let start = lines.firstIndex(where: { $0.hasPrefix("m=\(media) ") })
        else { return }
        let end = mediaEndIndex(in: lines, after: start)
        if let existing = lines[start..<end].firstIndex(where: { $0.hasPrefix("b=AS:") }) {
            lines[existing] = "b=AS:\(bitrateKbps)"
        } else {
            lines.insert("b=AS:\(bitrateKbps)", at: start + 1)
        }
    }

    private func patchOpusStereo(lines: inout [String]) {
        let opusPayloads = Set<String>(lines.compactMap { line in
            guard line.hasPrefix("a=rtpmap:"),
                  line.localizedCaseInsensitiveContains(" opus/")
            else { return nil }
            return line.dropFirst("a=rtpmap:".count).split(separator: " ").first.map(String.init)
        })
        for index in lines.indices where lines[index].hasPrefix("a=fmtp:") {
            let parts = lines[index].dropFirst("a=fmtp:".count).split(separator: " ", maxSplits: 1)
            guard parts.count == 2, opusPayloads.contains(String(parts[0])) else { continue }
            var parameters = orderedParameters(String(parts[1]))
            setParameter("stereo", value: "1", parameters: &parameters)
            lines[index] = "a=fmtp:\(parts[0]) \(parameters.map(\.value).joined(separator: ";"))"
        }
    }

    private func orderedParameters(_ raw: String) -> [(key: String, value: String)] {
        raw.split(separator: ";").map { part in
            let value = part.trimmingCharacters(in: .whitespaces)
            return (value.split(separator: "=", maxSplits: 1).first.map(String.init)?.lowercased() ?? value, value)
        }
    }

    private func setOptionalBitrate(
        _ key: String,
        _ value: Int?,
        _ parameters: inout [(key: String, value: String)]
    ) {
        guard let value, value > 0 else { return }
        setParameter(key, value: String(value), parameters: &parameters)
    }

    private func setParameter(
        _ key: String,
        value: String,
        parameters: inout [(key: String, value: String)]
    ) {
        let normalized = key.lowercased()
        if let index = parameters.firstIndex(where: { $0.key == normalized }) {
            parameters[index] = (normalized, "\(normalized)=\(value)")
        } else {
            parameters.append((normalized, "\(normalized)=\(value)"))
        }
    }

    private func parameter(named name: String, in raw: String) -> String? {
        let prefix = "\(name.lowercased())="
        return raw.split(separator: ";")
            .map { $0.trimmingCharacters(in: .whitespaces) }
            .first { $0.lowercased().hasPrefix(prefix) }
            .map { String($0.dropFirst(prefix.count)) }
    }

    private func rank(_ profile: String?, profileOrder: [String]) -> Int {
        guard let profile else { return profileOrder.count }
        return profileOrder.firstIndex(where: { profile.hasPrefix($0) }) ?? profileOrder.count
    }

    private func mediaEndIndex(in lines: [String], after start: Int) -> Int {
        let next = start + 1
        guard next < lines.endIndex else { return lines.endIndex }
        return lines[next..<lines.endIndex].firstIndex(where: { $0.hasPrefix("m=") })
            ?? lines.endIndex
    }
}

struct StreamingIceCandidateDescriptor: Equatable, Sendable {
    let candidateType: String?
    let transportProtocol: String?
    let addressFamily: String?
}

enum StreamingNativeIceCandidateAdapter {
    static func adapt(_ candidate: StreamingIceCandidate) -> StreamingIceCandidate? {
        let trimmed = candidate.sdp.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return nil }

        let nativeSdp = if trimmed.range(
            of: "a=",
            options: [.anchored, .caseInsensitive]
        ) != nil {
            String(trimmed.dropFirst(2)).trimmingCharacters(in: .whitespacesAndNewlines)
        } else {
            trimmed
        }
        guard !nativeSdp.isEmpty,
              nativeSdp.caseInsensitiveCompare("end-of-candidates") != .orderedSame
        else { return nil }

        let fields = nativeSdp.split(whereSeparator: \Character.isWhitespace).map(String.init)
        guard let prefix = fields.first,
              prefix.range(of: "candidate:", options: [.anchored, .caseInsensitive]) != nil
        else { return nil }

        let usesUDP = fields.indices.contains(2)
            && fields[2].caseInsensitiveCompare("udp") == .orderedSame
        let hasTCPType = fields.contains {
            $0.caseInsensitiveCompare("tcptype") == .orderedSame
                || $0.lowercased().hasPrefix("tcptype=")
        }
        guard !(usesUDP && hasTCPType) else { return nil }

        return StreamingIceCandidate(
            sdp: nativeSdp,
            sdpMid: candidate.sdpMid,
            sdpMLineIndex: candidate.sdpMLineIndex
        )
    }
}

enum StreamingIceCandidatePolicy {
    static func descriptor(_ candidate: StreamingIceCandidate) -> StreamingIceCandidateDescriptor {
        let fields = candidate.sdp.split(whereSeparator: \Character.isWhitespace).map(String.init)
        let typeIndex = fields.firstIndex { $0.caseInsensitiveCompare("typ") == .orderedSame }
        let candidateType = typeIndex.flatMap { index in
            fields.indices.contains(index + 1) ? fields[index + 1].lowercased() : nil
        }
        let transportProtocol = fields.indices.contains(2) ? fields[2].lowercased() : nil
        let address = fields.indices.contains(4) ? fields[4] : ""
        let addressFamily: String? = if address.contains(":") {
            "ipv6"
        } else if address.split(separator: ".").count == 4 {
            "ipv4"
        } else {
            nil
        }
        return StreamingIceCandidateDescriptor(
            candidateType: candidateType,
            transportProtocol: transportProtocol,
            addressFamily: addressFamily
        )
    }

    static func allows(_ candidate: StreamingIceCandidate, plan: StreamingWebRtcPlan) -> Bool {
        guard candidate.sdp != "a=end-of-candidates" else { return false }
        let type = descriptor(candidate).candidateType
        return type.map { plan.allowedCandidateTypes.contains($0) } ?? true
    }

    static func ordered(
        _ candidates: [StreamingIceCandidate],
        plan: StreamingWebRtcPlan
    ) -> [StreamingIceCandidate] {
        candidates.enumerated().filter { allows($0.element, plan: plan) }.sorted { left, right in
            let leftDescriptor = descriptor(left.element)
            let rightDescriptor = descriptor(right.element)
            let leftKey = sortKey(leftDescriptor, plan: plan, originalIndex: left.offset)
            let rightKey = sortKey(rightDescriptor, plan: plan, originalIndex: right.offset)
            return leftKey.lexicographicallyPrecedes(rightKey)
        }.map(\.element)
    }

    private static func sortKey(
        _ descriptor: StreamingIceCandidateDescriptor,
        plan: StreamingWebRtcPlan,
        originalIndex: Int
    ) -> [Int] {
        let typeRank = descriptor.candidateType.flatMap(plan.allowedCandidateTypes.firstIndex) ?? Int.max
        let familyRank = switch descriptor.addressFamily {
        case "ipv6": plan.preferIPv6 ? 0 : 1
        case "ipv4": plan.preferIPv6 ? 1 : 0
        default: 2
        }
        let protocolRank = descriptor.transportProtocol == "udp" ? 0 : 1
        return [typeRank, familyRank, protocolRank, originalIndex]
    }
}

#if canImport(WebRTC)
@preconcurrency import WebRTC

@MainActor
final class LibWebRTCPeerRuntimeFactory: StreamingPeerRuntimeFactory {
    func makeRuntime(
        eventSink: @escaping @MainActor @Sendable (StreamingPeerEvent) -> Void
    ) -> any StreamingPeerRuntime {
        LibWebRTCPeerRuntime(eventSink: eventSink)
    }
}

@MainActor
private final class LibWebRTCPeerRuntime: NSObject, StreamingPeerRuntime {
    private struct StatsProjectionState {
        var firstMediaAtMilliseconds: Double?
        var lastVideoBytes: UInt64?
        var lastFramesDecoded: UInt64?
        var lastObservedAtMilliseconds: Double?
    }

    private static let factory: RTCPeerConnectionFactory = {
        RTCInitializeSSL()
        return RTCPeerConnectionFactory(
            encoderFactory: RTCDefaultVideoEncoderFactory(),
            decoderFactory: RTCDefaultVideoDecoderFactory()
        )
    }()

    private let eventSink: @MainActor @Sendable (StreamingPeerEvent) -> Void
    private var peerConnection: RTCPeerConnection?
    private var frameProbe: FirstVideoFrameProbe?
    private var dataChannels: XboxStreamDataChannels?
    private var statsTask: Task<Void, Never>?
    private var statsProjectionState = StatsProjectionState()
    private var activePlan: StreamingWebRtcPlan?

    init(eventSink: @escaping @MainActor @Sendable (StreamingPeerEvent) -> Void) {
        self.eventSink = eventSink
    }

    func makeOffer(
        configuration prepared: StreamingPreparedSignaling,
        iceRestart: Bool
    ) async throws -> String {
        activePlan = prepared.webRtcPlan
        if iceRestart {
            guard let peerConnection else {
                throw StreamingRuntimeError.peerConnectionCreationFailed
            }
            return try await createAndApplyOffer(
                on: peerConnection,
                plan: prepared.webRtcPlan,
                iceRestart: true
            )
        }

        try configureAudioSession()
        statsProjectionState = StatsProjectionState()
        let configuration = RTCConfiguration()
        configuration.sdpSemantics = .unifiedPlan
        configuration.continualGatheringPolicy = .gatherOnce
        configuration.iceTransportPolicy = prepared.webRtcPlan.iceTransportPolicy == "relay"
            ? .relay
            : .all
        configuration.iceServers = prepared.iceServers.map { server in
            if let username = server.username, let credential = server.credential {
                RTCIceServer(
                    urlStrings: server.urls,
                    username: username,
                    credential: credential
                )
            } else {
                RTCIceServer(urlStrings: server.urls)
            }
        }
        let constraints = RTCMediaConstraints(
            mandatoryConstraints: nil,
            optionalConstraints: ["DtlsSrtpKeyAgreement": "true"]
        )
        guard let peer = Self.factory.peerConnection(
            with: configuration,
            constraints: constraints,
            delegate: self
        ) else {
            throw StreamingRuntimeError.peerConnectionCreationFailed
        }
        peerConnection = peer

        let audioInit = RTCRtpTransceiverInit()
        audioInit.direction = rtcDirection(prepared.webRtcPlan.audioDirection)
        let audioTransceiver = peer.addTransceiver(of: .audio, init: audioInit)
        let videoInit = RTCRtpTransceiverInit()
        videoInit.direction = rtcDirection(prepared.webRtcPlan.videoDirection)
        let videoTransceiver = peer.addTransceiver(of: .video, init: videoInit)
        try preferCodecs(audioTransceiver: audioTransceiver, videoTransceiver: videoTransceiver)

        let channels = XboxStreamDataChannels(
            targetVideoWidth: prepared.webRtcPlan.targetVideoWidth,
            targetVideoHeight: prepared.webRtcPlan.targetVideoHeight,
            eventSink: { [weak self] label, event in
                IOSRuntimeTrace.state(
                    domain: "ios-streaming",
                    event: "dataChannelStateChanged",
                    payload: [
                        "label": .string(label),
                        "event": .string(event),
                    ],
                    dimension: .network,
                    importance: .debug
                )
                self?.eventSink(.dataChannelEvent(label: label, event: event))
            },
            failureSink: { [weak self] message in
                self?.eventSink(.failed(message))
            }
        )
        channels.createChannels(on: peer)
        dataChannels = channels
        eventSink(.dataChannelEvent(label: "all", event: "profilesCreated"))

        return try await createAndApplyOffer(
            on: peer,
            plan: prepared.webRtcPlan,
            iceRestart: false
        )
    }

    private func createAndApplyOffer(
        on peer: RTCPeerConnection,
        plan: StreamingWebRtcPlan,
        iceRestart: Bool
    ) async throws -> String {
        let offerConstraints = RTCMediaConstraints(
            mandatoryConstraints: iceRestart ? ["IceRestart": "true"] : nil,
            optionalConstraints: nil
        )
        let offer = try await withCheckedThrowingContinuation {
            (continuation: CheckedContinuation<RTCSessionDescription, Error>) in
            peer.offer(for: offerConstraints) {
                description, error in
                if let error {
                    continuation.resume(throwing: error)
                } else if let description {
                    continuation.resume(returning: description)
                } else {
                    continuation.resume(throwing: StreamingRuntimeError.missingLocalDescription)
                }
            }
        }
        let projectedOffer = RTCSessionDescription(
            type: offer.type,
            sdp: StreamingOfferSdpProjector().project(offer.sdp, plan: plan)
        )
        try await setLocalDescription(projectedOffer, on: peer)
        return projectedOffer.sdp
    }

    func applyAnswer(_ sdp: String) async throws {
        guard let peerConnection else {
            throw StreamingRuntimeError.peerConnectionCreationFailed
        }
        let startedAt = Date()
        IOSRuntimeTrace.state(
            domain: "ios-streaming",
            event: "remoteDescriptionApplyStarted",
            payload: ["answerBytes": .integer(Int64(sdp.utf8.count))],
            dimension: .network,
            importance: .key
        )
        let answer = RTCSessionDescription(type: .answer, sdp: sdp)
        try await withCheckedThrowingContinuation {
            (continuation: CheckedContinuation<Void, Error>) in
            peerConnection.setRemoteDescription(answer) { error in
                IOSRuntimeTrace.state(
                    domain: "ios-streaming",
                    event: "remoteDescriptionApplyCallback",
                    payload: [
                        "elapsedMs": .integer(
                            Int64(Date().timeIntervalSince(startedAt) * 1_000)
                        ),
                        "hasError": .bool(error != nil),
                    ],
                    dimension: .network,
                    importance: .key
                )
                if let error {
                    continuation.resume(throwing: error)
                } else {
                    continuation.resume(returning: ())
                }
            }
            IOSRuntimeTrace.state(
                domain: "ios-streaming",
                event: "remoteDescriptionSetDispatched",
                payload: [
                    "elapsedMs": .integer(Int64(Date().timeIntervalSince(startedAt) * 1_000)),
                ],
                dimension: .network,
                importance: .key
            )
        }
        IOSRuntimeTrace.state(
            domain: "ios-streaming",
            event: "remoteDescriptionApplyCompleted",
            payload: [
                "elapsedMs": .integer(Int64(Date().timeIntervalSince(startedAt) * 1_000)),
            ],
            dimension: .network,
            importance: .key
        )
        startStatsCollection()
    }

    func addRemoteCandidates(_ candidates: [StreamingIceCandidate]) async throws {
        guard let peerConnection else {
            throw StreamingRuntimeError.peerConnectionCreationFailed
        }
        let plan = activePlan ?? .desktopWebRtcDirect
        let nativeCandidates = candidates.compactMap(StreamingNativeIceCandidateAdapter.adapt)
        let orderedCandidates = StreamingIceCandidatePolicy.ordered(nativeCandidates, plan: plan)
        var appliedCount = 0
        var addFailureCount = 0
        for candidate in orderedCandidates {
            let rtcCandidate = RTCIceCandidate(
                sdp: candidate.sdp,
                sdpMLineIndex: candidate.sdpMLineIndex,
                sdpMid: candidate.sdpMid
            )
            do {
                try await withCheckedThrowingContinuation {
                    (continuation: CheckedContinuation<Void, Error>) in
                    peerConnection.add(rtcCandidate) { error in
                        if let error {
                            continuation.resume(throwing: error)
                        } else {
                            continuation.resume(returning: ())
                        }
                    }
                }
                appliedCount += 1
            } catch {
                addFailureCount += 1
            }
        }
        IOSRuntimeTrace.event(
            domain: "ios-streaming",
            event: "remoteIceCandidatesApplied",
            payload: [
                "inputCount": .integer(Int64(candidates.count)),
                "nativeCandidateCount": .integer(Int64(nativeCandidates.count)),
                "orderedCandidateCount": .integer(Int64(orderedCandidates.count)),
                "appliedCount": .integer(Int64(appliedCount)),
                "addFailureCount": .integer(Int64(addFailureCount)),
            ],
            dimension: .network,
            importance: .key
        )
    }

    func stopInputAndHaptics() async {
        await dataChannels?.closeAfterInputDrain()
        dataChannels = nil
    }

    func closeTransport() async {
        statsTask?.cancel()
        statsTask = nil
        if let frameProbe, let track = peerConnection?.transceivers
            .compactMap({ $0.receiver.track as? RTCVideoTrack }).first {
            track.remove(frameProbe)
        }
        frameProbe = nil
        peerConnection?.close()
        peerConnection = nil
        activePlan = nil
        statsProjectionState = StatsProjectionState()
        deactivateAudioSession()
    }

    private func preferCodecs(
        audioTransceiver: RTCRtpTransceiver?,
        videoTransceiver: RTCRtpTransceiver?
    ) throws {
        if let audioTransceiver {
            let capabilities = Self.factory.rtpSenderCapabilities(
                forKind: kRTCMediaStreamTrackKindAudio
            )
            let codecs = capabilities.codecs.sorted {
                ($0.name.caseInsensitiveCompare("opus") == .orderedSame ? 0 : 1)
                    < ($1.name.caseInsensitiveCompare("opus") == .orderedSame ? 0 : 1)
            }
            try audioTransceiver.setCodecPreferences(codecs, error: ())
        }
        if let videoTransceiver {
            let capabilities = Self.factory.rtpSenderCapabilities(
                forKind: kRTCMediaStreamTrackKindVideo
            )
            let codecs = capabilities.codecs.sorted {
                Self.videoCodecRank($0.name) < Self.videoCodecRank($1.name)
            }
            try videoTransceiver.setCodecPreferences(codecs, error: ())
        }
    }

    private static func videoCodecRank(_ name: String) -> Int {
        switch name.lowercased() {
        case "h264": 0
        case "rtx": 1
        case "red": 2
        case "ulpfec": 3
        default: 10
        }
    }

    private func rtcDirection(_ direction: StreamingMediaDirection) -> RTCRtpTransceiverDirection {
        switch direction {
        case .sendReceive: .sendRecv
        case .receiveOnly: .recvOnly
        }
    }

    private func setLocalDescription(
        _ description: RTCSessionDescription,
        on peer: RTCPeerConnection
    ) async throws {
        try await withCheckedThrowingContinuation {
            (continuation: CheckedContinuation<Void, Error>) in
            peer.setLocalDescription(description) { error in
                if let error {
                    continuation.resume(throwing: error)
                } else {
                    continuation.resume(returning: ())
                }
            }
        }
    }

    private func configureAudioSession() throws {
        let session = RTCAudioSession.sharedInstance()
        let configuration = RTCAudioSessionConfiguration.webRTC()
        configuration.categoryOptions = [.allowBluetoothHFP, .allowBluetoothA2DP, .allowAirPlay]
        session.lockForConfiguration()
        defer { session.unlockForConfiguration() }
        try session.setConfiguration(configuration, active: true)
        session.isAudioEnabled = true
        IOSRuntimeTrace.state(
            domain: "ios-streaming",
            event: "audioSessionActivated",
            payload: ["mode": .string(configuration.mode)],
            dimension: .mediaSupply,
            importance: .key
        )
    }

    private func deactivateAudioSession() {
        let session = RTCAudioSession.sharedInstance()
        session.lockForConfiguration()
        defer { session.unlockForConfiguration() }
        session.isAudioEnabled = false
        try? session.setActive(false)
    }

    private func attachVideoTrack(_ track: RTCVideoTrack) {
        let probe = FirstVideoFrameProbe { [weak self] in
            self?.eventSink(.firstVideoFrame)
        }
        frameProbe = probe
        track.add(probe)
        eventSink(.videoTrack(StreamingVideoTrackHandle(rawValue: track)))
    }

    private func startStatsCollection() {
        statsTask?.cancel()
        statsTask = Task { [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(for: .seconds(1))
                guard let self, !Task.isCancelled else { return }
                self.collectStats()
            }
        }
    }

    private func collectStats() {
        peerConnection?.statistics { [weak self] report in
            Task { @MainActor [weak self] in
                guard let self else { return }
                self.eventSink(.stats(self.projectStats(report)))
            }
        }
    }

    private func projectStats(_ report: RTCStatisticsReport) -> StreamingPeerStats {
        let observedAtMilliseconds = Date().timeIntervalSince1970 * 1_000
        var inboundVideoBytes: UInt64 = 0
        var inboundAudioBytes: UInt64 = 0
        var framesDecoded: UInt64 = 0
        var framesDropped: UInt64 = 0
        var packetsLost: Int64 = 0
        var packetsReceived: UInt64 = 0
        var jitterSeconds: Double?
        var freezeCount: UInt64?
        var freezeDurationSeconds: Double?
        var nackCount: UInt64?
        var pliCount: UInt64?
        var firCount: UInt64?
        var roundTripTimeSeconds: Double?
        var selectedCandidatePairProtocol: String?
        var selectedCandidatePairAddressFamily: String?
        var selectedLocalCandidateType: String?
        var selectedRemoteCandidateType: String?
        for statistic in report.statistics.values {
            if statistic.type == "inbound-rtp" {
                let kind = Self.stringValue(statistic.values["kind"])
                    ?? Self.stringValue(statistic.values["mediaType"])
                let bytes = Self.uintValue(statistic.values["bytesReceived"])
                if kind == "video" {
                    inboundVideoBytes += bytes
                    framesDecoded += Self.uintValue(statistic.values["framesDecoded"])
                    framesDropped += Self.uintValue(statistic.values["framesDropped"])
                    packetsReceived += Self.uintValue(statistic.values["packetsReceived"])
                    jitterSeconds = Self.doubleValue(statistic.values["jitter"])
                    freezeCount = Self.optionalUIntValue(statistic.values["freezeCount"])
                    freezeDurationSeconds = Self.doubleValue(
                        statistic.values["totalFreezesDuration"]
                    )
                    nackCount = Self.optionalUIntValue(statistic.values["nackCount"])
                    pliCount = Self.optionalUIntValue(statistic.values["pliCount"])
                    firCount = Self.optionalUIntValue(statistic.values["firCount"])
                } else if kind == "audio" {
                    inboundAudioBytes += bytes
                }
                packetsLost += Self.intValue(statistic.values["packetsLost"])
            } else if statistic.type == "candidate-pair",
                      Self.stringValue(statistic.values["state"]) == "succeeded",
                      Self.boolValue(statistic.values["nominated"])
                        || Self.boolValue(statistic.values["selected"])
            {
                roundTripTimeSeconds = Self.doubleValue(
                    statistic.values["currentRoundTripTime"]
                )
                let pair = Self.projectCandidatePair(statistic, report: report)
                selectedCandidatePairProtocol = pair.transportProtocol
                selectedCandidatePairAddressFamily = pair.addressFamily
                selectedLocalCandidateType = pair.localCandidateType
                selectedRemoteCandidateType = pair.remoteCandidateType
            }
        }

        let mediaPresent = inboundVideoBytes > 0 || packetsReceived > 0 || framesDecoded > 0
        if mediaPresent, statsProjectionState.firstMediaAtMilliseconds == nil {
            statsProjectionState.firstMediaAtMilliseconds = observedAtMilliseconds
        }
        let receiveBitrateBps = Self.ratePerSecond(
            current: inboundVideoBytes,
            previous: statsProjectionState.lastVideoBytes,
            elapsedMilliseconds: statsProjectionState.lastObservedAtMilliseconds.map {
                observedAtMilliseconds - $0
            },
            multiplier: 8
        )
        let frameSupplyDelta = Self.signedDelta(
            current: framesDecoded,
            previous: statsProjectionState.lastFramesDecoded
        )
        statsProjectionState.lastVideoBytes = inboundVideoBytes
        statsProjectionState.lastFramesDecoded = framesDecoded
        statsProjectionState.lastObservedAtMilliseconds = observedAtMilliseconds

        return StreamingPeerStats(
            inboundVideoBytes: inboundVideoBytes,
            inboundAudioBytes: inboundAudioBytes,
            framesDecoded: framesDecoded,
            framesDropped: framesDropped,
            packetsLost: packetsLost,
            roundTripTimeSeconds: roundTripTimeSeconds,
            jitterSeconds: jitterSeconds,
            packetsReceived: packetsReceived,
            receiveBitrateBps: receiveBitrateBps,
            freezeCount: freezeCount,
            freezeDurationSeconds: freezeDurationSeconds,
            nackCount: nackCount,
            pliCount: pliCount,
            firCount: firCount,
            selectedCandidatePairProtocol: selectedCandidatePairProtocol,
            selectedCandidatePairAddressFamily: selectedCandidatePairAddressFamily,
            selectedLocalCandidateType: selectedLocalCandidateType,
            selectedRemoteCandidateType: selectedRemoteCandidateType,
            firstMediaAtMilliseconds: statsProjectionState.firstMediaAtMilliseconds,
            lastMediaAtMilliseconds: mediaPresent ? observedAtMilliseconds : nil,
            frameSupplyDelta: frameSupplyDelta
        )
    }

    private static func projectCandidatePair(
        _ pair: RTCStatistics,
        report: RTCStatisticsReport
    ) -> (
        transportProtocol: String?,
        addressFamily: String?,
        localCandidateType: String?,
        remoteCandidateType: String?
    ) {
        let local = stringValue(pair.values["localCandidateId"])
            .flatMap { report.statistics[$0] }
        let remote = stringValue(pair.values["remoteCandidateId"])
            .flatMap { report.statistics[$0] }
        let transportProtocol = stringValue(local?.values["protocol"])
            ?? stringValue(remote?.values["protocol"])
        let localFamily = candidateAddressFamily(local)
        let remoteFamily = candidateAddressFamily(remote)
        let addressFamily: String? = if localFamily == nil, remoteFamily == nil {
            nil
        } else {
            "local:\(localFamily ?? "unknown"),remote:\(remoteFamily ?? "unknown")"
        }
        return (
            transportProtocol?.lowercased(),
            addressFamily,
            stringValue(local?.values["candidateType"])?.lowercased(),
            stringValue(remote?.values["candidateType"])?.lowercased()
        )
    }

    private static func candidateAddressFamily(_ statistic: RTCStatistics?) -> String? {
        guard let address = stringValue(statistic?.values["address"])
            ?? stringValue(statistic?.values["ip"])
        else { return nil }
        if address.contains(":") { return "ipv6" }
        if address.split(separator: ".").count == 4 { return "ipv4" }
        return "unknown"
    }

    private static func ratePerSecond(
        current: UInt64,
        previous: UInt64?,
        elapsedMilliseconds: Double?,
        multiplier: Double
    ) -> Double? {
        guard let previous, current >= previous,
              let elapsedMilliseconds, elapsedMilliseconds > 0
        else { return nil }
        return Double(current - previous) * multiplier * 1_000 / elapsedMilliseconds
    }

    private static func signedDelta(current: UInt64, previous: UInt64?) -> Int64? {
        guard let previous, current >= previous else { return nil }
        return Int64(clamping: current - previous)
    }

    nonisolated private static func stringValue(_ value: NSObject?) -> String? {
        value as? String
    }

    nonisolated private static func uintValue(_ value: NSObject?) -> UInt64 {
        (value as? NSNumber)?.uint64Value ?? 0
    }

    nonisolated private static func optionalUIntValue(_ value: NSObject?) -> UInt64? {
        (value as? NSNumber)?.uint64Value
    }

    nonisolated private static func intValue(_ value: NSObject?) -> Int64 {
        (value as? NSNumber)?.int64Value ?? 0
    }

    nonisolated private static func doubleValue(_ value: NSObject?) -> Double? {
        (value as? NSNumber)?.doubleValue
    }

    nonisolated private static func boolValue(_ value: NSObject?) -> Bool {
        (value as? NSNumber)?.boolValue ?? false
    }
}

extension LibWebRTCPeerRuntime: RTCPeerConnectionDelegate {
    nonisolated func peerConnection(_: RTCPeerConnection, didChange _: RTCSignalingState) {}
    nonisolated func peerConnection(_: RTCPeerConnection, didAdd _: RTCMediaStream) {}
    nonisolated func peerConnection(_: RTCPeerConnection, didRemove _: RTCMediaStream) {}
    nonisolated func peerConnectionShouldNegotiate(_: RTCPeerConnection) {}

    nonisolated func peerConnection(
        _: RTCPeerConnection,
        didChange newState: RTCIceConnectionState
    ) {
        let state: StreamingPeerConnectionState = switch newState {
        case .new: .new
        case .checking: .checking
        case .connected, .completed: .connected
        case .disconnected: .disconnected
        case .failed: .failed
        case .closed: .closed
        case .count: .failed
        @unknown default: .failed
        }
        DispatchQueue.main.async { [weak self] in
            self?.eventSink(.connectionStateChanged(state))
        }
    }

    nonisolated func peerConnection(
        _: RTCPeerConnection,
        didChange newState: RTCIceGatheringState
    ) {
        guard newState == .complete else { return }
        DispatchQueue.main.async { [weak self] in
            self?.eventSink(.localIceGatheringComplete)
        }
    }

    nonisolated func peerConnection(
        _: RTCPeerConnection,
        didGenerate candidate: RTCIceCandidate
    ) {
        let projected = StreamingIceCandidate(
            sdp: candidate.sdp,
            sdpMid: candidate.sdpMid,
            sdpMLineIndex: candidate.sdpMLineIndex
        )
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            let plan = self.activePlan ?? .desktopWebRtcDirect
            guard StreamingIceCandidatePolicy.allows(projected, plan: plan) else { return }
            self.eventSink(.localCandidate(projected))
        }
    }

    nonisolated func peerConnection(_: RTCPeerConnection, didRemove _: [RTCIceCandidate]) {}
    nonisolated func peerConnection(_: RTCPeerConnection, didOpen _: RTCDataChannel) {}

    nonisolated func peerConnection(
        _: RTCPeerConnection,
        didStartReceivingOn transceiver: RTCRtpTransceiver
    ) {
        Task { @MainActor [weak self] in
            guard let self else { return }
            if let videoTrack = transceiver.receiver.track as? RTCVideoTrack {
                self.attachVideoTrack(videoTrack)
            } else if let audioTrack = transceiver.receiver.track as? RTCAudioTrack {
                audioTrack.isEnabled = true
                self.eventSink(.audioTrackReady)
            }
        }
    }
}

@MainActor
private final class FirstVideoFrameProbe: NSObject, RTCVideoRenderer {
    private let onFirstFrame: @MainActor () -> Void
    private var receivedFirstFrame = false

    init(onFirstFrame: @escaping @MainActor () -> Void) {
        self.onFirstFrame = onFirstFrame
    }

    nonisolated func setSize(_: CGSize) {}

    nonisolated func renderFrame(_ frame: RTCVideoFrame?) {
        guard frame != nil else { return }
        Task { @MainActor [weak self] in
            guard let self, !self.receivedFirstFrame else { return }
            self.receivedFirstFrame = true
            self.onFirstFrame()
        }
    }
}
#else
@MainActor
final class LibWebRTCPeerRuntimeFactory: StreamingPeerRuntimeFactory {
    func makeRuntime(
        eventSink _: @escaping @MainActor @Sendable (StreamingPeerEvent) -> Void
    ) -> any StreamingPeerRuntime {
        UnavailableStreamingPeerRuntime()
    }
}

@MainActor
private final class UnavailableStreamingPeerRuntime: StreamingPeerRuntime {
    func makeOffer(
        configuration _: StreamingPreparedSignaling,
        iceRestart _: Bool
    ) async throws -> String {
        throw StreamingRuntimeError.webRTCUnavailable
    }
    func applyAnswer(_: String) async throws {}
    func addRemoteCandidates(_: [StreamingIceCandidate]) async throws {}
    func stopInputAndHaptics() async {}
    func closeTransport() async {}
}
#endif
