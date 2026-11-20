import Foundation

struct StreamingSampleMetrics: Equatable, Sendable {
    var sampledAtMilliseconds: Double = 0
    var windowMilliseconds: Double?
    var receivedFps: Double?
    var decodedFps: Double?
    var submittedFps: Double?
    var presentedFps: Double?
    var totalBitrateBps: Double?
    var packetLossPercent: Double?
}

/// 使用同一采样窗的累计计数计算速率；首次、重连和计数回退重新建立基线。
struct StreamingMetricsSampler {
    private struct Sample {
        let at: Double
        let stats: StreamingPeerStats
        let display: StreamingDisplayHealthSnapshot?
    }
    private var previous: Sample?
    private var lossWindow: [Sample] = []
    private var latest = StreamingSampleMetrics()

    mutating func observe(_ diagnostics: StreamingSessionDiagnostics) -> StreamingSampleMetrics {
        guard let stats = diagnostics.peerStats else { return .init() }
        let at = stats.observedAtMilliseconds ?? diagnostics.updatedAtMilliseconds
        guard at.isFinite, at > 0 else { return .init() }
        if previous?.at == at { return latest }
        let current = Sample(at: at, stats: stats, display: diagnostics.displayHealth)
        defer { previous = current }
        guard let previous, at > previous.at, at - previous.at <= 3_000,
            stats.inboundVideoBytes >= previous.stats.inboundVideoBytes,
            stats.packetsReceived >= previous.stats.packetsReceived,
            countersAdvance(stats.framesReceived, previous.stats.framesReceived),
            countersAdvance(stats.observedFramesDecoded, previous.stats.observedFramesDecoded)
        else {
            lossWindow = [current]
            latest = .init(sampledAtMilliseconds: at)
            return latest
        }
        let elapsed = at - previous.at
        func rate(_ value: UInt64?, _ old: UInt64?) -> Double? {
            guard let value, let old, value >= old else { return nil }
            return Double(value - old) * 1_000 / elapsed
        }
        func displayRate(_ value: Int64?, _ old: Int64?) -> Double? {
            guard let value, let old, value >= old, old >= 0 else { return nil }
            return Double(value - old) * 1_000 / elapsed
        }
        lossWindow.append(current)
        lossWindow.removeAll { at - $0.at > 5_000 }
        if lossWindow.count > 64 { lossWindow.removeFirst(lossWindow.count - 64) }
        var loss: Double?
        if let baseline = lossWindow.first, baseline.at < at,
            let lost = stats.videoPacketsLost, let oldLost = baseline.stats.videoPacketsLost
        {
            let received = stats.packetsReceived - baseline.stats.packetsReceived
            let dropped = max(0, lost - oldLost)
            let total = Double(received) + Double(dropped)
            if total > 0 { loss = Double(dropped) * 100 / total }
        }
        let audioRate = rate(stats.inboundAudioBytes, previous.stats.inboundAudioBytes)
        let videoRate = rate(stats.inboundVideoBytes, previous.stats.inboundVideoBytes)
        latest = StreamingSampleMetrics(
            sampledAtMilliseconds: at, windowMilliseconds: elapsed,
            receivedFps: rate(stats.framesReceived, previous.stats.framesReceived),
            decodedFps: rate(stats.observedFramesDecoded, previous.stats.observedFramesDecoded),
            submittedFps: displayRate(
                diagnostics.displayHealth?.submitCount, previous.display?.submitCount),
            presentedFps: displayRate(
                diagnostics.displayHealth?.presentCount, previous.display?.presentCount),
            totalBitrateBps: audioRate.flatMap { audio in videoRate.map { (audio + $0) * 8 } },
            packetLossPercent: loss)
        return latest
    }

    private func countersAdvance(_ current: UInt64?, _ previous: UInt64?) -> Bool {
        guard let current, let previous else { return true }
        return current >= previous
    }
}

enum StreamingDiagnosticGroup: String, CaseIterable, Identifiable {
    case network, display, recovery, session
    var id: String { rawValue }
    var title: String {
        switch self {
        case .network: "网络"
        case .display: "显示"
        case .recovery: "恢复"
        case .session: "会话"
        }
    }
    var hint: String {
        switch self {
        case .network: "查看延迟、丢包、码率与控制通道。"
        case .display: "对照接收、解码、提交与呈现帧率，以及上屏长尾。"
        case .recovery: "查看媒体供给、恢复归因与恢复效果。"
        case .session: "查看连接耗时、首帧耗时与采样状态。"
        }
    }
}

struct StreamingDiagnosticRow: Identifiable {
    let id: String
    let title: String
    let value: String
}

struct StreamingDiagnosticsModel {
    let diagnostics: StreamingSessionDiagnostics
    let state: StreamingFeatureState
    let picture: MobilePicturePreferences
    var nowMilliseconds = ProcessInfo.processInfo.systemUptime * 1_000

    var observation: String {
        guard diagnostics.peerStats != nil, diagnostics.metrics.sampledAtMilliseconds > 0 else {
            return "unavailable"
        }
        guard nowMilliseconds >= diagnostics.metrics.sampledAtMilliseconds else {
            return "unavailable"
        }
        return nowMilliseconds - diagnostics.metrics.sampledAtMilliseconds > 3_000
            ? "stale" : "fresh"
    }
    var fresh: Bool { observation == "fresh" }
    var resolution: String {
        fresh ? dimensions(configuration?.sourceWidth, configuration?.sourceHeight) : "--"
    }
    var presentFps: String { fresh ? decimal(diagnostics.metrics.presentedFps, digits: 0) : "--" }
    var videoBitrate: String { fresh ? bitrate(diagnostics.peerStats?.receiveBitrateBps) : "--" }
    var rtt: String {
        fresh ? duration(diagnostics.peerStats?.roundTripTimeSeconds.map { $0 * 1_000 }) : "--"
    }
    var packetLoss: String { fresh ? percent(diagnostics.metrics.packetLossPercent) : "--" }
    var notice: String? {
        if observation == "stale" { return "数据暂缺" }
        if observation == "unavailable" { return "采集中" }
        if state == .recovering { return "画面恢复中" }
        if diagnostics.metrics.presentedFps == 0 { return "画面暂时停滞" }
        return nil
    }
    var renderLabel: String {
        guard let configuration else { return "采集中" }
        let mode = configuration.sharpness <= 0 ? "原画" : configuration.sharpness <= 2 ? "清晰" : "柔和"
        return configuration.superResolutionActive ? "\(mode) · SR" : mode
    }
    var initialGroup: StreamingDiagnosticGroup {
        if state == .recovering { return .recovery }
        return diagnostics.metrics.presentedFps == 0 ? .display : .network
    }
    private var configuration: StreamingDisplayConfiguration? {
        diagnostics.displayHealth?.configuration
    }

    func rows(_ group: StreamingDiagnosticGroup) -> [StreamingDiagnosticRow] {
        let stats = diagnostics.peerStats
        let display = diagnostics.displayHealth
        let decision = diagnostics.supervision
        func row(_ id: String, _ title: String, _ value: String) -> StreamingDiagnosticRow {
            .init(id: id, title: title, value: value)
        }
        let result: [StreamingDiagnosticRow]
        switch group {
        case .network:
            result = [
                row("rtt", "网络 RTT", rtt),
                row("rttBaseline", "路径 RTT 基线", duration(decision?.quality?.rttBaselineMs)),
                row("rttDelta", "RTT 增量", duration(decision?.quality?.rttDeltaMs)),
                row(
                    "jitter", "网络抖动",
                    fresh ? duration(stats?.jitterSeconds.map { $0 * 1_000 }) : "--"),
                row("packetLoss", "近 5 秒丢包", packetLoss),
                row("qualityObserved", "实际视频码率", videoBitrate),
                row(
                    "qualityRequested", "请求码率上限",
                    bitrate(diagnostics.requestedVideoBitrateKbps.map { Double($0) * 1_000 })),
                row(
                    "qualityNegotiated", "远端确认上限",
                    bitrate(diagnostics.negotiatedVideoBitrateKbps.map { Double($0) * 1_000 })),
                row(
                    "totalBitrate", "总下行",
                    fresh ? bitrate(diagnostics.metrics.totalBitrateBps) : "--"),
                row("qualityEffect", "网络动作效果", decision?.quality?.effect ?? "--"),
                row("bandwidthState", "带宽状态", decision?.networkPressure ?? "--"),
                row(
                    "controlChannelState", "控制通道",
                    diagnostics.peer?.dataChannels?.readyStates["control"] ?? "--"),
                row(
                    "feedback", "NACK / PLI / FIR",
                    "\(integer(stats?.nackCount)) / \(integer(stats?.pliCount)) / \(integer(stats?.firCount))"
                ),
            ]
        case .display:
            result = [
                row(
                    "receivedFps", "接收帧 / 秒",
                    fresh ? decimal(diagnostics.metrics.receivedFps) : "--"),
                row(
                    "decodedFps", "解码帧 / 秒", fresh ? decimal(diagnostics.metrics.decodedFps) : "--"),
                row(
                    "rendererSubmitFps", "渲染提交 / 秒",
                    fresh ? decimal(diagnostics.metrics.submittedFps) : "--"),
                row(
                    "videoAdvancementFps", "视频推进 / 秒",
                    fresh ? decimal(diagnostics.metrics.presentedFps) : "--"),
                row("sourceSize", "实际源尺寸", resolution),
                row(
                    "outputSize", "实际输出尺寸",
                    fresh
                        ? dimensions(configuration?.outputWidth, configuration?.outputHeight) : "--"
                ),
                row("submitAge", "最新提交距今", age(display?.lastSubmitAtMilliseconds)),
                row("renderPipelineType", "渲染管线", configuration == nil ? "--" : "Metal"),
                row("srSetting", "超分（设置）", picture.superResolution ? "开启" : "关闭"),
                row(
                    "srRuntime", "超分（运行态）",
                    configuration.map { $0.superResolutionActive ? "MetalFX" : "标准缩放" } ?? "--"),
                row("renderProcessing", "锐化后处理", renderLabel),
                row(
                    "submitPresent", "提交 / 呈现",
                    "\(integer(display?.submitCount)) / \(integer(display?.presentCount))"),
                row(
                    "submitPresentP50", "提交到呈现 P50",
                    duration(display?.submitToPresentP50Milliseconds)),
                row(
                    "submitPresentTail", "提交到呈现 P95 / P99",
                    "\(duration(display?.submitToPresentP95Milliseconds)) / \(duration(display?.submitToPresentP99Milliseconds))"
                ),
                row(
                    "frameCauses", "累计覆盖 / 呈现丢弃",
                    "\(integer(display?.mailboxOverwriteCount)) / \(integer(display.map { $0.dropCountByReason.values.reduce(0, +) }))"
                ),
                row("presentDelta", "呈现帧增量", integer(display?.displayedFrameDelta)),
            ]
        case .recovery:
            result = [
                row("presentationMilestone", "呈现阶段", state.traceCode),
                row(
                    "recoveryCause", "恢复归因",
                    decision?.probeEffect?.reason ?? decision?.quality?.reason ?? "--"),
                row("recoveryEffect", "播放恢复效果", decision?.probeEffect?.result ?? "--"),
                row("keyframeSuccessRate", "关键帧成功率", "--"),
                row(
                    "decodeRecovery", "解码恢复",
                    decision.map { $0.decodeRecoveryEligible ? "可执行" : "观察中" } ?? "--"),
                row("frameSupplyDelta", "帧供给增量", integer(stats?.frameSupplyDelta)),
                row(
                    "steadySupply", "持续帧供给",
                    fresh ? (decision.map { $0.hasInboundProgress ? "健康" : "停滞" } ?? "--") : "--"),
                row("lastMediaAge", "最近媒体距今", age(stats?.lastMediaAtMilliseconds)),
                row("freezeCount", "冻结次数", integer(stats?.freezeCount)),
                row(
                    "freezeDuration", "累计冻结时长",
                    duration(stats?.freezeDurationSeconds.map { $0 * 1_000 })),
            ]
        case .session:
            result = [
                row("connectedElapsed", "连接耗时", duration(diagnostics.connectedElapsedMilliseconds)),
                row(
                    "mediaReadyElapsed", "首帧耗时", duration(diagnostics.mediaReadyElapsedMilliseconds)
                ),
                row("transportState", "传输", diagnostics.peer?.iceConnectionState ?? "--"),
                row(
                    "transportPath", "连接路径",
                    stats.map {
                        "\($0.selectedLocalCandidateType ?? "--") → \($0.selectedRemoteCandidateType ?? "--") · \($0.selectedCandidatePairProtocol ?? "--")"
                    } ?? "--"),
                row(
                    "frameWindow", "窗口 / 样本数",
                    diagnostics.metrics.windowMilliseconds.map { "\(duration($0)) / 2" } ?? "--"),
                row("observation", "观测 / 样本", observation),
                row("sampleAge", "采样距今", age(diagnostics.metrics.sampledAtMilliseconds)),
                row("audioLifecycle", "音频状态", diagnostics.audioLifecycleState ?? "--"),
                row("audioRoute", "音频路由", diagnostics.audioRoute ?? "--"),
                row(
                    "handshake", "通道握手",
                    diagnostics.peer?.dataChannels.map { $0.handshakeAcknowledged ? "完成" : "等待" }
                        ?? "--"),
            ]
        }
        let required = Set([
            "rtt", "jitter", "packetLoss", "qualityObserved", "totalBitrate", "controlChannelState",
            "receivedFps", "decodedFps", "rendererSubmitFps", "videoAdvancementFps", "sourceSize",
            "outputSize", "renderPipelineType", "srRuntime", "presentationMilestone",
            "recoveryCause",
            "recoveryEffect", "keyframeSuccessRate", "connectedElapsed", "mediaReadyElapsed",
            "frameWindow", "observation", "sampleAge", "frameSupplyDelta", "steadySupply",
        ])
        return result.filter {
            required.contains($0.id)
                || !$0.value.replacingOccurrences(of: " / ", with: "").replacingOccurrences(
                    of: "--", with: ""
                ).isEmpty
        }
    }

    var clipboardSummary: String {
        let excluded = Set(["transportPath", "observation", "audioRoute"])
        let groups = Dictionary(
            uniqueKeysWithValues: StreamingDiagnosticGroup.allCases.map { group in
                (
                    group.rawValue,
                    Dictionary(
                        uniqueKeysWithValues: rows(group).filter { !excluded.contains($0.id) }.map {
                            ($0.id, $0.value)
                        })
                )
            })
        guard
            let data = try? JSONSerialization.data(
                withJSONObject: groups, options: [.prettyPrinted, .sortedKeys]),
            let text = String(data: data, encoding: .utf8)
        else { return "{}" }
        return text
    }

    private func decimal(_ value: Double?, digits: Int = 1) -> String {
        guard let value, value.isFinite, value >= 0 else { return "--" }
        return String(format: "%.*f", digits, value)
    }
    private func duration(_ value: Double?) -> String {
        let formatted = decimal(value)
        return formatted == "--" ? formatted : "\(formatted) ms"
    }
    private func bitrate(_ value: Double?) -> String {
        let formatted = decimal(value.map { $0 / 1_000_000 })
        return formatted == "--" ? formatted : "\(formatted) Mbps"
    }
    private func percent(_ value: Double?) -> String {
        let formatted = decimal(value)
        return formatted == "--" ? formatted : "\(formatted)%"
    }
    private func dimensions(_ width: Int?, _ height: Int?) -> String {
        guard let width, let height, width > 0, height > 0 else { return "--" }
        return "\(width) × \(height)"
    }
    private func integer<T: BinaryInteger>(_ value: T?) -> String {
        value.map { String(describing: $0) } ?? "--"
    }
    private func age(_ value: Double?) -> String {
        guard let value, value > 0 else { return "--" }
        return duration(max(0, nowMilliseconds - value))
    }
}
