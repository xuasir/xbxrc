import Foundation
import CryptoKit
enum IOSRuntimeTraceValue: Codable, Equatable, Sendable {
    case string(String)
    case integer(Int64)
    case double(Double)
    case bool(Bool)
    case array([IOSRuntimeTraceValue])
    case object([String: IOSRuntimeTraceValue])
    case null

    init(from decoder: any Decoder) throws {
        let container = try decoder.singleValueContainer()
        if container.decodeNil() {
            self = .null
        } else if let value = try? container.decode(Bool.self) {
            self = .bool(value)
        } else if let value = try? container.decode(Int64.self) {
            self = .integer(value)
        } else if let value = try? container.decode(Double.self) {
            self = .double(value)
        } else if let value = try? container.decode(String.self) {
            self = .string(value)
        } else if let value = try? container.decode([IOSRuntimeTraceValue].self) {
            self = .array(value)
        } else {
            self = .object(try container.decode([String: IOSRuntimeTraceValue].self))
        }
    }

    func encode(to encoder: any Encoder) throws {
        var container = encoder.singleValueContainer()
        switch self {
        case .string(let value): try container.encode(value)
        case .integer(let value): try container.encode(value)
        case .double(let value): try container.encode(value)
        case .bool(let value): try container.encode(value)
        case .array(let value): try container.encode(value)
        case .object(let value): try container.encode(value)
        case .null: try container.encodeNil()
        }
    }
}
enum StreamingRuntimeDiagnostics {
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
        let bundleMIDCount =
            lines.first(where: { $0.hasPrefix("a=group:BUNDLE ") })
            .map { max(0, $0.split(separator: " ").count - 1) } ?? 0
        let videoPayloads = mediaPayloads(in: videoSections.first)
        let audioPayloads = mediaPayloads(in: audioSections.first)
        let videoCodecs = mediaCodecs(kind: "video", in: lines)
        let audioCodecs = mediaCodecs(kind: "audio", in: lines)
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
private static func mediaCodecs(kind: String, in lines: [String]) -> [String] {
        guard let start = lines.firstIndex(where: { $0.hasPrefix("m=\(kind) ") }) else {
            return []
        }
        let section = lines.dropFirst(start + 1).prefix { !$0.hasPrefix("m=") }
        // RTP payload 只在当前 media section 中映射；重复诊断行采用最后一个值。
        let codecMap = section.reduce(into: [String: String]()) { codecs, line in
            if let (payload, codec) = rtpMapEntry(line) { codecs[payload] = codec }
        }
        return mediaPayloads(in: lines[start]).compactMap { codecMap[$0] }
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
}

let fixtures: [(String, [String], [String])] = [
    ("m=audio 9 RTP/SAVPF 96\na=rtpmap:96 opus/48000/2\nm=video 9 RTP/SAVPF 96 97\na=rtpmap:96 H264/90000\na=rtpmap:97 rtx/90000\n", ["opus"], ["H264", "rtx"]),
    ("m=audio 9 RTP/SAVPF 111\na=rtpmap:111 opus/48000/2\na=rtpmap:111 opus/48000/2\n", ["opus"], []),
    ("", [], []),
    ("m=video 9 RTP/SAVPF 96\na=rtpmap:96 H264/90000\nm=video 9 RTP/SAVPF 96\na=rtpmap:96 VP9/90000\n", [], ["H264"]),
    ("m=audio 9 RTP/SAVPF 96\nm=video 9 RTP/SAVPF 96\na=rtpmap:96 H264/90000\n", [], ["H264"])
]
for (sdp,audio,video) in fixtures {
    guard case .object(let summary)=StreamingRuntimeDiagnostics.sdpSummaryPayload(sdp) else { fatalError("missing summary") }
    precondition(summary["audioCodecs"] == .array(audio.map(IOSRuntimeTraceValue.string)))
    precondition(summary["videoCodecs"] == .array(video.map(IOSRuntimeTraceValue.string)))
}
print("PASS: 5 production SDP diagnostic regression cases")
