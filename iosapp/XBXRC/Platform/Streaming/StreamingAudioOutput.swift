import AVFoundation
import Foundation
@preconcurrency import WebRTC

struct StreamingAudioOutputPreferences: Equatable, Sendable {
    let negotiatedChannels: Int
    let preferredChannels: Int
    let sampleRate: Double
    let ioBufferDuration: Double
    let route: String
    private static let nativeDefaults: (channels: Int, sampleRate: Double, buffer: Double) = {
        let configuration = RTCAudioSessionConfiguration()
        return (
            configuration.outputNumberOfChannels, configuration.sampleRate,
            configuration.ioBufferDuration
        )
    }()

    init(
        stereoAudio: Bool, negotiatedChannels: Int? = nil,
        maximumChannels _: Int? = nil, outputPortTypes: [String] = []
    ) {
        let ports = Set(outputPortTypes)
        let hfp = ports.contains(AVAudioSession.Port.bluetoothHFP.rawValue)
        let bluetooth =
            hfp || ports.contains(AVAudioSession.Port.bluetoothA2DP.rawValue)
            || ports.contains(AVAudioSession.Port.bluetoothLE.rawValue)
        self.negotiatedChannels = stereoAudio ? min(2, max(1, negotiatedChannels ?? 2)) : 1
        // 远端 Opus 能力单独保留；本地格式与默认 ADM 的 PCM 声道/周期一致。
        preferredChannels = Self.nativeDefaults.channels
        sampleRate = Self.nativeDefaults.sampleRate
        ioBufferDuration = Self.nativeDefaults.buffer
        route =
            hfp
            ? "bluetoothHFP"
            : bluetooth
                ? "bluetooth"
                : ports.contains(AVAudioSession.Port.builtInSpeaker.rawValue)
                    ? "speaker"
                    : ports.contains(AVAudioSession.Port.headphones.rawValue)
                        || ports.contains(AVAudioSession.Port.usbAudio.rawValue)
                        ? "headphones"
                        : ports.contains(AVAudioSession.Port.airPlay.rawValue)
                            ? "airPlay" : "system"
    }

    /// stereo 描述对端的接收偏好，sprop-stereo 描述对端的发送声道。
    static func negotiatedChannels(in sdp: String, stereoRequested: Bool) -> Int? {
        var audioLines: [String] = []
        var audioPayloads = Set<String>()
        var audio = false
        for rawLine in sdp.split(separator: "\n") {
            let line = rawLine.trimmingCharacters(in: .whitespacesAndNewlines)
            if line.hasPrefix("m=") {
                let parts = line.split(separator: " ").map(String.init)
                audio = parts.first == "m=audio" && parts.count > 3 && parts[1] != "0"
                if audio { audioPayloads.formUnion(parts.dropFirst(3)) }
            } else if audio {
                audioLines.append(line)
            }
        }
        for line in audioLines where line.hasPrefix("a=rtpmap:") {
            let parts = line.dropFirst("a=rtpmap:".count).split(separator: " ", maxSplits: 1)
            guard parts.count == 2, audioPayloads.contains(String(parts[0])) else { continue }
            let codec = parts[1].split(separator: "/")
            guard codec.first?.lowercased() == "opus" else { continue }
            guard stereoRequested, codec.count >= 3, (Int(codec[2]) ?? 1) >= 2 else { return 1 }
            let prefix = "a=fmtp:\(parts[0]) "
            let parameters = audioLines.first { $0.hasPrefix(prefix) }?.dropFirst(prefix.count)
            let monoSource =
                parameters?.split(separator: ";").contains {
                    $0.trimmingCharacters(in: .whitespaces).lowercased() == "sprop-stereo=0"
                } == true
            return monoSource ? 1 : 2
        }
        return nil
    }
}

struct StreamingAudioOutputSnapshot: Equatable, Sendable {
    let negotiatedChannels: Int
    let preferredChannels: Int
    let actualChannels: Int
    let sampleRate: Double
    let ioBufferMilliseconds: Double
    let outputLatencyMilliseconds: Double
    let route: String

    init?(payload: [String: IOSRuntimeTraceValue]) {
        guard case .integer(let negotiated) = payload["negotiatedOutputChannels"],
            case .integer(let preferred) = payload["preferredOutputChannels"],
            case .integer(let actual) = payload["outputNumberOfChannels"],
            case .double(let sampleRate) = payload["sampleRate"],
            case .double(let buffer) = payload["ioBufferDurationMs"],
            case .double(let latency) = payload["outputLatencyMs"],
            case .string(let route) = payload["outputRoute"]
        else { return nil }
        negotiatedChannels = Int(clamping: negotiated)
        preferredChannels = Int(clamping: preferred)
        actualChannels = Int(clamping: actual)
        self.sampleRate = sampleRate
        ioBufferMilliseconds = buffer
        outputLatencyMilliseconds = latency
        self.route = route
    }
}
