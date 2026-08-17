import MediaPlayer
import SwiftUI
import UIKit

struct StreamingControlsOverlay: View {
    @ObservedObject var store: StreamingFeatureStore
    let revealToken: Int
    let revealSource: StreamingControlsRevealSource

    @State private var controls = StreamingControlsState()
    @State private var text = ""
    @State private var userSheetPresented = false
    @State private var textInputPresented = false
    @State private var confirmExit = false
    @State private var confirmPowerOff = false
    @State private var autoHideTask: Task<Void, Never>?

    var body: some View {
        ZStack {
            if let message = store.controlMessage {
                VStack {
                    Text(message)
                        .font(.subheadline.weight(.semibold))
                        .foregroundStyle(.white)
                        .lineLimit(1)
                        .padding(.horizontal, 14)
                        .padding(.vertical, 9)
                        .glassEffect(.regular.tint(.black.opacity(0.22)), in: Capsule())
                        .transition(.opacity)
                    Spacer()
                }
                .padding(.top, 18)
                .padding(.horizontal, 20)
            }
            if controls.dockVisible {
                VStack {
                    Spacer()
                    dock
                }
                .padding(.horizontal, 20)
                .padding(.bottom, 18)
                .transition(.opacity.combined(with: .move(edge: .bottom)))
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .accessibilityAction(named: "打开串流控制") {
            reveal(source: .accessibility)
        }
        .onChange(of: revealToken) { _, _ in
            if revealSource == .singleTap {
                controls.toggleDock()
                controls.dockVisible ? scheduleAutoHide() : cancelAutoHide()
            } else {
                reveal(source: revealSource)
            }
        }
        .onChange(of: store.controlMessage) { _, message in
            guard let message else { return }
            scheduleAutoHide()
            Task {
                try? await Task.sleep(for: .seconds(2))
                await MainActor.run { store.clearControlMessage(ifMatching: message) }
            }
        }
        .sheet(isPresented: $userSheetPresented, onDismiss: finishUserSheetDismissal) {
            StreamingUserControlsSheet(
                store: store,
                onExit: {
                    controls.present(.confirmingExit)
                    userSheetPresented = false
                },
                onPowerOff: {
                    controls.present(.confirmingPowerOff)
                    userSheetPresented = false
                }
            )
        }
        .alert("结束串流？", isPresented: $confirmExit) {
            Button("取消", role: .cancel) { resumeDock() }
            Button("结束串流", role: .destructive) { store.stop() }
        }
        .alert("关闭主机并结束串流？", isPresented: $confirmPowerOff) {
            Button("取消", role: .cancel) { resumeDock() }
            Button("关闭主机", role: .destructive) { store.powerOffAndStop() }
        }
        .alert("发送文字", isPresented: $textInputPresented) {
            TextField("输入文字", text: $text)
            Button("取消", role: .cancel) { text = ""; resumeDock() }
            Button("发送") {
                store.sendText(text)
                text = ""
                resumeDock()
            }
        }
        .onDisappear { cancelAutoHide() }
    }

    private var dock: some View {
        GlassEffectContainer(spacing: 10) {
            HStack(spacing: 10) {
                dockButton(
                    systemName: store.audioMuted ? "speaker.slash.fill" : "speaker.wave.2.fill",
                    label: store.audioMuted ? "恢复声音" : "静音"
                ) {
                    store.setAudioMuted(!store.audioMuted)
                    scheduleAutoHide()
                }
                .disabled(!store.controlCapabilities.canMuteAudio)

                if store.controlCapabilities.canSendText {
                    dockButton(systemName: "text.cursor", label: "发送文字") {
                        cancelAutoHide()
                        controls.present(.textInput)
                        textInputPresented = true
                    }
                }

                dockButton(
                    systemName: store.videoContentMode == .fit
                        ? "arrow.down.right.and.arrow.up.left"
                        : "arrow.up.left.and.arrow.down.right",
                    label: "画面\(store.videoContentMode.title)"
                ) {
                    store.toggleVideoContentMode()
                    scheduleAutoHide()
                }

                dockButton(systemName: "ellipsis", label: "更多") {
                    cancelAutoHide()
                    controls.present(.userSheet)
                    userSheetPresented = true
                }
            }
            .padding(8)
            .glassEffect(.regular.tint(.black.opacity(0.18)), in: Capsule())
        }
    }

    private func dockButton(
        systemName: String,
        label: String,
        action: @escaping () -> Void
    ) -> some View {
        Button(action: action) {
            Image(systemName: systemName)
                .font(.system(size: 19, weight: .semibold))
                .foregroundStyle(.white)
                .frame(width: 50, height: 50)
                .contentShape(Circle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(label)
    }

    private func reveal(source: StreamingControlsRevealSource) {
        controls.reveal()
        UIImpactFeedbackGenerator(style: .light).impactOccurred()
        scheduleAutoHide()
        IOSRuntimeTrace.state(
            domain: "ios-streaming",
            event: "streamControlsVisibilityChanged",
            payload: [
                "visible": .bool(true),
                "source": .string(source.rawValue),
            ],
            dimension: .input,
            importance: .debug
        )
    }

    private func resumeDock() {
        controls.reveal()
        scheduleAutoHide()
    }

    private func finishUserSheetDismissal() {
        switch controls.surface {
        case .confirmingExit:
            confirmExit = true
        case .confirmingPowerOff:
            confirmPowerOff = true
        default:
            resumeDock()
        }
    }

    private func scheduleAutoHide() {
        cancelAutoHide()
        guard controls.dockVisible else { return }
        autoHideTask = Task {
            try? await Task.sleep(for: .seconds(3))
            guard !Task.isCancelled else { return }
            await MainActor.run {
                controls.hide()
                IOSRuntimeTrace.state(
                    domain: "ios-streaming",
                    event: "streamControlsVisibilityChanged",
                    payload: ["visible": .bool(false), "source": .string("autoHide")],
                    dimension: .input,
                    importance: .debug
                )
            }
        }
    }

    private func cancelAutoHide() {
        autoHideTask?.cancel()
        autoHideTask = nil
    }
}

private struct StreamingUserControlsSheet: View {
    @ObservedObject var store: StreamingFeatureStore
    let onExit: () -> Void
    let onPowerOff: () -> Void

    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(spacing: 0) {
                    valueRow("连接", value: store.connectionSummary.title)
                    Divider().opacity(0.35)
                    VStack(alignment: .leading, spacing: 10) {
                        Text("系统音量")
                            .font(.subheadline.weight(.semibold))
                        StreamingSystemVolumeView()
                            .frame(height: 32)
                    }
                    .padding(16)
                    Divider().opacity(0.35)
                    Picker(
                        "画面",
                        selection: Binding(
                            get: { store.videoContentMode },
                            set: { store.setVideoContentMode($0) }
                        )
                    ) {
                        ForEach(StreamingVideoContentMode.allCases, id: \.self) { mode in
                            Text(mode.title).tag(mode)
                        }
                    }
                    .pickerStyle(.segmented)
                    .padding(16)
                    if IOSRuntimeTrace.currentProfile != .off {
                        Divider().opacity(0.35)
                        NavigationLink {
                            StreamingDebugView(store: store)
                        } label: {
                            HStack {
                                Label("调试信息", systemImage: "waveform.path.ecg")
                                Spacer()
                                Image(systemName: "chevron.right")
                                    .foregroundStyle(.secondary)
                            }
                            .contentShape(Rectangle())
                            .padding(16)
                        }
                        .buttonStyle(.plain)
                    }
                    Divider().opacity(0.35)
                    Button(role: .destructive, action: onExit) {
                        Label("结束串流", systemImage: "rectangle.portrait.and.arrow.right")
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .padding(16)
                    }
                    if store.controlCapabilities.canPowerOff {
                        Divider().opacity(0.35)
                        Button(role: .destructive, action: onPowerOff) {
                            Label("关闭主机并结束", systemImage: "power")
                                .frame(maxWidth: .infinity, alignment: .leading)
                                .padding(16)
                        }
                    }
                }
                .glassEffect(.regular, in: RoundedRectangle(cornerRadius: 24, style: .continuous))
                .padding(16)
            }
            .navigationTitle("串流控制")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("完成") { dismiss() }
                }
            }
        }
        .presentationDetents([.medium, .large])
        .presentationBackground(.clear)
    }

    private func valueRow(_ title: String, value: String) -> some View {
        HStack {
            Text(title).font(.subheadline.weight(.semibold))
            Spacer()
            Text(value).foregroundStyle(.secondary)
        }
        .padding(16)
    }
}

private struct StreamingDebugView: View {
    @ObservedObject var store: StreamingFeatureStore
    @State private var sharedTrace: StreamingTraceShareItem?
    @State private var exportError: String?

    var body: some View {
        List {
            Section("会话") {
                row("目标", store.streamTarget?.rawValue ?? "未采集")
                row("状态", store.state.traceCode)
                row("Attempt", store.videoTrack?.traceContext?.attemptID ?? "未采集")
                row("Generation", unsigned(store.videoTrack?.traceContext?.generation))
                row("Peer epoch", unsigned(store.videoTrack?.traceContext?.peerEpoch))
                row("采样年龄", sampleAge(store.diagnostics.updatedAtMilliseconds))
            }
            Section("网络") {
                let stats = store.diagnostics.peerStats
                row("RTT", duration(stats?.roundTripTimeSeconds.map { $0 * 1_000 }))
                row("Jitter", duration(stats?.jitterSeconds.map { $0 * 1_000 }))
                row("接收码率", bitrate(stats?.receiveBitrateBps))
                row("Packets", number(stats?.packetsReceived))
                row("丢包", signedNumber(stats?.packetsLost))
                row("Candidate", candidatePath(stats))
            }
            Section("恢复") {
                let peer = store.diagnostics.peer
                row("会话恢复", store.state == .recovering ? "进行中" : "空闲")
                row("ICE", peer?.iceConnectionState ?? "未采集")
                row("恢复链", "未采集")
            }
            Section("Media supply") {
                let stats = store.diagnostics.peerStats
                row("视频字节", number(stats?.inboundVideoBytes))
                row("音频字节", number(stats?.inboundAudioBytes))
                row("已解码", number(stats?.framesDecoded))
                row("已丢弃", number(stats?.framesDropped))
                row("帧供给 delta", signedNumber(stats?.frameSupplyDelta))
                row("NACK / PLI / FIR", feedback(stats))
            }
            Section("Steady supply") {
                let stats = store.diagnostics.peerStats
                row("状态", stats?.frameSupplyDelta.map { $0 > 0 ? "健康" : "停滞" } ?? "未采集")
                row("最近媒体年龄", sampleAge(stats?.lastMediaAtMilliseconds ?? 0))
                row("供给中断次数", "未采集")
            }
            Section("上屏") {
                let display = store.diagnostics.displayHealth
                row("Submit / Present", pair(display?.submitCount, display?.presentCount))
                row("Submit-Present P50", duration(display?.submitToPresentP50Milliseconds))
                row("Submit-Present P95", duration(display?.submitToPresentP95Milliseconds))
                row("Submit-Present P99", "未采集")
                row("Present delta", signedNumber(display?.displayedFrameDelta))
                row("Mailbox overwrite", signedNumber(display?.mailboxOverwriteCount))
                row("Drawable busy", display.map { $0.currentDrawableBusy ? "是" : "否" } ?? "未采集")
            }
            Section("音频") {
                row("Lifecycle", store.diagnostics.audioLifecycleState ?? "未采集")
                row("Route", store.diagnostics.audioRoute ?? "未采集")
                row("远端音轨", audioTrackState)
            }
            Section("控制通道") {
                let peer = store.diagnostics.peer
                row("Signaling", peer?.signalingState ?? "未采集")
                row("ICE", peer?.iceConnectionState ?? "未采集")
                row("Control ready", peer?.dataChannels.map { $0.controlReady ? "是" : "否" } ?? "未采集")
                row("Handshake", peer?.dataChannels.map { $0.handshakeAcknowledged ? "完成" : "等待" } ?? "未采集")
            }
            Section {
                Button {
                    UIPasteboard.general.string = diagnosticSummary
                } label: {
                    Label("复制诊断摘要", systemImage: "doc.on.doc")
                }
                Button {
                    Task {
                        do {
                            let url = try await IOSRuntimeTrace.prepareExport(allFiles: false)
                            sharedTrace = StreamingTraceShareItem(url: url)
                        } catch {
                            exportError = CloudLibraryDiagnostics.safeError(error)
                        }
                    }
                } label: {
                    Label("导出 Runtime Trace", systemImage: "square.and.arrow.up")
                }
            }
        }
        .navigationTitle("调试信息")
        .navigationBarTitleDisplayMode(.inline)
        .sheet(item: $sharedTrace) { item in
            StreamingActivityView(url: item.url)
        }
        .alert("Trace 导出失败", isPresented: exportErrorPresented) {
            Button("知道了", role: .cancel) { exportError = nil }
        } message: {
            Text(exportError ?? "当前没有可导出的 Trace")
        }
    }

    private func row(_ title: String, _ value: String) -> some View {
        HStack(alignment: .firstTextBaseline) {
            Text(title)
            Spacer(minLength: 16)
            Text(value)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.trailing)
                .textSelection(.enabled)
        }
    }

    private func duration(_ value: Double?) -> String {
        value.map { String(format: "%.1f ms", $0) } ?? "未采集"
    }

    private func milliseconds(_ value: Double) -> String {
        value > 0 ? String(format: "%.0f ms", value) : "未采集"
    }

    private func sampleAge(_ timestamp: Double) -> String {
        guard timestamp > 0 else { return "未采集" }
        let age = max(0, ProcessInfo.processInfo.systemUptime * 1_000 - timestamp)
        return milliseconds(age)
    }

    private func bitrate(_ value: Double?) -> String {
        value.map { String(format: "%.2f Mbps", $0 / 1_000_000) } ?? "未采集"
    }

    private func number<T: BinaryInteger>(_ value: T?) -> String {
        guard let value else { return "未采集" }
        return String(describing: value)
    }

    private func signedNumber<T: BinaryInteger>(_ value: T?) -> String {
        guard let value else { return "未采集" }
        return String(describing: value)
    }

    private func unsigned(_ value: UInt64?) -> String {
        value.map { String($0) } ?? "未采集"
    }

    private func pair<T: BinaryInteger>(_ first: T?, _ second: T?) -> String {
        guard let first, let second else { return "未采集" }
        return "\(first) / \(second)"
    }

    private func candidatePath(_ stats: StreamingPeerStats?) -> String {
        guard let stats else { return "未采集" }
        let local = stats.selectedLocalCandidateType ?? "?"
        let remote = stats.selectedRemoteCandidateType ?? "?"
        let transport = stats.selectedCandidatePairProtocol ?? "?"
        return "\(local) → \(remote) · \(transport)"
    }

    private func feedback(_ stats: StreamingPeerStats?) -> String {
        guard let stats else { return "未采集" }
        return "\(stats.nackCount ?? 0) / \(stats.pliCount ?? 0) / \(stats.firCount ?? 0)"
    }

    private var audioTrackState: String {
        guard store.diagnostics.audioTrackReady else { return "未就绪" }
        return store.audioMuted ? "静音" : "播放"
    }

    private var diagnosticSummary: String {
        let stats = store.diagnostics.peerStats
        let display = store.diagnostics.displayHealth
        return [
            "target=\(store.streamTarget?.rawValue ?? "unknown")",
            "state=\(store.state.traceCode)",
            "ice=\(store.diagnostics.peer?.iceConnectionState ?? "unknown")",
            "rtt=\(duration(stats?.roundTripTimeSeconds.map { $0 * 1_000 }))",
            "jitter=\(duration(stats?.jitterSeconds.map { $0 * 1_000 }))",
            "frameSupplyDelta=\(signedNumber(stats?.frameSupplyDelta))",
            "submitPresentP95=\(duration(display?.submitToPresentP95Milliseconds))",
            "audio=\(audioTrackState)",
        ].joined(separator: "\n")
    }

    private var exportErrorPresented: Binding<Bool> {
        Binding(
            get: { exportError != nil },
            set: { if !$0 { exportError = nil } }
        )
    }
}

private struct StreamingSystemVolumeView: UIViewRepresentable {
    func makeUIView(context: Context) -> MPVolumeView {
        MPVolumeView(frame: .zero)
    }

    func updateUIView(_ uiView: MPVolumeView, context: Context) {}
}

private struct StreamingTraceShareItem: Identifiable {
    let id = UUID()
    let url: URL
}

private struct StreamingActivityView: UIViewControllerRepresentable {
    let url: URL

    func makeUIViewController(context: Context) -> UIActivityViewController {
        UIActivityViewController(activityItems: [url], applicationActivities: nil)
    }

    func updateUIViewController(
        _ uiViewController: UIActivityViewController,
        context: Context
    ) {}
}
