import AVFoundation
import CryptoKit
import Foundation

struct StreamingMediaSampleProjection: Equatable, Sendable {
    let mediaAdvanced: Bool
    let firstMediaAtMilliseconds: Double?
    let lastMediaAtMilliseconds: Double?
    let frameSupplyDelta: Int64?
}

struct StreamingMediaSampleTracker: Sendable {
    private var firstMediaAtMilliseconds: Double?
    private var lastMediaAtMilliseconds: Double?
    private var lastVideoBytes: UInt64?
    private var lastVideoPacketsReceived: UInt64?
    private var lastFramesDecoded: UInt64?

    mutating func observe(
        inboundVideoBytes: UInt64,
        packetsReceived: UInt64,
        framesDecoded: UInt64,
        observedAtMilliseconds: Double
    ) -> StreamingMediaSampleProjection {
        let hasPreviousSample = lastVideoBytes != nil
            || lastVideoPacketsReceived != nil
            || lastFramesDecoded != nil
        let byteDelta = Self.positiveDelta(current: inboundVideoBytes, previous: lastVideoBytes)
        let packetDelta = Self.positiveDelta(
            current: packetsReceived,
            previous: lastVideoPacketsReceived
        )
        let frameSupplyDelta = Self.signedDelta(
            current: framesDecoded,
            previous: lastFramesDecoded
        )
        let mediaAdvanced = if hasPreviousSample {
            byteDelta > 0 || packetDelta > 0 || (frameSupplyDelta ?? 0) > 0
        } else {
            inboundVideoBytes > 0 || packetsReceived > 0 || framesDecoded > 0
        }

        if mediaAdvanced {
            if firstMediaAtMilliseconds == nil {
                firstMediaAtMilliseconds = observedAtMilliseconds
            }
            lastMediaAtMilliseconds = observedAtMilliseconds
        }
        lastVideoBytes = inboundVideoBytes
        lastVideoPacketsReceived = packetsReceived
        lastFramesDecoded = framesDecoded

        return StreamingMediaSampleProjection(
            mediaAdvanced: mediaAdvanced,
            firstMediaAtMilliseconds: firstMediaAtMilliseconds,
            lastMediaAtMilliseconds: lastMediaAtMilliseconds,
            frameSupplyDelta: frameSupplyDelta
        )
    }

    private static func positiveDelta(current: UInt64, previous: UInt64?) -> UInt64 {
        guard let previous, current >= previous else { return 0 }
        return current - previous
    }

    private static func signedDelta(current: UInt64, previous: UInt64?) -> Int64? {
        guard let previous, current >= previous else { return nil }
        return Int64(clamping: current - previous)
    }
}

private struct CreatedPeerDataChannelsResult: Sendable {
    let createdChannels: [XboxStreamCreatedDataChannel]
    let failedLabels: [String]
}

private enum StreamingRuntimeDiagnostics {
    static func sdpSummaryPayload(_ sdp: String) -> IOSRuntimeTraceValue {
        let normalized = sdp.replacingOccurrences(of: "\r\n", with: "\n")
        let lines = normalized.split(separator: "\n").map(String.init)
        let mediaSections = lines.filter { $0.hasPrefix("m=") }
        let audioSections = mediaSections.filter { $0.hasPrefix("m=audio ") }
        let videoSections = mediaSections.filter { $0.hasPrefix("m=video ") }
        let applicationSections = mediaSections.filter { $0.hasPrefix("m=application ") }
        let extmapCount = lines.filter { $0.hasPrefix("a=extmap:") }.count
        let feedbackCount = lines.filter { $0.hasPrefix("a=rtcp-fb:") }.count
        let candidateCount = lines.filter {
            $0.hasPrefix("a=candidate:") || $0.hasPrefix("candidate:")
        }.count
        let midCount = lines.filter { $0.hasPrefix("a=mid:") }.count
        let bundleMIDCount = lines.first(where: { $0.hasPrefix("a=group:BUNDLE ") })
            .map { max(0, $0.split(separator: " ").count - 1) } ?? 0
        let codecMap = Dictionary(uniqueKeysWithValues: lines.compactMap(rtpMapEntry))
        let videoPayloads = mediaPayloads(in: videoSections.first)
        let audioPayloads = mediaPayloads(in: audioSections.first)
        let videoCodecs = videoPayloads.compactMap { codecMap[$0] }
        let audioCodecs = audioPayloads.compactMap { codecMap[$0] }
        let h264Profiles = Array(Set(lines.compactMap(profileLevelID))).sorted()

        return .object([
            "fingerprint": .string(fingerprint(sdp)),
            "lineCount": .integer(Int64(lines.count)),
            "mediaSectionCount": .integer(Int64(mediaSections.count)),
            "audioSectionCount": .integer(Int64(audioSections.count)),
            "videoSectionCount": .integer(Int64(videoSections.count)),
            "applicationSectionCount": .integer(Int64(applicationSections.count)),
            "midCount": .integer(Int64(midCount)),
            "bundleMidCount": .integer(Int64(bundleMIDCount)),
            "extmapCount": .integer(Int64(extmapCount)),
            "rtcpFeedbackCount": .integer(Int64(feedbackCount)),
            "candidateLineCount": .integer(Int64(candidateCount)),
            "audioPayloadCount": .integer(Int64(audioPayloads.count)),
            "videoPayloadCount": .integer(Int64(videoPayloads.count)),
            "audioCodecs": .array(audioCodecs.map(IOSRuntimeTraceValue.string)),
            "videoCodecs": .array(videoCodecs.map(IOSRuntimeTraceValue.string)),
            "h264Profiles": .array(h264Profiles.map(IOSRuntimeTraceValue.string)),
        ])
    }

    static func threadContextPayload() -> IOSRuntimeTraceValue {
        let threadName = Thread.current.name?.isEmpty == false
            ? Thread.current.name!
            : "unnamed"
        return .object([
            "isMainThread": .bool(Thread.isMainThread),
            "threadName": .string(threadName),
            "qualityOfService": .string(qosName(Thread.current.qualityOfService)),
        ])
    }

    static func runtimeContextPayload(plan: StreamingWebRtcPlan) -> [String: IOSRuntimeTraceValue] {
        [
            "deviceModel": .string(deviceModelIdentifier()),
            "osVersion": .string(ProcessInfo.processInfo.operatingSystemVersionString),
            "audioDirection": .string(plan.audioDirection.rawValue),
            "videoDirection": .string(plan.videoDirection.rawValue),
            "iceTransportPolicy": .string(plan.iceTransportPolicy),
            "preferIPv6": .bool(plan.preferIPv6),
            "stereoAudio": .bool(plan.stereoAudio),
            "targetVideoWidth": .integer(Int64(plan.targetVideoWidth)),
            "targetVideoHeight": .integer(Int64(plan.targetVideoHeight)),
            "audioBitrateKbps": plan.audioBitrateKbps.map { .integer(Int64($0)) } ?? .null,
            "maxVideoBitrateKbps": plan.maxVideoBitrateKbps.map { .integer(Int64($0)) } ?? .null,
            "h264Profiles": .array(plan.h264Profiles.map(IOSRuntimeTraceValue.string)),
            "requiredVideoRtcpFeedback": .array(
                plan.requiredVideoRtcpFeedback.map(IOSRuntimeTraceValue.string)
            ),
            "allowedCandidateTypes": .array(
                plan.allowedCandidateTypes.map(IOSRuntimeTraceValue.string)
            ),
        ]
    }

    private static func rtpMapEntry(_ line: String) -> (String, String)? {
        guard line.hasPrefix("a=rtpmap:") else { return nil }
        let raw = line.dropFirst("a=rtpmap:".count)
        let parts = raw.split(separator: " ", maxSplits: 1).map(String.init)
        guard parts.count == 2 else { return nil }
        let codec = parts[1].split(separator: "/").first.map(String.init) ?? parts[1]
        return (parts[0], codec)
    }

    private static func mediaPayloads(in mediaLine: String?) -> [String] {
        guard let mediaLine else { return [] }
        let fields = mediaLine.split(separator: " ").map(String.init)
        guard fields.count > 3 else { return [] }
        return Array(fields.dropFirst(3))
    }

    private static func profileLevelID(_ line: String) -> String? {
        guard line.hasPrefix("a=fmtp:") else { return nil }
        let lowered = line.lowercased()
        guard let range = lowered.range(of: "profile-level-id=") else { return nil }
        let suffix = lowered[range.upperBound...]
        let profile = suffix.split(separator: ";", maxSplits: 1).first.map(String.init) ?? ""
        return profile.isEmpty ? nil : profile
    }

    private static func fingerprint(_ value: String) -> String {
        let digest = SHA256.hash(data: Data(value.utf8))
        return digest.prefix(8).map { String(format: "%02x", $0) }.joined()
    }

    private static func deviceModelIdentifier() -> String {
        var systemInfo = utsname()
        uname(&systemInfo)
        let machine = systemInfo.machine
        return withUnsafePointer(to: machine) {
            $0.withMemoryRebound(
                to: CChar.self,
                capacity: MemoryLayout.size(ofValue: machine)
            ) {
                String(cString: $0)
            }
        }
    }

    private static func qosName(_ qos: QualityOfService) -> String {
        switch qos {
        case .userInteractive: "userInteractive"
        case .userInitiated: "userInitiated"
        case .utility: "utility"
        case .background: "background"
        case .default: "default"
        @unknown default: "unknown"
        }
    }
}

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
        patchMediaBitrate(
            lines: &lines,
            media: "audio",
            bitrateKbps: plan.audioBitrateKbps
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

final class LibWebRTCPeerRuntimeFactory: StreamingPeerRuntimeFactory, @unchecked Sendable {
    func makeRuntime(
        eventSink: @escaping @Sendable (StreamingPeerEvent) -> Void
    ) -> any StreamingPeerRuntime {
        LibWebRTCPeerRuntime(eventSink: eventSink)
    }
}

private final class LibWebRTCPeerRuntime: NSObject, StreamingPeerRuntime, @unchecked Sendable {
    private struct StatsProjectionState {
        var mediaSamples = StreamingMediaSampleTracker()
        var lastVideoBytes: UInt64?
        var lastObservedAtMilliseconds: Double?
    }

    private static let factory: RTCPeerConnectionFactory = {
        RTCInitializeSSL()
        return RTCPeerConnectionFactory(
            encoderFactory: RTCDefaultVideoEncoderFactory(),
            decoderFactory: RTCDefaultVideoDecoderFactory()
        )
    }()

    private let apiQueue = DispatchQueue(
        label: "com.xbxrc.ios.webrtc.peer-runtime",
        qos: .userInitiated
    )
    private let eventSink: @Sendable (StreamingPeerEvent) -> Void
    private var peerConnection: RTCPeerConnection?
    private var frameProbe: FirstVideoFrameProbe?
    private var dataChannels: XboxStreamDataChannels?
    private var statsTask: Task<Void, Never>?
    private var statsProjectionState = StatsProjectionState()
    private var activePlan: StreamingWebRtcPlan?

    init(eventSink: @escaping @Sendable (StreamingPeerEvent) -> Void) {
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

        try await runOnAPIQueue { [self] in
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
        }
        guard let peer = peerConnection else {
            throw StreamingRuntimeError.peerConnectionCreationFailed
        }

        let channels = await MainActor.run {
            XboxStreamDataChannels(
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
        }
        let createdDataChannels = try await runOnAPIQueue {
            var createdChannels: [XboxStreamCreatedDataChannel] = []
            var failedLabels: [String] = []
            for profile in streamDataChannelProfiles() {
                let configuration = RTCDataChannelConfiguration()
                configuration.isOrdered = profile.ordered
                configuration.`protocol` = profile.protocolName
                if let channel = peer.dataChannel(
                    forLabel: profile.label,
                    configuration: configuration
                ) {
                    createdChannels.append(
                        XboxStreamCreatedDataChannel(label: profile.label, channel: channel)
                    )
                } else {
                    failedLabels.append(profile.label)
                }
            }
            return CreatedPeerDataChannelsResult(
                createdChannels: createdChannels,
                failedLabels: failedLabels
            )
        }
        await MainActor.run {
            channels.attachCreatedChannels(
                createdDataChannels.createdChannels,
                failedLabels: createdDataChannels.failedLabels
            )
        }
        dataChannels = channels
        eventSink(.dataChannelEvent(label: "all", event: "profilesCreated"))
        let dataChannelSnapshot = await MainActor.run { channels.debugSnapshot() }
        let runtimeConfigurationPayload = Self.runtimeConfigurationPayload(
            plan: prepared.webRtcPlan,
            peer: peer,
            dataChannelSnapshot: dataChannelSnapshot
        )
        IOSRuntimeTrace.snapshot(
            domain: "ios-streaming",
            event: "peerRuntimeConfigured",
            payload: runtimeConfigurationPayload,
            dimension: .network,
            importance: .key
        )

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
            apiQueue.async {
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
        }
        let projectedOffer = RTCSessionDescription(
            type: offer.type,
            sdp: StreamingOfferSdpProjector().project(offer.sdp, plan: plan)
        )
        IOSRuntimeTrace.state(
            domain: "ios-streaming",
            event: "localOfferPrepared",
            payload: [
                "iceRestart": .bool(iceRestart),
                "offerSummary": StreamingRuntimeDiagnostics.sdpSummaryPayload(projectedOffer.sdp),
                "threadContext": StreamingRuntimeDiagnostics.threadContextPayload(),
            ],
            dimension: .network,
            importance: .key
        )
        try await setLocalDescription(projectedOffer, on: peer)
        return projectedOffer.sdp
    }

    func applyAnswer(_ sdp: String) async throws {
        guard let peerConnection else {
            throw StreamingRuntimeError.peerConnectionCreationFailed
        }
        let startedAt = Date()
        let dataChannelSnapshot = await MainActor.run { dataChannels?.debugSnapshot() }
        let peerSnapshot = Self.peerSnapshotPayload(
            peerConnection: peerConnection,
            dataChannelSnapshot: dataChannelSnapshot
        )
        IOSRuntimeTrace.state(
            domain: "ios-streaming",
            event: "remoteDescriptionApplyStarted",
            payload: [
                "answerBytes": .integer(Int64(sdp.utf8.count)),
                "answerSummary": StreamingRuntimeDiagnostics.sdpSummaryPayload(sdp),
                "peerSnapshot": peerSnapshot,
                "threadContext": StreamingRuntimeDiagnostics.threadContextPayload(),
            ],
            dimension: .network,
            importance: .key
        )
        let answer = RTCSessionDescription(type: .answer, sdp: sdp)
        try await withCheckedThrowingContinuation {
            (continuation: CheckedContinuation<Void, Error>) in
            IOSRuntimeTrace.state(
                domain: "ios-streaming",
                event: "remoteDescriptionSetWillDispatch",
                payload: [
                    "threadContext": StreamingRuntimeDiagnostics.threadContextPayload(),
                ],
                dimension: .network,
                importance: .key
            )
            apiQueue.async {
                IOSRuntimeTrace.state(
                    domain: "ios-streaming",
                    event: "remoteDescriptionSetDispatched",
                    payload: [
                        "elapsedMs": .integer(Int64(Date().timeIntervalSince(startedAt) * 1_000)),
                        "threadContext": StreamingRuntimeDiagnostics.threadContextPayload(),
                    ],
                    dimension: .network,
                    importance: .key
                )
                IOSRuntimeTrace.state(
                    domain: "ios-streaming",
                    event: "remoteDescriptionSetInvoking",
                    payload: [
                        "threadContext": StreamingRuntimeDiagnostics.threadContextPayload(),
                    ],
                    dimension: .network,
                    importance: .key
                )
                peerConnection.setRemoteDescription(answer) { error in
                    let nsError = error as NSError?
                    IOSRuntimeTrace.state(
                        domain: "ios-streaming",
                        event: "remoteDescriptionApplyCallback",
                        payload: [
                            "elapsedMs": .integer(
                                Int64(Date().timeIntervalSince(startedAt) * 1_000)
                            ),
                            "hasError": .bool(error != nil),
                            "errorDomain": nsError.map { .string($0.domain) } ?? .null,
                            "errorCode": nsError.map { .integer(Int64($0.code)) } ?? .null,
                            "threadContext": StreamingRuntimeDiagnostics.threadContextPayload(),
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
            }
        }
        IOSRuntimeTrace.state(
            domain: "ios-streaming",
            event: "remoteDescriptionApplyCompleted",
            payload: [
                "elapsedMs": .integer(Int64(Date().timeIntervalSince(startedAt) * 1_000)),
                "threadContext": StreamingRuntimeDiagnostics.threadContextPayload(),
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
                    apiQueue.async {
                        peerConnection.add(rtcCandidate) { error in
                            if let error {
                                continuation.resume(throwing: error)
                            } else {
                                continuation.resume(returning: ())
                            }
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
        if let dataChannels {
            await MainActor.run {
                dataChannels.stopInputAndHaptics()
            }
        }
        dataChannels = nil
    }

    func debugSnapshot() async -> StreamingPeerDebugSnapshot {
        let transceivers = peerConnection?.transceivers ?? []
        let dataChannelSnapshot = await MainActor.run {
            dataChannels?.debugSnapshot()
        }
        return StreamingPeerDebugSnapshot(
            signalingState: peerConnection.map(Self.signalingStateName) ?? "missing",
            iceConnectionState: peerConnection.map(Self.iceConnectionStateName) ?? "missing",
            iceGatheringState: peerConnection.map(Self.iceGatheringStateName) ?? "missing",
            transceiverCount: transceivers.count,
            audioReceiverTrackCount: transceivers.filter {
                $0.receiver.track is RTCAudioTrack
            }.count,
            videoReceiverTrackCount: transceivers.filter {
                $0.receiver.track is RTCVideoTrack
            }.count,
            localDescriptionSet: peerConnection?.localDescription != nil,
            remoteDescriptionSet: peerConnection?.remoteDescription != nil,
            dataChannels: dataChannelSnapshot
        )
    }

    func closeTransport() async {
        if let dataChannels {
            await dataChannels.closeAfterInputDrain()
        }
        dataChannels = nil
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
            apiQueue.async {
                peer.setLocalDescription(description) { error in
                    if let error {
                        continuation.resume(throwing: error)
                    } else {
                        continuation.resume(returning: ())
                    }
                }
            }
        }
    }

    private func runOnAPIQueue<T: Sendable>(
        _ operation: @escaping @Sendable () throws -> T
    ) async throws -> T {
        try await withCheckedThrowingContinuation { continuation in
            apiQueue.async {
                do {
                    let result = try operation()
                    continuation.resume(returning: result)
                } catch {
                    continuation.resume(throwing: error)
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
        let route = AVAudioSession.sharedInstance().currentRoute
        IOSRuntimeTrace.state(
            domain: "ios-streaming",
            event: "audioSessionActivated",
            payload: [
                "mode": .string(configuration.mode),
                "category": .string(configuration.category),
                "outputPortTypes": .array(route.outputs.map {
                    .string($0.portType.rawValue)
                }),
                "inputPortTypes": .array(route.inputs.map {
                    .string($0.portType.rawValue)
                }),
                "sampleRate": .double(session.sampleRate),
                "ioBufferDurationMs": .double(session.ioBufferDuration * 1_000),
            ],
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
            guard let self else { return }
            self.eventSink(.stats(self.projectStats(report)))
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

        let mediaSample = statsProjectionState.mediaSamples.observe(
            inboundVideoBytes: inboundVideoBytes,
            packetsReceived: packetsReceived,
            framesDecoded: framesDecoded,
            observedAtMilliseconds: observedAtMilliseconds
        )
        let receiveBitrateBps = Self.ratePerSecond(
            current: inboundVideoBytes,
            previous: statsProjectionState.lastVideoBytes,
            elapsedMilliseconds: statsProjectionState.lastObservedAtMilliseconds.map {
                observedAtMilliseconds - $0
            },
            multiplier: 8
        )
        statsProjectionState.lastVideoBytes = inboundVideoBytes
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
            firstMediaAtMilliseconds: mediaSample.firstMediaAtMilliseconds,
            lastMediaAtMilliseconds: mediaSample.lastMediaAtMilliseconds,
            frameSupplyDelta: mediaSample.frameSupplyDelta
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

    private static func runtimeConfigurationPayload(
        plan: StreamingWebRtcPlan,
        peer: RTCPeerConnection,
        dataChannelSnapshot: StreamingDataChannelDebugSnapshot?
    ) -> [String: IOSRuntimeTraceValue] {
        var payload = StreamingRuntimeDiagnostics.runtimeContextPayload(plan: plan)
        payload["peerSnapshot"] = peerSnapshotPayload(
            peerConnection: peer,
            dataChannelSnapshot: dataChannelSnapshot
        )
        return payload
    }

    private static func peerSnapshotPayload(
        peerConnection: RTCPeerConnection,
        dataChannelSnapshot: StreamingDataChannelDebugSnapshot?
    ) -> IOSRuntimeTraceValue {
        let transceivers = peerConnection.transceivers
        return .object([
            "signalingState": .string(signalingStateName(peerConnection)),
            "iceConnectionState": .string(iceConnectionStateName(peerConnection)),
            "iceGatheringState": .string(iceGatheringStateName(peerConnection)),
            "transceiverCount": .integer(Int64(transceivers.count)),
            "audioReceiverTrackCount": .integer(Int64(transceivers.filter {
                $0.receiver.track is RTCAudioTrack
            }.count)),
            "videoReceiverTrackCount": .integer(Int64(transceivers.filter {
                $0.receiver.track is RTCVideoTrack
            }.count)),
            "localDescriptionSet": .bool(peerConnection.localDescription != nil),
            "remoteDescriptionSet": .bool(peerConnection.remoteDescription != nil),
            "dataChannels": dataChannelSnapshot.map(dataChannelSnapshotPayload) ?? .null,
        ])
    }

    private static func dataChannelSnapshotPayload(
        _ snapshot: StreamingDataChannelDebugSnapshot
    ) -> IOSRuntimeTraceValue {
        .object([
            "readyStates": .object(snapshot.readyStates.mapValues(IOSRuntimeTraceValue.string)),
            "phases": .object(snapshot.phases.mapValues(IOSRuntimeTraceValue.string)),
            "handshakeAcknowledged": .bool(snapshot.handshakeAcknowledged),
            "controlReady": .bool(snapshot.controlReady),
            "inputStarted": .bool(snapshot.inputStarted),
            "terminalReason": snapshot.terminalReason.map(IOSRuntimeTraceValue.string) ?? .null,
        ])
    }

    private static func signalingStateName(_ peerConnection: RTCPeerConnection) -> String {
        switch peerConnection.signalingState {
        case .stable: "stable"
        case .haveLocalOffer: "haveLocalOffer"
        case .haveLocalPrAnswer: "haveLocalPrAnswer"
        case .haveRemoteOffer: "haveRemoteOffer"
        case .haveRemotePrAnswer: "haveRemotePrAnswer"
        case .closed: "closed"
        @unknown default: "unknown"
        }
    }

    private static func iceConnectionStateName(_ peerConnection: RTCPeerConnection) -> String {
        switch peerConnection.iceConnectionState {
        case .new: "new"
        case .checking: "checking"
        case .connected: "connected"
        case .completed: "completed"
        case .failed: "failed"
        case .disconnected: "disconnected"
        case .closed: "closed"
        case .count: "count"
        @unknown default: "unknown"
        }
    }

    private static func iceGatheringStateName(_ peerConnection: RTCPeerConnection) -> String {
        switch peerConnection.iceGatheringState {
        case .new: "new"
        case .gathering: "gathering"
        case .complete: "complete"
        @unknown default: "unknown"
        }
    }
}

extension LibWebRTCPeerRuntime: RTCPeerConnectionDelegate {
    func peerConnection(_: RTCPeerConnection, didChange _: RTCSignalingState) {}
    func peerConnection(_: RTCPeerConnection, didAdd _: RTCMediaStream) {}
    func peerConnection(_: RTCPeerConnection, didRemove _: RTCMediaStream) {}
    func peerConnectionShouldNegotiate(_: RTCPeerConnection) {}

    func peerConnection(
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
        eventSink(.connectionStateChanged(state))
    }

    func peerConnection(
        _: RTCPeerConnection,
        didChange newState: RTCIceGatheringState
    ) {
        guard newState == .complete else { return }
        eventSink(.localIceGatheringComplete)
    }

    func peerConnection(
        _: RTCPeerConnection,
        didGenerate candidate: RTCIceCandidate
    ) {
        let projected = StreamingIceCandidate(
            sdp: candidate.sdp,
            sdpMid: candidate.sdpMid,
            sdpMLineIndex: candidate.sdpMLineIndex
        )
        let plan = activePlan ?? .desktopWebRtcDirect
        guard StreamingIceCandidatePolicy.allows(projected, plan: plan) else { return }
        eventSink(.localCandidate(projected))
    }

    func peerConnection(_: RTCPeerConnection, didRemove _: [RTCIceCandidate]) {}
    func peerConnection(_: RTCPeerConnection, didOpen _: RTCDataChannel) {}

    func peerConnection(
        _: RTCPeerConnection,
        didStartReceivingOn transceiver: RTCRtpTransceiver
    ) {
        if let videoTrack = transceiver.receiver.track as? RTCVideoTrack {
            attachVideoTrack(videoTrack)
        } else if let audioTrack = transceiver.receiver.track as? RTCAudioTrack {
            audioTrack.isEnabled = true
            eventSink(.audioTrackReady)
        }
    }
}

private final class FirstVideoFrameProbe: NSObject, RTCVideoRenderer {
    private let onFirstFrame: @Sendable () -> Void
    private var receivedFirstFrame = false

    init(onFirstFrame: @escaping @Sendable () -> Void) {
        self.onFirstFrame = onFirstFrame
    }

    func setSize(_: CGSize) {}

    func renderFrame(_ frame: RTCVideoFrame?) {
        guard frame != nil else { return }
        guard !receivedFirstFrame else { return }
        receivedFirstFrame = true
        onFirstFrame()
    }
}
#else
final class LibWebRTCPeerRuntimeFactory: StreamingPeerRuntimeFactory, @unchecked Sendable {
    func makeRuntime(
        eventSink _: @escaping @Sendable (StreamingPeerEvent) -> Void
    ) -> any StreamingPeerRuntime {
        UnavailableStreamingPeerRuntime()
    }
}

private final class UnavailableStreamingPeerRuntime: StreamingPeerRuntime, @unchecked Sendable {
    func makeOffer(
        configuration _: StreamingPreparedSignaling,
        iceRestart _: Bool
    ) async throws -> String {
        throw StreamingRuntimeError.webRTCUnavailable
    }
    func applyAnswer(_: String) async throws {}
    func addRemoteCandidates(_: [StreamingIceCandidate]) async throws {}
    func debugSnapshot() async -> StreamingPeerDebugSnapshot {
        StreamingPeerDebugSnapshot(
            signalingState: "unavailable",
            iceConnectionState: "unavailable",
            iceGatheringState: "unavailable",
            transceiverCount: 0,
            audioReceiverTrackCount: 0,
            videoReceiverTrackCount: 0,
            localDescriptionSet: false,
            remoteDescriptionSet: false,
            dataChannels: nil
        )
    }
    func stopInputAndHaptics() async {}
    func closeTransport() async {}
}
#endif
